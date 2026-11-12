//! Proton 性能记录、统计与本地报告.
mod capture;
mod migrate;
mod report;
mod statistics;

pub use capture::Recording;
pub use statistics::{analyze, Analysis, Metrics, Point};

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::environment::RuntimeEnvironment;
use crate::identity::GameId;

/// 每个游戏保留的未固定记录条数.
const RETENTION: usize = 10;

/// 采样进行中的标记文件, 采样前的清理靠它跳过可能仍在写入的目录.
const RECORDING_MARKER: &str = ".recording";

/// 采样点与帧时间分布存放在记录自己的目录里, 索引只保留可枚举的摘要.
const SAMPLES_FILE: &str = "samples.json";

/// 搬进 records/ 后清掉会话目录里的历史文件, 采样数据与记录元数据已覆盖它们.
const LEGACY_SESSION_FILE: &str = "session.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuIdentity {
    pub uuid: String,
    pub name: String,
    pub driver: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Facts {
    pub game_executable: String,
    pub game_version: Option<String>,
    pub launch_mode: String,
    pub display_backend: String,
    pub game_mode: crate::game_mode::Options,
    pub vkd3d_config: Option<String>,
    pub proton_identity: String,
    pub kernel: Option<String>,
    pub cpu: Option<String>,
    pub gpu: Option<GpuIdentity>,
}

/// 采样时的运行环境快照; 环境之后被改名或删除都不影响历史记录.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentSnapshot {
    pub id: String,
    pub name: String,
    pub proton: String,
}

impl EnvironmentSnapshot {
    pub fn capture(environment: &RuntimeEnvironment) -> Self {
        let proton = environment
            .proton
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| environment.proton.identity.clone());
        Self {
            id: environment.id.clone(),
            name: environment.name.clone(),
            proton,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub id: String,
    pub game: GameId,
    pub started_at: u64,
    pub duration_seconds: f64,
    pub frames: usize,
    pub metrics: Metrics,
    pub facts: Facts,
    pub environment: EnvironmentSnapshot,
    pub warnings: Vec<String>,
    pub pinned: bool,
}

/// 一条记录的采样数据, 只有生成报告时才读取.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Samples {
    pub histogram: Vec<f64>,
    pub points: Vec<Point>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    #[serde(default)]
    pub recording_enabled: bool,
    #[serde(default)]
    pub records: Vec<Record>,
}

/// 汇总到界面的记录, 不带采样点和直方图.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary<'a> {
    recording_enabled: bool,
    records: Vec<RecordSummary<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordSummary<'a> {
    id: &'a str,
    started_at: u64,
    duration_seconds: f64,
    frames: usize,
    metrics: &'a Metrics,
    facts: &'a Facts,
    environment: &'a EnvironmentSnapshot,
    warnings: &'a [String],
    pinned: bool,
}

pub struct Store {
    root: PathBuf,
    data: Overview,
    /// 还没搬干净的旧比较目录, 每次提交都重试一次.
    pending: migrate::Legacy,
}

