//! Proton 性能比较的记录、统计与本地报告.
mod capture;
mod report;
mod statistics;

pub use capture::Recording;
pub use statistics::{analyze, Analysis, Metrics, Point, Session};

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::environment::RuntimeEnvironment;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub id: String,
    pub name: String,
    pub created_at: u64,
    pub a: RuntimeEnvironment,
    pub b: RuntimeEnvironment,
    pub sessions: Vec<Session>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub active: Option<String>,
    pub comparisons: Vec<Comparison>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary<'a> {
    active: Option<&'a str>,
    comparisons: Vec<ComparisonSummary<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ComparisonSummary<'a> {
    id: &'a str,
    name: &'a str,
    created_at: u64,
    a: &'a RuntimeEnvironment,
    b: &'a RuntimeEnvironment,
    sessions: Vec<SessionSummary<'a>>,
    error: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionSummary<'a> {
    id: &'a str,
    side: &'a str,
    started_at: u64,
    duration_seconds: f64,
    frames: usize,
    metrics: &'a Metrics,
    facts: &'a Facts,
    warnings: &'a [String],
}

pub struct Store {
    root: PathBuf,
    data: Overview,
}

impl Store {
    pub fn load(root: PathBuf) -> Result<Self> {
        let path = root.join("index.json");
        let data = if path.try_exists()? {
            serde_json::from_slice(&fs::read(path)?).context("性能比较记录无效")?
        } else {
            Overview::default()
        };
        Ok(Self { root, data })
    }

    #[cfg(test)]
    fn overview(&self) -> &Overview {
        &self.data
    }