impl Store {
    pub fn load(root: PathBuf) -> Result<Self> {
        let path = root.join("index.json");
        if !path.try_exists()? {
            // 索引缺失时分不清孤儿目录和原始日志, 不做任何清理.
            return Ok(Self {
                root,
                data: Overview::default(),
                pending: migrate::Legacy::default(),
            });
        }
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path)?).context("性能记录无效")?;
        let old_index = value.get("comparisons").is_some();
        let (data, legacy) = if old_index {
            migrate::overview(value)?
        } else {
            (
                serde_json::from_value(value).context("性能记录无效")?,
                migrate::Legacy::default(),
            )
        };
        let mut store = Self {
            root,
            data,
            pending: migrate::Legacy::default(),
        };
        // 索引只在迁移全部完成时才改写, 重试期间采样数据一直留在旧索引里.
        if old_index {
            if store.adopt_legacy(&legacy) {
                atomic_write(&path, &serde_json::to_vec(&store.data)?)?;
            } else {
                store.pending = legacy;
            }
        }
        // 还有旧比较目录没搬完时索引不是全部真相, 等迁移完成后的那次加载再清理.
        if store.pending.is_empty() {
            store.sweep(true);
        }
        Ok(store)
    }

    #[cfg(test)]
    fn data(&self) -> &Overview {
        &self.data
    }

    pub fn recording_enabled(&self) -> bool {
        self.data.recording_enabled
    }

    pub fn set_recording(&mut self, enabled: bool) -> Result<()> {
        let mut next = self.data.clone();
        next.recording_enabled = enabled;
        self.commit(next)
    }

    pub fn summary(&self, game: &GameId) -> Summary<'_> {
        Summary {
            recording_enabled: self.data.recording_enabled,
            records: self
                .data
                .records
                .iter()
                .filter(|record| &record.game == game)
                .map(|record| RecordSummary {
                    id: &record.id,
                    started_at: record.started_at,
                    duration_seconds: record.duration_seconds,
                    frames: record.frames,
                    metrics: &record.metrics,
                    facts: &record.facts,
                    environment: &record.environment,
                    warnings: &record.warnings,
                    pinned: record.pinned,
                })
                .collect(),
        }
    }

    pub fn record(&self, id: &str) -> Result<&Record> {
        self.data
            .records
            .iter()
            .find(|record| record.id == id)
            .context("性能记录不存在")
    }

    /// 记录的采样数据; 缺少文件时按没有采样点处理, 报告仍然可以生成.
    pub fn samples(&self, id: &str) -> Result<Samples> {
        // id 来自索引, 拼接路径前先确认它就是一个记录目录名.
        if !is_record_id(id) {
            return Ok(Samples::default());
        }
        match fs::read(self.records_dir().join(id).join(SAMPLES_FILE)) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("性能记录采样数据无效"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Samples::default()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn prepare(
        &self,
        game: &GameId,
        environment: &RuntimeEnvironment,
        facts: Facts,
        command: &mut std::process::Command,
    ) -> Result<Option<Recording>> {
        if !self.data.recording_enabled {
            return Ok(None);
        }
        self.sweep(false);
        Recording::prepare(&self.root, game, environment, facts, command).map(Some)
    }

    pub fn finish(&mut self, recording: &Recording) -> Result<()> {
        // 采样结束, 目录不再需要「进行中」标记保护.
        let _ = fs::remove_file(recording.directory().join(RECORDING_MARKER));
        let Some(record) = recording.finish() else {
            remove_record_dir(recording.directory());
            return Ok(());
        };
        let mut next = self.data.clone();
        next.records.insert(0, record);
        let dropped = prune(&mut next.records);
        self.commit(next)?;
        for id in dropped {
            // 形状不合适的 id 只从索引里移除, 不拼接成路径.
            if is_record_id(&id) {
                remove_record_dir(&self.records_dir().join(&id));
            }
        }
        Ok(())
    }

    pub fn pin(&mut self, id: &str, pinned: bool) -> Result<()> {
        let mut next = self.data.clone();
        next.records
            .iter_mut()
            .find(|record| record.id == id)
            .context("性能记录不存在")?
            .pinned = pinned;
        self.commit(next)
    }

    pub fn delete(&mut self, id: &str) -> Result<()> {
        let mut next = self.data.clone();
        let index = next
            .records
            .iter()
            .position(|record| record.id == id)
            .context("性能记录不存在")?;
        next.records.remove(index);
        self.commit(next)?;
        // 形状不合适的 id 只从索引里移除, 不拼接成路径.
        if is_record_id(id) {
            remove_record_dir(&self.records_dir().join(id));
        }
        Ok(())
    }

    pub fn svg(&self, a: &str, b: &str) -> Result<String> {
        let (a, b) = self.pair(a, b)?;
        let (a_samples, b_samples) = (self.samples(&a.id)?, self.samples(&b.id)?);
        Ok(report::svg(a, &a_samples, b, &b_samples))
    }

    pub fn export(&self, a: &str, b: &str, destination: &Path) -> Result<PathBuf> {
        if !destination.is_absolute() || destination.is_dir() {
            bail!("请选择 PNG 文件的绝对保存路径");
        }
        let svg = self.svg(a, b)?;
        let destination = destination.with_extension("png");
        atomic_write(&destination, &report::png(&svg)?)?;
        Ok(destination)
    }

    fn pair(&self, a: &str, b: &str) -> Result<(&Record, &Record)> {
        if a == b {
            bail!("请选择两条不同的性能记录");
        }
        Ok((self.record(a)?, self.record(b)?))
    }

    fn records_dir(&self) -> PathBuf {
        self.root.join("records")
    }

    /// 把旧比较目录里的会话目录搬进 `records/`, 搬完再补写旧索引里的采样数据;
    /// 两步都成功才删除旧目录. 返回是否全部处理完成, 未完成时调用方不改写索引,
    /// 下次加载继续迁移同一个目录.
    fn adopt_legacy(&self, legacy: &migrate::Legacy) -> bool {
        let records = self.records_dir();
        let mut complete = true;
        for comparison in &legacy.comparisons {
            // 只处理迁移读到的旧比较目录, 不按目录名猜测; 不是合法旧比较目录名的
            // id 不拼接路径, 也不做任何文件操作.
            if !is_legacy_id(&comparison.id) {
                continue;
            }
            let directory = self.root.join(&comparison.id);
            if !directory.is_dir() {
                continue;
            }
            // 读不出来的目录既搬不走也删不得, 原样留给下次加载.
            let Ok(sessions) = fs::read_dir(&directory) else {
                complete = false;
                continue;
            };
            let mut moved = true;
            for session in sessions.flatten() {
                let name = session.file_name().to_string_lossy().into_owned();
                if !session.path().is_dir() || !self.data.records.iter().any(|r| r.id == name) {
                    continue;
                }
                let target = records.join(&name);
                if target.exists() {
                    // 会话目录里的历史文件由采样数据和记录元数据承担.
                    let _ = fs::remove_file(target.join(LEGACY_SESSION_FILE));
                    continue;
                }
                if fs::create_dir_all(&records).is_err()
                    || fs::rename(session.path(), &target).is_err()
                {
                    moved = false;
                    continue;
                }
                let _ = fs::remove_file(target.join(LEGACY_SESSION_FILE));
            }
            // 目录搬完之后才写采样数据: 写不进去时旧目录保留, 下次加载重新迁移.
            if !moved || !self.restore_samples(&comparison.sessions) {
                complete = false;
                continue;
            }
            // 采样失败的会话目录没有对应记录, 随旧目录一起丢弃.
            let _ = fs::remove_dir_all(&directory);
        }
        complete
    }

    /// 目录搬进 `records/` 之后补写旧索引里的采样数据, 已有的采样文件不重复写.
    /// 返回是否全部写入; 写不进去就当作迁移未完成.
    fn restore_samples(&self, sessions: &[migrate::LegacySession]) -> bool {
        let mut complete = true;
        for session in sessions {
            // 只处理有对应记录的会话; 其余旧会话目录随旧比较目录一起丢弃.
            if !is_record_id(&session.id)
                || !self
                    .data
                    .records
                    .iter()
                    .any(|record| record.id == session.id)
            {
                continue;
            }
            let directory = self.records_dir().join(&session.id);
            if directory.join(SAMPLES_FILE).is_file() {
                continue;
            }
            // 目录还没搬进来时无从写入, 留给下一次重试.
            if !directory.is_dir() {
                complete = false;
                continue;
            }
            if serde_json::to_vec(&session.samples)
                .map_err(anyhow::Error::from)
                .and_then(|bytes| atomic_write(&directory.join(SAMPLES_FILE), &bytes))
                .is_err()
            {
                complete = false;
            }
        }
        complete
    }

    /// 清理索引里没有记录的目录.
    ///
    /// `startup` 为真表示正在加载: 此时索引就是唯一真相, 索引没有的目录一律删除,
    /// 不管它有没有采样日志或进行中的标记. 为假表示采样前的清理, 带 `.recording`
    /// 或已有采样日志的目录可能在采样中, 一律跳过.
    fn sweep(&self, startup: bool) {
        let Ok(entries) = fs::read_dir(self.records_dir()) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            // 目录名就是记录 id, 不像记录目录的一律不动.
            if !is_record_id(&name) {
                continue;
            }
            if self.data.records.iter().any(|record| record.id == name) {
                continue;
            }
            let directory = entry.path();
            if !startup && (directory.join(RECORDING_MARKER).exists() || holds_samples(&directory))
            {
                continue;
            }
            remove_record_dir(&directory);
        }
    }

    fn commit(&mut self, next: Overview) -> Result<()> {
        // 旧比较目录没搬完时旧索引是采样数据唯一的所在, 不能改写也不能删文件.
        if !self.pending.is_empty() {
            if !self.adopt_legacy(&self.pending) {
                bail!("旧性能记录尚未迁移完成, 请检查数据目录权限后重试");
            }
            self.pending = migrate::Legacy::default();
        }
        fs::create_dir_all(&self.root)?;
        atomic_write(&self.root.join("index.json"), &serde_json::to_vec(&next)?)?;
        self.data = next;
        Ok(())
    }
}