    pub fn summary(&self, game: crate::game_profile::GameId) -> Summary<'_> {
        Summary {
            active: self.data.active.as_deref(),
            comparisons: self
                .data
                .comparisons
                .iter()
                .filter(|c| c.a.game == game)
                .map(|c| ComparisonSummary {
                    id: &c.id,
                    name: &c.name,
                    created_at: c.created_at,
                    a: &c.a,
                    b: &c.b,
                    error: c.error.as_deref(),
                    sessions: c
                        .sessions
                        .iter()
                        .map(|s| SessionSummary {
                            id: &s.id,
                            side: &s.side,
                            started_at: s.started_at,
                            duration_seconds: s.duration_seconds,
                            frames: s.frames,
                            metrics: &s.metrics,
                            facts: &s.facts,
                            warnings: &s.warnings,
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    pub fn create(
        &mut self,
        name: String,
        a: RuntimeEnvironment,
        b: RuntimeEnvironment,
    ) -> Result<()> {
        let name = name.trim().to_string();
        if name.is_empty() || name.chars().count() > 80 {
            bail!("比较名称须为 1-80 个字符");
        }
        if a.id == b.id
            || a.game != b.game
            || a.distribution != b.distribution
            || a.install_root != b.install_root
        {
            bail!("请选择同一游戏、地区和安装目录的两个不同运行环境");
        }
        let id = format!(
            "{:x}",
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        );
        let mut next = self.data.clone();
        next.active = Some(id.clone());
        next.comparisons.insert(
            0,
            Comparison {
                id,
                name,
                created_at: now_ms(),
                a,
                b,
                sessions: Vec::new(),
                error: None,
            },
        );
        self.commit(next)
    }

    pub fn activate(&mut self, id: Option<String>) -> Result<()> {
        if let Some(id) = &id {
            self.comparison(id)?;
        }
        let mut next = self.data.clone();
        next.active = id;
        self.commit(next)
    }

    pub fn comparison(&self, id: &str) -> Result<&Comparison> {
        self.data
            .comparisons
            .iter()
            .find(|c| c.id == id)
            .context("性能比较不存在")
    }

    pub fn prepare(
        &self,
        environment: &RuntimeEnvironment,
        facts: Facts,
        command: &mut std::process::Command,
    ) -> Result<Option<Recording>> {
        let Some(id) = &self.data.active else {
            return Ok(None);
        };
        let comparison = self.comparison(id)?;
        let expected = if environment.id == comparison.a.id {
            &comparison.a
        } else if environment.id == comparison.b.id {
            &comparison.b
        } else {
            return Ok(None);
        };
        if environment.proton != expected.proton
            || environment.install_root != expected.install_root
            || environment.game != expected.game
            || environment.distribution != expected.distribution
        {
            bail!("运行环境已变化, 请创建新的性能比较");
        }
        Recording::prepare(&self.root, comparison, environment, facts, command).map(Some)
    }

    pub fn fail(&mut self, error: String) -> Result<()> {
        let mut next = self.data.clone();
        if let Some(comparison) = next
            .comparisons
            .iter_mut()
            .find(|c| Some(&c.id) == next.active.as_ref())
        {
            comparison.error = Some(error);
            self.commit(next)?;
        }
        Ok(())
    }

    pub fn finish(&mut self, recording: &Recording) -> Result<()> {
        let session = recording.finish();
        let mut next = self.data.clone();
        let comparison = next
            .comparisons
            .iter_mut()
            .find(|c| c.id == recording.comparison)
            .context("性能比较不存在")?;
        match session {
            Ok(session) => {
                comparison.sessions.push(session);
                comparison.error = None;
            }
            Err(error) => comparison.error = Some(format!("本次采样失败: {error:#}")),
        }
        let directory = self.root.join(&comparison.id);
        if let Err(error) = report::write(comparison, &directory) {
            comparison.error = Some(format!("生成性能报告失败: {error:#}"));
        }
        self.commit(next)
    }

    pub fn report_svg(&self, id: &str) -> Result<String> {
        self.comparison(id)?;
        fs::read_to_string(self.root.join(id).join("report.svg"))
            .context("尚无性能报告, 请先完成一次游戏采样")
    }

    pub fn export(&self, id: &str, destination: &Path) -> Result<PathBuf> {
        self.comparison(id)?;
        if !destination.is_absolute() || destination.is_dir() {
            bail!("请选择 PNG 文件的绝对保存路径");
        }
        let source = self.root.join(id).join("report.png");
        let destination = destination.with_extension("png");
        let bytes = fs::read(source).context("尚无 PNG 报告")?;
        atomic_write(&destination, &bytes)?;
        Ok(destination)
    }

    fn commit(&mut self, next: Overview) -> Result<()> {
        fs::create_dir_all(&self.root)?;
        atomic_write(&self.root.join("index.json"), &serde_json::to_vec(&next)?)?;
        self.data = next;
        Ok(())
    }
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
    use crate::distribution::Distribution;
    use crate::environment::ProtonPin;
    use crate::game_profile::GameId;
    use std::process::Command;

    fn environment(id: &str) -> RuntimeEnvironment {
        RuntimeEnvironment {
            id: id.into(),
            name: format!("Proton {id}"),
            proton: ProtonPin {
                path: format!("/proton/{id}").into(),
                identity: id.into(),
            },
            install_root: "/game".into(),
            game: GameId::Nte,
            distribution: Distribution::Cn,
            created_at: 1,
        }
    }

    fn write_log(root: &Path, recording: &Recording, frametime: f64) {
        let directory = fs::read_dir(root.join(&recording.comparison))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .max_by_key(|e| e.file_name())
            .unwrap()
            .path();
        let mut csv = String::from("os,cpu,gpu\nLinux,CPU,GPU\nfps,frametime,elapsed\n");
        for frame in 1..=1500 {
            csv.push_str(&format!(
                "60,{frametime},{}\n",
                (frame as f64 * frametime * 1e6) as u64
            ));
        }
        fs::write(directory.join("HTGame_2026-11-08_01-00-00.csv"), csv).unwrap();
        let mut hardware = String::new();
        for second in 0..=(frametime * 1.5) as u64 {
            let point = Point {
                seconds: second as f64,
                gpu_load: Some(70.0 + (second as f64 * 0.6).sin() * 10.0),
                vram_mib: Some(4000.0 + frametime * 80.0 + second as f64 * 8.0),
                power_w: Some(80.0 + (second as f64 * 0.4).sin() * 6.0),
                rss_mib: Some(2400.0 + second as f64 * 12.0),
                ..Point::default()
            };
            hardware.push_str(&serde_json::to_string(&point).unwrap());
            hardware.push('\n');
        }
        fs::write(directory.join("hardware.jsonl"), hardware).unwrap();
    }

    #[test]
    fn automatically_generates_single_side_then_comparison_png_and_persists_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().into()).unwrap();
        store
            .create(
                "Proton A/B & 图表".into(),
                environment("a"),
                environment("b"),
            )
            .unwrap();
        let id = store.overview().active.clone().unwrap();
        let mut command = Command::new("game");
        let facts = Facts {
            game_executable: "HTGame".into(),
            game_version: Some("1.0".into()),
            display_backend: "XWayland".into(),
            ..Facts::default()
        };
        let a = store
            .prepare(&environment("a"), facts.clone(), &mut command)
            .unwrap()
            .unwrap();
        let config = command
            .get_envs()
            .find(|(k, _)| *k == "MANGOHUD_CONFIG")
            .unwrap()
            .1
            .unwrap()
            .to_string_lossy();
        assert!(config.contains("log_interval=0"));
        assert!(config.contains("upload_logs=0"));
        write_log(dir.path(), &a, 10.0);
        store.finish(&a).unwrap();
        let svg = store.report_svg(&id).unwrap();
        assert_eq!(store.overview().comparisons[0].sessions.len(), 1);
        assert!(svg.contains("A/B &amp; 图表"));
        let b = store
            .prepare(&environment("b"), facts, &mut command)
            .unwrap()
            .unwrap();
        write_log(dir.path(), &b, 20.0);
        store.finish(&b).unwrap();
        let path = store.export(&id, &dir.path().join("comparison")).unwrap();
        let png = fs::read(path).unwrap();
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
        let reloaded = Store::load(dir.path().into()).unwrap();
        let loaded = reloaded.overview();
        assert_eq!(loaded.comparisons[0].sessions.len(), 2);
        assert_eq!(loaded.comparisons[0].sessions[0].metrics.fps, 100.0);
        assert_eq!(loaded.comparisons[0].sessions[1].metrics.fps, 50.0);
        assert!(store.report_svg(&id).unwrap().contains("-50.0%"));
        let summary = serde_json::to_value(store.summary(GameId::Nte)).unwrap();
        assert!(summary["comparisons"][0]["sessions"][0]
            .get("points")
            .is_none());
        assert!(summary["comparisons"][0]["sessions"][0]
            .get("histogram")
            .is_none());
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.path().join("index.json")).unwrap()).unwrap();
        assert!(persisted["comparisons"][0]["sessions"][0]["points"].is_array());
        for session in &mut store.data.comparisons[0].sessions {
            session.points.resize(4000, Point::default());
        }
        assert_eq!(
            serde_json::to_value(store.summary(GameId::Nte)).unwrap(),
            summary
        );
        assert_eq!(
            serde_json::to_value(store.summary(GameId::Hsr)).unwrap()["comparisons"],
            serde_json::json!([])
        );

        // 本地可选视觉检查输出, 默认测试不向临时目录外写入.
        if let Some(path) = std::env::var_os("ODDITY_PERFORMANCE_PREVIEW") {
            fs::write(path, png).unwrap();
        }
    }

    #[test]
    fn rejects_incompatible_environments_and_does_not_capture_unselected_or_paused_runs() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().into()).unwrap();
        let mut other = environment("b");
        other.install_root = "/other".into();
        assert!(store
            .create("test".into(), environment("a"), other)
            .is_err());
        assert!(store
            .create("test".into(), environment("a"), environment("a"))
            .is_err());
        store
            .create("test".into(), environment("a"), environment("b"))
            .unwrap();
        let mut command = Command::new("game");
        assert!(store
            .prepare(&environment("c"), Facts::default(), &mut command)
            .unwrap()
            .is_none());
        store.activate(None).unwrap();
        assert!(store
            .prepare(&environment("a"), Facts::default(), &mut command)
            .unwrap()
            .is_none());
        assert_eq!(command.get_envs().count(), 0);
        assert!(store.activate(Some("missing".into())).is_err());
    }

    #[test]
    fn failed_capture_preserves_existing_sessions_and_reports_the_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().into()).unwrap();
        store
            .create("test".into(), environment("a"), environment("b"))
            .unwrap();
        let id = store.overview().active.clone().unwrap();
        let recording = store
            .prepare(
                &environment("a"),
                Facts {
                    game_executable: "HTGame".into(),
                    ..Facts::default()
                },
                &mut Command::new("game"),
            )
            .unwrap()
            .unwrap();
        store.finish(&recording).unwrap();
        assert!(store.overview().comparisons[0]
            .error
            .as_ref()
            .unwrap()
            .contains("实际找到 0 份"));
        assert!(store.report_svg(&id).unwrap().contains("采样失败"));
    }

    #[test]
    fn averages_sessions_equally_instead_of_weighting_by_frame_count() {
        let Analysis {
            metrics: m,
            histogram,
            points,
            frames,
            duration_seconds,
        } = analyze({
            let mut log = String::from("fps,frametime,elapsed\n");
            for i in 1..=1500 {
                log.push_str(&format!("100,10,{}\n", i * 10_000_000u64));
            }
            std::io::Cursor::new(log)
        })
        .unwrap();
        let mut a = Session {
            id: "a".into(),
            side: "A".into(),
            started_at: 1,
            duration_seconds,
            frames,
            metrics: m,
            histogram,
            points,
            facts: Facts::default(),
            warnings: vec![],
        };
        let mut b = a.clone();
        a.frames = 1_000_000;
        b.metrics.fps = 50.0;
        let aggregate = statistics::aggregate(&[&a, &b]).unwrap();
        assert_eq!(aggregate.fps, 75.0);
        assert!(aggregate.vram_mean_mib.is_none());
    }
}