/// 记录 id 由 `Recording` 以十六进制生成; 只有形如它的单个路径组件才允许拼接路径.
fn is_record_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// 旧比较目录的 id 是纳秒的十六进制表示; 拼接路径或删除目录前必须确认它的形状.
fn is_legacy_id(id: &str) -> bool {
    (8..=32).contains(&id.len())
        && id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// 删除记录目录; 删不掉不影响索引, 但要留下是哪一层目录没能回收.
fn remove_record_dir(directory: &Path) {
    match fs::remove_dir_all(directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => eprintln!("删除性能记录目录失败: {} ({error})", directory.display()),
    }
}

/// 目录里已有采样日志 (或读不出来), 不能当成残留清理.
fn holds_samples(directory: &Path) -> bool {
    let Ok(entries) = fs::read_dir(directory) else {
        return true;
    };
    entries.flatten().any(|entry| {
        entry
            .file_name()
            .to_string_lossy()
            .to_ascii_lowercase()
            .ends_with(".csv")
    })
}

/// 每个游戏只保留最近的未固定记录, 返回被淘汰的记录 id.
fn prune(records: &mut Vec<Record>) -> Vec<String> {
    let mut kept: BTreeMap<GameId, usize> = BTreeMap::new();
    let mut dropped = Vec::new();
    records.retain(|record| {
        if record.pinned {
            return true;
        }
        let count = kept.entry(record.game.clone()).or_default();
        *count += 1;
        if *count <= RETENTION {
            return true;
        }
        dropped.push(record.id.clone());
        false
    });
    dropped
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(path.parent().context("保存目录不存在")?)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::ChannelId;
    use crate::environment::ProtonPin;
    use std::process::Command;

    fn environment(id: &str) -> RuntimeEnvironment {
        RuntimeEnvironment {
            id: id.into(),
            name: format!("Proton {id}"),
            proton: ProtonPin {
                path: format!("/proton/GE-Proton-{id}").into(),
                identity: id.into(),
            },
            install_root: "/game".into(),
            game: GameId::from("nte"),
            distribution: ChannelId::new("cn"),
            created_at: 1,
        }
    }

    fn facts() -> Facts {
        Facts {
            game_executable: crate::testing::GAME_EXE_NAME.into(),
            display_backend: "XWayland/X11".into(),
            ..Facts::default()
        }
    }

    fn legacy_session(id: &str, side: &str, fps: f64, started_at: u64) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "side": side,
            "startedAt": started_at,
            "durationSeconds": 60.0,
            "frames": 600,
            "metrics": {
                "fps": fps,
                "low1": fps,
                "p95": 16.0,
                "p99": 20.0,
                "stallPercent": 0.0,
                "vramMeanMib": null,
                "rssMeanMib": null,
                "gpuLoadMean": null,
                "powerMeanW": null
            },
            "histogram": vec![100.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            "points": [{"seconds": 1.0, "fps": fps}],
            "facts": facts(),
            "warnings": [],
        })
    }

    fn store(dir: &tempfile::TempDir) -> Store {
        Store::load(dir.path().into()).unwrap()
    }

    fn write_log(recording: &Recording, frametime: f64) {
        let mut csv = String::from("os,cpu,gpu\nLinux,CPU,GPU\nfps,frametime,elapsed\n");
        for frame in 1..=1500 {
            csv.push_str(&format!(
                "60,{frametime},{}\n",
                (frame as f64 * frametime * 1e6) as u64
            ));
        }
        fs::write(
            recording.directory().join(format!(
                "{}_2026-11-08_01-00-00.csv",
                crate::testing::GAME_EXE_NAME
            )),
            csv,
        )
        .unwrap();
        let mut hardware = String::new();
        for second in 0..=(frametime * 1.5) as u64 {
            let point = Point {
                seconds: second as f64,
                gpu_load: Some(70.0 + (second as f64 * 0.6).sin() * 10.0),
                vram_mib: Some(4000.0 + frametime * 80.0 + second as f64 * 8.0),
                power_w: Some(80.0 + (second as f64 * 0.4).sin() * 6.0),
                ..Point::default()
            };
            hardware.push_str(&serde_json::to_string(&point).unwrap());
            hardware.push('\n');
        }
        fs::write(recording.directory().join("hardware.jsonl"), hardware).unwrap();
    }

    fn capture(store: &Store, id: &str, frametime: f64) -> Recording {
        let recording = store
            .prepare(
                &GameId::from("nte"),
                &environment(id),
                facts(),
                &mut Command::new("game"),
            )
            .unwrap()
            .unwrap();
        write_log(&recording, frametime);
        recording
    }

    #[test]
    fn records_a_launch_only_while_the_switch_is_on_and_keeps_the_environment_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        assert!(!store.recording_enabled());
        let mut command = Command::new("game");
        assert!(store
            .prepare(
                &GameId::from("nte"),
                &environment("a"),
                facts(),
                &mut command
            )
            .unwrap()
            .is_none());
        assert_eq!(command.get_envs().count(), 0);

        store.set_recording(true).unwrap();
        let recording = capture(&store, "a", 10.0);
        store.finish(&recording).unwrap();
        let records = &store.data().records;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].metrics.fps, 100.0);
        assert_eq!(records[0].environment.name, "Proton a");
        assert_eq!(records[0].environment.proton, "GE-Proton-a");
        assert_eq!(records[0].game, GameId::from("nte"));
        assert!(!records[0].pinned);
        let samples = store.samples(&records[0].id).unwrap();
        assert_eq!(samples.histogram.len(), 11);
        assert!(samples.points.iter().any(|p| p.gpu_load.is_some()));
        let summary = serde_json::to_value(store.summary(&GameId::from("nte"))).unwrap();
        assert_eq!(summary["recordingEnabled"], true);
        assert!(summary["records"][0].get("points").is_none());
        assert!(summary["records"][0].get("histogram").is_none());
        assert_eq!(
            summary["records"][0]["environment"]["proton"],
            "GE-Proton-a"
        );
        assert_eq!(
            serde_json::to_value(store.summary(&GameId::from("hsr"))).unwrap()["records"],
            serde_json::json!([])
        );

        // 记录目录只保留索引引用的条目.
        let reloaded = Store::load(dir.path().into()).unwrap();
        assert!(reloaded.recording_enabled());
        assert_eq!(reloaded.data().records.len(), 1);
        assert!(dir
            .path()
            .join("records")
            .join(&reloaded.data().records[0].id)
            .join("metadata.json")
            .exists());
    }

    #[test]
    fn keeps_sample_points_and_the_histogram_out_of_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        store.set_recording(true).unwrap();
        let recording = capture(&store, "a", 10.0);
        let id = recording.id().to_string();
        store.finish(&recording).unwrap();

        let samples = store.samples(&id).unwrap();
        assert_eq!(samples.histogram.len(), 11);
        assert!((samples.histogram.iter().sum::<f64>() - 100.0).abs() < 1e-6);
        assert!(samples.points.iter().any(|p| p.fps.is_some()));
        assert!(samples.points.iter().any(|p| p.gpu_load.is_some()));
        let persisted: serde_json::Value = serde_json::from_slice(
            &fs::read(dir.path().join("records").join(&id).join("samples.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(persisted["histogram"].as_array().unwrap().len(), 11);
        assert!(!persisted["points"].as_array().unwrap().is_empty());

        // 索引只留摘要, 采样点随游玩时长增长也不影响它.
        let index: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.path().join("index.json")).unwrap()).unwrap();
        assert!(index["records"][0].get("points").is_none());
        assert!(index["records"][0].get("histogram").is_none());

        // 采样数据缺失的记录按没有采样点处理, 不让读取失败.
        fs::remove_file(dir.path().join("records").join(&id).join("samples.json")).unwrap();
        let missing = store.samples(&id).unwrap();
        assert!(missing.points.is_empty());
        assert!(missing.histogram.is_empty());
    }

    #[test]
    fn mutes_the_mangohud_environment_into_the_launch_command() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        store.set_recording(true).unwrap();
        let mut command = Command::new("game");
        let recording = store
            .prepare(
                &GameId::from("nte"),
                &environment("a"),
                facts(),
                &mut command,
            )
            .unwrap()
            .unwrap();
        let variables: BTreeMap<_, _> = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.unwrap_or_default().to_string_lossy().into_owned(),
                )
            })
            .collect();
        assert_eq!(variables["MANGOHUD"], "1");
        let config = &variables["MANGOHUD_CONFIG"];
        assert!(config.contains("log_interval=0"));
        assert!(config.contains("upload_logs=0"));
        assert!(config.contains(&recording.directory().to_string_lossy().into_owned()));
    }

    #[test]
    fn discards_runs_without_a_usable_log_and_removes_their_directory() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        store.set_recording(true).unwrap();
        let missing = store
            .prepare(
                &GameId::from("nte"),
                &environment("a"),
                facts(),
                &mut Command::new("game"),
            )
            .unwrap()
            .unwrap();
        let missing_directory = missing.directory().to_path_buf();
        store.finish(&missing).unwrap();
        assert!(store.data().records.is_empty());
        assert!(!missing_directory.exists());

        let broken = store
            .prepare(
                &GameId::from("nte"),
                &environment("a"),
                facts(),
                &mut Command::new("game"),
            )
            .unwrap()
            .unwrap();
        fs::write(
            broken.directory().join(format!(
                "{}_2026-11-08_01-00-00.csv",
                crate::testing::GAME_EXE_NAME
            )),
            "fps,frametime,elapsed\n60,NaN,6000000000\n",
        )
        .unwrap();
        let broken_directory = broken.directory().to_path_buf();
        store.finish(&broken).unwrap();
        assert!(store.data().records.is_empty());
        assert!(!broken_directory.exists());
    }

    #[test]
    fn keeps_the_latest_ten_unpinned_records_per_game() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        store.set_recording(true).unwrap();
        let mut oldest = String::new();
        for round in 0..12 {
            let recording = capture(&store, "a", 10.0 + round as f64);
            if round == 0 {
                oldest = recording.id().to_string();
            }
            store.finish(&recording).unwrap();
        }
        assert_eq!(store.data().records.len(), RETENTION);
        assert!(store
            .data()
            .records
            .iter()
            .all(|record| record.id != oldest));
        assert!(!dir.path().join("records").join(&oldest).exists());
        assert_eq!(store.data().records[0].metrics.fps, 1000.0 / 21.0);

        assert!(store.pin(&oldest, true).is_err());
        let latest = store.data().records[0].id.clone();
        store.pin(&latest, true).unwrap();
        assert!(store.record(&latest).unwrap().pinned);
        store.pin(&latest, false).unwrap();
        assert!(!store.record(&latest).unwrap().pinned);
    }

    #[test]
    fn pinned_records_survive_the_retention_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        store.set_recording(true).unwrap();
        let first = capture(&store, "a", 10.0);
        let first_id = first.id().to_string();
        store.finish(&first).unwrap();
        store.pin(&first_id, true).unwrap();
        for _ in 0..RETENTION + 1 {
            let recording = capture(&store, "a", 10.0);
            store.finish(&recording).unwrap();
        }
        let records = &store.data().records;
        assert_eq!(records.len(), RETENTION + 1);
        assert!(store.record(&first_id).unwrap().pinned);
        assert!(dir.path().join("records").join(&first_id).exists());
    }

    #[test]
    fn unpinning_keeps_the_record_until_the_next_recording() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        store.set_recording(true).unwrap();
        let first = capture(&store, "a", 10.0);
        let first_id = first.id().to_string();
        store.finish(&first).unwrap();
        store.pin(&first_id, true).unwrap();
        for _ in 0..RETENTION {
            let recording = capture(&store, "a", 10.0);
            store.finish(&recording).unwrap();
        }
        assert_eq!(store.data().records.len(), RETENTION + 1);

        // 取消固定只写索引, 超出上限的记录留到下一次采样.
        store.pin(&first_id, false).unwrap();
        assert_eq!(store.data().records.len(), RETENTION + 1);
        assert!(store.record(&first_id).is_ok());
        assert!(dir.path().join("records").join(&first_id).exists());

        let recording = capture(&store, "a", 10.0);
        store.finish(&recording).unwrap();
        assert_eq!(store.data().records.len(), RETENTION);
        assert!(store.record(&first_id).is_err());
        assert!(!dir.path().join("records").join(&first_id).exists());
    }

    #[test]
    fn adopts_leftover_session_directories_on_the_next_load() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = serde_json::json!({
            "active": "0badc0de",
            "comparisons": [{
                "id": "0badc0de",
                "name": "A/B",
                "createdAt": 1,
                "a": environment("a"),
                "b": environment("b"),
                "sessions": [
                    legacy_session("1a1a", "A", 100.0, 1_700_000_000_000),
                    legacy_session("2b2b", "B", 80.0, 1_700_000_100_000),
                ],
            }],
        });
        let directory = dir.path().join("0badc0de");
        for id in ["1a1a", "2b2b"] {
            let session = directory.join(id);
            fs::create_dir_all(&session).unwrap();
            fs::write(session.join("hardware.jsonl"), "{}").unwrap();
        }
        // 采样失败的会话没有对应记录, 只剩一个空目录.
        fs::create_dir_all(directory.join("3c3c")).unwrap();
        fs::write(directory.join("report.svg"), "<svg/>").unwrap();
        fs::write(dir.path().join("index.json"), legacy.to_string()).unwrap();

        let store = Store::load(dir.path().into()).unwrap();
        assert_eq!(store.data().records.len(), 2);
        for id in ["1a1a", "2b2b"] {
            assert!(dir
                .path()
                .join("records")
                .join(id)
                .join("hardware.jsonl")
                .exists());
            assert!(dir
                .path()
                .join("records")
                .join(id)
                .join("samples.json")
                .exists());
        }
        // 旧索引里的采样点搬进记录目录, 报告不需要旧索引.
        let samples = store.samples("1a1a").unwrap();
        assert_eq!(samples.histogram.len(), 11);
        assert_eq!(samples.points.len(), 1);
        assert_eq!(samples.points[0].fps, Some(100.0));
        assert_eq!(samples.points[0].seconds, 1.0);
        assert!(!dir.path().join("records").join("3c3c").exists());
        assert!(!directory.exists());
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.path().join("index.json")).unwrap()).unwrap();
        assert!(persisted.get("comparisons").is_none());
        assert!(persisted["records"][0].get("points").is_none());
    }

    #[test]
    fn keeps_the_old_index_when_the_legacy_directories_cannot_be_moved() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = serde_json::json!({
            "active": null,
            "comparisons": [{
                "id": "0badc0de",
                "name": "A/B",
                "createdAt": 1,
                "a": environment("a"),
                "b": environment("b"),
                "sessions": [legacy_session("1a1a", "A", 100.0, 1_700_000_000_000)],
            }],
        });
        let session = dir.path().join("0badc0de").join("1a1a");
        fs::create_dir_all(&session).unwrap();
        fs::write(session.join("hardware.jsonl"), "{}").unwrap();
        fs::write(dir.path().join("index.json"), legacy.to_string()).unwrap();
        // 记录目录被占成普通文件, 会话目录搬不过去.
        fs::write(dir.path().join("records"), "").unwrap();

        let store = Store::load(dir.path().into()).unwrap();
        assert_eq!(store.data().records.len(), 1);
        // 搬移没完成时索引保持旧格式, 下次加载重新迁移.
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.path().join("index.json")).unwrap()).unwrap();
        assert!(persisted.get("comparisons").is_some());
        assert!(session.join("hardware.jsonl").exists());
        // 目录没搬进 records/, 采样数据留在旧索引里等下一次重试.
        assert!(!session.join(SAMPLES_FILE).exists());
        assert!(!dir.path().join("records").join("1a1a").exists());
    }

    #[test]
    fn refuses_to_commit_until_the_legacy_migration_completes() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = serde_json::json!({
            "active": null,
            "comparisons": [{
                "id": "0badc0de",
                "name": "A/B",
                "createdAt": 1,
                "a": environment("a"),
                "b": environment("b"),
                "sessions": [legacy_session("1a1a", "A", 100.0, 1_700_000_000_000)],
            }],
        });
        let comparison = dir.path().join("0badc0de");
        let session = comparison.join("1a1a");
        fs::create_dir_all(&session).unwrap();
        fs::write(session.join("hardware.jsonl"), "{}").unwrap();
        fs::write(session.join("session.json"), "{}").unwrap();
        fs::write(dir.path().join("index.json"), legacy.to_string()).unwrap();
        // 记录目录被占成普通文件, 会话目录搬不过去.
        fs::write(dir.path().join("records"), "").unwrap();

        let mut store = Store::load(dir.path().into()).unwrap();
        let id = store.data().records[0].id.clone();
        // 迁移没完成时提交直接失败, 索引保持旧格式, 旧目录和其中的会话数据原样保留.
        let error = store.pin(&id, false).unwrap_err();
        assert_eq!(
            error.to_string(),
            "旧性能记录尚未迁移完成, 请检查数据目录权限后重试"
        );
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.path().join("index.json")).unwrap()).unwrap();
        assert!(persisted.get("comparisons").is_some());
        assert_eq!(persisted["comparisons"].as_array().unwrap().len(), 1);
        assert!(session.join("hardware.jsonl").exists());
        assert!(session.join(LEGACY_SESSION_FILE).exists());
        assert!(!dir.path().join("records").join(&id).exists());
        assert!(!store.pending.is_empty());
        assert!(store.record(&id).unwrap().pinned);

        // 障碍解除后再次提交补齐迁移, 索引此时才升级.
        fs::remove_file(dir.path().join("records")).unwrap();
        store.pin(&id, false).unwrap();
        assert!(store.pending.is_empty());
        assert!(!store.record(&id).unwrap().pinned);
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.path().join("index.json")).unwrap()).unwrap();
        assert!(persisted.get("comparisons").is_none());
        assert_eq!(persisted["records"][0]["pinned"], false);
        assert!(!comparison.exists());
        let moved = dir.path().join("records").join(&id);
        assert!(moved.join("hardware.jsonl").exists());
        assert!(!moved.join(LEGACY_SESSION_FILE).exists());
        assert_eq!(store.samples(&id).unwrap().points[0].fps, Some(100.0));

        // 迁移完成后再加载不做重复搬移, 记录目录原样保留.
        let store = Store::load(dir.path().into()).unwrap();
        assert_eq!(store.data().records.len(), 1);
        assert!(moved.join("hardware.jsonl").exists());
        assert_eq!(store.samples(&id).unwrap().points[0].fps, Some(100.0));
    }

    #[test]
    fn removes_the_sampling_directory_when_the_launch_is_abandoned() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        store.set_recording(true).unwrap();
        let abandoned = store
            .prepare(
                &GameId::from("nte"),
                &environment("a"),
                facts(),
                &mut Command::new("game"),
            )
            .unwrap()
            .unwrap();
        let directory = abandoned.directory().to_path_buf();
        assert!(directory.join(RECORDING_MARKER).exists());
        drop(abandoned);
        assert!(!directory.exists());

        // 采样结果交出去之后目录归记录所有, 析构不再回收.
        let recording = capture(&store, "a", 10.0);
        let directory = recording.directory().to_path_buf();
        store.finish(&recording).unwrap();
        drop(recording);
        assert!(directory.join("metadata.json").exists());
        assert!(!directory.join(RECORDING_MARKER).exists());
    }

    #[test]
    fn ignores_legacy_ids_that_cannot_name_a_comparison_directory() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = serde_json::json!({
            "active": null,
            "comparisons": [{
                "id": "../outside",
                "name": "A/B",
                "createdAt": 1,
                "a": environment("a"),
                "b": environment("b"),
                "sessions": [legacy_session("1a1a", "A", 100.0, 1_700_000_000_000)],
            }, {
                "id": "1a1",
                "name": "A/B",
                "createdAt": 1,
                "a": environment("a"),
                "b": environment("b"),
                "sessions": [legacy_session("2b2b", "A", 90.0, 1_700_000_100_000)],
            }],
        });
        // 名字不像旧比较目录的目录不搬也不删, 里面的会话目录原样保留.
        let short = dir.path().join("1a1");
        fs::create_dir_all(short.join("2b2b")).unwrap();
        fs::write(short.join("2b2b").join("hardware.jsonl"), "{}").unwrap();
        fs::write(dir.path().join("index.json"), legacy.to_string()).unwrap();

        let store = Store::load(dir.path().into()).unwrap();
        assert_eq!(store.data().records.len(), 2);
        assert!(short.join("2b2b").join("hardware.jsonl").exists());
        assert!(!short.join("2b2b").join(SAMPLES_FILE).exists());
        assert!(!dir.path().join("records").join("2b2b").exists());
        // 跳过的 id 不影响索引升级.
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.path().join("index.json")).unwrap()).unwrap();
        assert!(persisted.get("comparisons").is_none());
        assert_eq!(persisted["records"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn does_not_recreate_legacy_session_directories_that_were_already_moved() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = serde_json::json!({
            "active": null,
            "comparisons": [{
                "id": "0badc0de",
                "name": "A/B",
                "createdAt": 1,
                "a": environment("a"),
                "b": environment("b"),
                "sessions": [legacy_session("1a1a", "A", 100.0, 1_700_000_000_000)],
            }],
        });
        // 会话目录已经搬进 records/, 旧比较目录已经删掉, 索引还是旧格式.
        let moved = dir.path().join("records").join("1a1a");
        fs::create_dir_all(&moved).unwrap();
        fs::write(moved.join("hardware.jsonl"), "{}").unwrap();
        fs::write(dir.path().join("index.json"), legacy.to_string()).unwrap();

        let store = Store::load(dir.path().into()).unwrap();
        assert_eq!(store.data().records.len(), 1);
        assert!(!dir.path().join("0badc0de").exists());
        assert!(moved.join("hardware.jsonl").exists());
        assert!(store.samples("1a1a").unwrap().points.is_empty());
    }

    #[test]
    fn keeps_the_old_directory_until_the_legacy_samples_are_written() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = serde_json::json!({
            "active": null,
            "comparisons": [{
                "id": "0badc0de",
                "name": "A/B",
                "createdAt": 1,
                "a": environment("a"),
                "b": environment("b"),
                "sessions": [legacy_session("1a1a", "A", 100.0, 1_700_000_000_000)],
            }],
        });
        let comparison = dir.path().join("0badc0de");
        let session = comparison.join("1a1a");
        fs::create_dir_all(&session).unwrap();
        fs::write(session.join("hardware.jsonl"), "{}").unwrap();
        // 采样文件在旧会话目录里被占成目录, 搬进 records/ 之后写不进去.
        fs::create_dir_all(session.join(SAMPLES_FILE)).unwrap();
        fs::write(dir.path().join("index.json"), legacy.to_string()).unwrap();
        // 索引里没有记录的目录在迁移完成前不能被当成残留清理.
        let leftover = dir.path().join("records").join("3c4d");
        fs::create_dir_all(&leftover).unwrap();

        let store = Store::load(dir.path().into()).unwrap();
        assert_eq!(store.data().records.len(), 1);
        // 会话目录已经搬进 records/, 但采样数据没写成, 迁移停在未完成状态.
        let moved = dir.path().join("records").join("1a1a");
        assert!(moved.join("hardware.jsonl").exists());
        assert!(!moved.join(SAMPLES_FILE).is_file());
        assert!(comparison.exists());
        assert!(leftover.exists());
        // 旧索引原样保留, 采样数据留待下一次重试.
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.path().join("index.json")).unwrap()).unwrap();
        assert!(persisted.get("comparisons").is_some());

        // 障碍解除后的下一次加载补写采样数据, 旧目录删除, 此时才按索引清理残留目录.
        fs::remove_dir_all(moved.join(SAMPLES_FILE)).unwrap();
        let store = Store::load(dir.path().into()).unwrap();
        assert_eq!(store.data().records.len(), 1);
        assert!(moved.join(SAMPLES_FILE).is_file());
        assert_eq!(store.samples("1a1a").unwrap().points[0].fps, Some(100.0));
        assert!(!leftover.exists());
        assert!(!comparison.exists());
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.path().join("index.json")).unwrap()).unwrap();
        assert!(persisted.get("comparisons").is_none());
    }

    #[test]
    fn keeps_record_directories_when_the_index_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        store.set_recording(true).unwrap();
        let recording = capture(&store, "a", 10.0);
        let id = recording.id().to_string();
        store.finish(&recording).unwrap();
        drop(store);

        fs::remove_file(dir.path().join("index.json")).unwrap();
        let store = Store::load(dir.path().into()).unwrap();
        assert!(store.data().records.is_empty());
        assert!(dir.path().join("records").join(&id).exists());
    }

    #[test]
    fn deletes_records_with_their_directory_and_samples() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        store.set_recording(true).unwrap();
        let recording = capture(&store, "a", 10.0);
        let id = recording.id().to_string();
        store.finish(&recording).unwrap();
        let samples = dir.path().join("records").join(&id).join("samples.json");
        assert!(samples.exists());
        store.delete(&id).unwrap();
        assert!(store.data().records.is_empty());
        assert!(!dir.path().join("records").join(&id).exists());
        assert!(!samples.exists());
        assert!(store.delete(&id).is_err());
    }

    #[test]
    fn sweeps_directories_the_index_does_not_reference() {
        let records = tempfile::tempdir().unwrap();
        let root = records.path().join("records");
        // 崩溃或退出打断采样后留下的目录, 索引里没有对应记录.
        let sampled = root.join("1a2b");
        let failed = root.join("3c4d");
        fs::create_dir_all(&sampled).unwrap();
        fs::create_dir_all(&failed).unwrap();
        let log = sampled.join(format!(
            "{}_2026-11-08_01-00-00.csv",
            crate::testing::GAME_EXE_NAME
        ));
        fs::write(&log, "fps,frametime,elapsed\n").unwrap();
        // 启动在 prepare 之后失败, 目录连同标记一起留在 records/ 里.
        fs::write(failed.join(".recording"), "").unwrap();
        fs::write(
            records.path().join("index.json"),
            serde_json::to_vec(&Overview::default()).unwrap(),
        )
        .unwrap();

        // 名字不像记录 id 的条目不属于记录目录, 不动.
        let foreign = root.join("notes");
        fs::create_dir_all(&foreign).unwrap();

        let mut store = store(&records);
        assert!(store.data().records.is_empty());
        // 加载时索引即唯一真相, 没有记录的目录不论有没有标注都得回收.
        assert!(!failed.exists());
        assert!(!sampled.exists());
        assert!(foreign.exists());

        // 采样前的清理不能动可能正在进行的采样, 也不能动已有日志的目录.
        let running = root.join("5e6f");
        let sampled = root.join("7a8b");
        let empty = root.join("9c0d");
        fs::create_dir_all(&running).unwrap();
        fs::create_dir_all(&sampled).unwrap();
        fs::create_dir_all(&empty).unwrap();
        fs::write(running.join(RECORDING_MARKER), "").unwrap();
        let log = sampled.join(format!(
            "{}_2026-11-08_01-00-00.csv",
            crate::testing::GAME_EXE_NAME
        ));
        fs::write(&log, "fps,frametime,elapsed\n").unwrap();

        store.set_recording(true).unwrap();
        store
            .prepare(
                &GameId::from("nte"),
                &environment("a"),
                facts(),
                &mut Command::new("game"),
            )
            .unwrap()
            .unwrap();
        assert!(!empty.exists());
        assert!(running.join(RECORDING_MARKER).exists());
        assert!(log.exists());
    }

    #[test]
    fn marks_sampling_directories_until_the_recording_is_collected() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        store.set_recording(true).unwrap();
        let recording = capture(&store, "a", 10.0);
        assert!(recording.directory().join(".recording").exists());
        store.finish(&recording).unwrap();
        assert!(!recording.directory().join(".recording").exists());
        assert!(dir
            .path()
            .join("records")
            .join(recording.id())
            .join("metadata.json")
            .exists());
    }

    #[test]
    fn upgrades_an_old_index_without_comparisons() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("index.json"),
            serde_json::json!({"active": null, "comparisons": []}).to_string(),
        )
        .unwrap();

        let store = Store::load(dir.path().into()).unwrap();
        assert!(store.data().records.is_empty());
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.path().join("index.json")).unwrap()).unwrap();
        assert!(persisted.get("comparisons").is_none());
        assert_eq!(persisted["records"], serde_json::json!([]));
    }

    #[test]
    fn deletes_index_entries_whose_id_cannot_name_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        store.set_recording(true).unwrap();
        let recording = capture(&store, "a", 10.0);
        let id = recording.id().to_string();
        store.finish(&recording).unwrap();
        drop(store);

        // 索引里出现形状不合法的条目时, 删除只移除条目本身.
        let path = dir.path().join("index.json");
        let mut index: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        index["records"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "id": "1A2B",
                "game": "nte",
                "startedAt": 1_700_000_000_000u64,
                "durationSeconds": 60.0,
                "frames": 600,
                "metrics": {
                    "fps": 60.0,
                    "low1": 50.0,
                    "p95": 16.0,
                    "p99": 20.0,
                    "stallPercent": 0.0,
                    "vramMeanMib": null,
                    "rssMeanMib": null,
                    "gpuLoadMean": null,
                    "powerMeanW": null
                },
                "facts": facts(),
                "environment": {"id": "a", "name": "Proton a", "proton": "GE-Proton-a"},
                "warnings": [],
                "pinned": false,
            }));
        fs::write(&path, index.to_string()).unwrap();

        let mut store = Store::load(dir.path().into()).unwrap();
        assert_eq!(store.data().records.len(), 2);
        store.delete("1A2B").unwrap();
        assert_eq!(store.data().records.len(), 1);
        assert!(store.record("1A2B").is_err());
        assert!(store.record(&id).is_ok());
        assert!(dir.path().join("records").join(&id).exists());

        // 索引里没有的 id 一律报错, 不能静默成功.
        for missing in ["", "..", "../records", "1a2b/3", "missing"] {
            assert!(store.delete(missing).is_err());
        }
        assert_eq!(store.data().records.len(), 1);
    }

    #[test]
    fn upgrades_the_old_comparison_index_into_pinned_records() {
        let dir = tempfile::tempdir().unwrap();
        let a = environment("a");
        let b = environment("b");
        let legacy = serde_json::json!({
            "active": "old",
            "comparisons": [{
                "id": "0badc0de",
                "name": "A/B",
                "createdAt": 1,
                "a": a,
                "b": b,
                "sessions": [
                    legacy_session("1a1a", "A", 100.0, 1_700_000_000_000),
                    legacy_session("2b2b", "B", 80.0, 1_700_000_100_000),
                    legacy_session("3c3c", "B", 90.0, 1_700_000_200_000),
                ],
                "error": "未找到 MangoHud",
            }],
        });
        fs::create_dir_all(dir.path().join("0badc0de")).unwrap();
        fs::write(dir.path().join("0badc0de").join("report.svg"), "<svg/>").unwrap();
        for id in ["1a1a", "2b2b", "3c3c"] {
            let session_dir = dir.path().join("0badc0de").join(id);
            fs::create_dir_all(&session_dir).unwrap();
            fs::write(
                session_dir.join("hardware.jsonl"),
                "{}
",
            )
            .unwrap();
        }
        fs::write(dir.path().join("index.json"), legacy.to_string()).unwrap();
        let store = Store::load(dir.path().into()).unwrap();
        let records = &store.data().records;
        assert_eq!(records.len(), 3);
        assert!(!store.recording_enabled());
        assert!(records.iter().all(|record| record.pinned));
        // 迁移后按启动时间倒序, 最新的记录排在最前.
        assert_eq!(records[0].metrics.fps, 90.0);
        assert_eq!(records[1].environment.name, "Proton b");
        assert_eq!(records[2].environment.name, "Proton a");
        assert_eq!(records[1].game, GameId::from("nte"));
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.path().join("index.json")).unwrap()).unwrap();
        assert!(persisted.get("comparisons").is_none());
        assert_eq!(persisted["records"].as_array().unwrap().len(), 3);
        assert_eq!(persisted["recordingEnabled"], false);
        // 原始日志和采样数据搬进记录目录, 旧比较目录连同报告产物一起清掉.
        for id in ["1a1a", "2b2b", "3c3c"] {
            assert!(dir
                .path()
                .join("records")
                .join(id)
                .join("hardware.jsonl")
                .exists());
            assert!(dir
                .path()
                .join("records")
                .join(id)
                .join("samples.json")
                .exists());
        }
        assert!(persisted["records"][0].get("histogram").is_none());
        assert_eq!(store.samples("3c3c").unwrap().points[0].fps, Some(90.0));
        assert!(!dir.path().join("old").exists());
    }

    #[test]
    fn renders_and_exports_a_report_for_two_records_with_b_minus_a_delta() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        store.set_recording(true).unwrap();
        let slow = capture(&store, "a", 20.0);
        let slow_id = slow.id().to_string();
        store.finish(&slow).unwrap();
        let fast = capture(&store, "b", 10.0);
        let fast_id = fast.id().to_string();
        store.finish(&fast).unwrap();

        let svg = store.svg(&slow_id, &fast_id).unwrap();
        assert!(svg.contains("Proton a"));
        assert!(svg.contains("Proton b"));
        assert!(svg.contains("GE-Proton-a"));
        assert!(svg.contains("UTC"));
        assert!(svg.contains("+100.0%"));
        assert!(svg.contains("Δ (B - A)"));
        let reversed = store.svg(&fast_id, &slow_id).unwrap();
        assert!(reversed.contains("-50.0%"));

        let path = store
            .export(&slow_id, &fast_id, &dir.path().join("报告"))
            .unwrap();
        assert_eq!(path.extension().unwrap(), "png");
        let png = fs::read(&path).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        let pixmap = resvg::tiny_skia::Pixmap::decode_png(&png).unwrap();
        assert!(
            pixmap
                .pixels()
                .iter()
                .take(pixmap.width() as usize * 55)
                .filter(|p| p.red() < 200 && p.green() < 200 && p.blue() < 200)
                .count()
                > 100,
            "PNG 标题文字必须实际渲染"
        );

        assert!(store.svg(&slow_id, &slow_id).is_err());
        assert!(store.svg(&slow_id, "missing").is_err());

        // 本地可选视觉检查输出, 默认测试不向临时目录外写入.
        if let Some(path) = std::env::var_os("ODDITY_PERFORMANCE_PREVIEW") {
            fs::write(path, png).unwrap();
        }
    }

    #[test]
    fn renders_reports_for_records_that_have_no_samples_left() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        store.set_recording(true).unwrap();
        let slow = capture(&store, "a", 20.0);
        let slow_id = slow.id().to_string();
        store.finish(&slow).unwrap();
        let fast = capture(&store, "b", 10.0);
        let fast_id = fast.id().to_string();
        store.finish(&fast).unwrap();
        for id in [&slow_id, &fast_id] {
            fs::remove_file(dir.path().join("records").join(id).join("samples.json")).unwrap();
        }

        let svg = store.svg(&slow_id, &fast_id).unwrap();
        assert!(svg.contains("Proton a"));
        assert!(svg.contains("Proton b"));
        assert!(svg.contains("平均 FPS"));
        let path = store
            .export(&slow_id, &fast_id, &dir.path().join("报告"))
            .unwrap();
        assert!(fs::read(path).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
    }
}
