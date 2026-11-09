use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};

use super::{analyze, now_ms, Analysis, Comparison, Facts, GpuIdentity, Point, Session};
use crate::environment::RuntimeEnvironment;

#[derive(Debug)]
pub struct Recording {
    pub comparison: String,
    directory: PathBuf,
    id: String,
    side: String,
    started_at: u64,
    facts: Facts,
    origin: Option<Instant>,
    gpu: Option<String>,
}

impl Recording {
    pub fn prepare(
        root: &Path,
        comparison: &Comparison,
        environment: &RuntimeEnvironment,
        facts: Facts,
        command: &mut Command,
    ) -> Result<Self> {
        let id = format!(
            "{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        );
        let directory = root.join(&comparison.id).join(&id);
        fs::create_dir_all(&directory)?;
        let path = directory.to_string_lossy();
        if path.contains([',', '\n', '\r']) {
            bail!("性能采样目录不能包含逗号或换行")
        }
        let overlay = facts.game_mode.overlay;
        // read_cfg 保留既有画面与限帧设置, 仅覆盖本次采样所需选项.
        let mut config = std::env::var("MANGOHUD_CONFIG").unwrap_or_else(|_| "read_cfg".into());
        use std::fmt::Write as _;
        let _ = write!(config, ",autostart_log=1,log_duration=0,log_interval=0,log_versioning=0,output_folder={path},no_display={},permit_upload=0,upload_logs=0", u8::from(!overlay));
        command.env("MANGOHUD", "1").env("MANGOHUD_CONFIG", &config);
        fs::write(directory.join("sampling-config.txt"), config)?;
        fs::write(
            directory.join("metadata.json"),
            serde_json::to_vec_pretty(&facts)?,
        )?;
        Ok(Self {
            comparison: comparison.id.clone(),
            directory,
            id,
            side: if comparison.a.id == environment.id {
                "A"
            } else {
                "B"
            }
            .into(),
            started_at: now_ms(),
            facts,
            origin: None,
            gpu: None,
        })
    }

    /// 在已有的游戏进程探测之后采样, 不扫描或采集无关进程的环境.
    pub fn sample(&mut self, pids: &[u32]) -> Result<()> {
        let mut point = Point {
            seconds: self
                .origin
                .get_or_insert_with(Instant::now)
                .elapsed()
                .as_secs_f64(),
            ..Point::default()
        };
        let mut rss = None;
        let mut swap = None;
        for pid in pids {
            if let Ok(status) = fs::read_to_string(format!("/proc/{pid}/status")) {
                for line in status.lines() {
                    if let Some((key, value)) = line.split_once(':') {
                        let value = value
                            .split_whitespace()
                            .next()
                            .and_then(|v| v.parse::<f64>().ok());
                        if let Some(value) = value {
                            match key {
                                "VmRSS" => *rss.get_or_insert(0.0) += value / 1024.0,
                                "VmSwap" => *swap.get_or_insert(0.0) += value / 1024.0,
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
        point.rss_mib = rss;
        point.swap_mib = swap;
        point.system_available_mib = fs::read_to_string("/proc/meminfo").ok().and_then(|s| {
            s.lines().find_map(|l| {
                l.strip_prefix("MemAvailable:")
                    .and_then(|v| v.split_whitespace().next()?.parse::<f64>().ok())
                    .map(|v| v / 1024.0)
            })
        });
        // XML 同时列出图形与计算上下文, 按游戏 PID 找到参与渲染的显卡.
        if self.gpu.is_none() {
            self.gpu = query(&["-q", "-x"]).and_then(|xml| graphics_gpu(&xml, pids));
        }
        if let Some(output) = self.gpu.as_ref().and_then(|_| query(&["--query-gpu=uuid,utilization.gpu,memory.used,power.draw,temperature.gpu,name,driver_version", "--format=csv,noheader,nounits"])) {
            for line in output.lines() {
                let fields: Vec<_> = line.split(',').map(str::trim).collect();
                if fields.len() != 7 || Some(fields[0]) != self.gpu.as_deref() { continue; }
                let number = |i: usize| fields[i].parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0);
                point.gpu_load = number(1); point.vram_mib = number(2); point.power_w = number(3); point.temperature_c = number(4);
                self.facts.gpu = Some(GpuIdentity {uuid:fields[0].into(), name:fields[5].into(), driver:fields[6].into()});
            }
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.directory.join("hardware.jsonl"))?;
        serde_json::to_writer(&mut file, &point)?;
        writeln!(file)?;
        Ok(())
    }

    pub fn finish(&self) -> Result<Session> {
        let game_exe = &self.facts.game_executable;
        let mut files = fs::read_dir(&self.directory)?.collect::<std::io::Result<Vec<_>>>()?;
        files.retain(|file| {
            let name = file.file_name().to_string_lossy().into_owned();
            name.starts_with(&format!("{game_exe}_"))
                && name.ends_with(".csv")
                && !name.ends_with("_summary.csv")
        });
        // 多日志意味着热键分段或多个交换链, 首版不猜测如何拼接它们.
        if files.len() != 1 {
            bail!(
                "需要一份连续的游戏帧日志, 实际找到 {} 份; 请确认 MangoHud 已生效且未手动切换日志",
                files.len()
            )
        }
        let file = fs::File::open(files[0].path())?;
        let Analysis {
            mut metrics,
            histogram,
            mut points,
            frames,
            duration_seconds,
        } = analyze(file)?;
        let mut warnings = Vec::new();
        if let Ok(file) = fs::File::open(self.directory.join("hardware.jsonl")) {
            for line in BufReader::new(file).lines() {
                let point: Point = serde_json::from_str(&line?)?;
                points.push(point);
            }
        }
        if !points.iter().any(|p| p.gpu_load.is_some()) {
            warnings.push("GPU 指标不可用; 不影响帧时间比较".into());
        }
        let mean = |field: fn(&Point) -> Option<f64>| {
            let values: Vec<_> = points
                .iter()
                .filter(|p| p.seconds >= 5.0)
                .filter_map(field)
                .collect();
            (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
        };
        metrics.vram_mean_mib = mean(|p| p.vram_mib);
        metrics.rss_mean_mib = mean(|p| p.rss_mib);
        metrics.gpu_load_mean = mean(|p| p.gpu_load);
        metrics.power_mean_w = mean(|p| p.power_w);
        points.sort_by(|a, b| a.seconds.total_cmp(&b.seconds));
        fs::write(
            self.directory.join("metadata.json"),
            serde_json::to_vec_pretty(&self.facts)?,
        )?;
        let session = Session {
            id: self.id.clone(),
            side: self.side.clone(),
            started_at: self.started_at,
            duration_seconds,
            frames,
            metrics,
            histogram,
            points,
            facts: self.facts.clone(),
            warnings,
        };
        fs::write(
            self.directory.join("session.json"),
            serde_json::to_vec(&session)?,
        )?;
        Ok(session)
    }
}

/// NVIDIA XML 是外部数据, 在此解析为确定的显卡和进程类型.
fn graphics_gpu(xml: &str, pids: &[u32]) -> Option<String> {
    use serde::Deserialize;
    #[derive(Deserialize)]
    struct Inventory {
        #[serde(rename = "gpu", default)]
        gpus: Vec<Gpu>,
    }
    #[derive(Deserialize)]
    struct Gpu {
        uuid: String,
        #[serde(default)]
        processes: Processes,
    }
    #[derive(Default, Deserialize)]
    struct Processes {
        #[serde(rename = "process_info", default)]
        entries: Vec<Process>,
    }
    #[derive(Deserialize)]
    struct Process {
        pid: u32,
        #[serde(rename = "type")]
        kind: String,
    }
    let inventory: Inventory = quick_xml::de::from_str(xml).ok()?;
    inventory
        .gpus
        .into_iter()
        .find(|gpu| {
            gpu.processes
                .entries
                .iter()
                .any(|p| pids.contains(&p.pid) && p.kind.split('+').any(|kind| kind.trim() == "G"))
        })
        .map(|gpu| gpu.uuid)
}

// XML 可能超过管道容量, 使用临时文件接收, 避免等待子进程时堵住其输出.
fn query(arguments: &[&str]) -> Option<String> {
    let mut output = tempfile::tempfile().ok()?;
    let mut child = Command::new("nvidia-smi")
        .args(arguments)
        .stdout(Stdio::from(output.try_clone().ok()?))
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                output.seek(SeekFrom::Start(0)).ok()?;
                let mut text = String::new();
                output.read_to_string(&mut text).ok()?;
                return Some(text);
            }
            Ok(None) if start.elapsed() < Duration::from_millis(200) => {
                std::thread::sleep(Duration::from_millis(10))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::ProtonPin;
    use std::os::unix::fs::PermissionsExt;

    fn recording(root: &Path) -> Recording {
        let a = RuntimeEnvironment {
            id: "a".into(),
            name: "A".into(),
            proton: ProtonPin {
                path: "/proton".into(),
                identity: "pin".into(),
            },
            install_root: "/game".into(),
            game: Default::default(),
            distribution: Default::default(),
            created_at: 1,
        };
        let mut b = a.clone();
        b.id = "b".into();
        let comparison = Comparison {
            id: "comparison".into(),
            name: "test".into(),
            created_at: 1,
            a: a.clone(),
            b,
            sessions: vec![],
            error: None,
        };
        Recording::prepare(
            root,
            &comparison,
            &a,
            Facts {
                game_executable: "HTGame".into(),
                ..Facts::default()
            },
            &mut Command::new("game"),
        )
        .unwrap()
    }

    #[test]
    #[ignore = "需要 NVIDIA 和可见的 Vulkan 图形进程"]
    fn samples_real_vulkan_process() {
        let _env = crate::testing::EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let mut cube = Command::new("vkcube")
            .args([
                "--wsi", "xcb", "--width", "100", "--height", "100", "--c", "1200",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("启动 Vulkan 示例");
        let mut recording = recording(dir.path());
        for _ in 0..20 {
            recording.sample(&[cube.id()]).unwrap();
            if recording.facts.gpu.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = cube.kill();
        let _ = cube.wait();
        let points: Vec<Point> = fs::read_to_string(recording.directory.join("hardware.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert!(recording.facts.gpu.is_some(), "未找到 Vulkan 进程的 GPU");
        assert!(points
            .iter()
            .any(|p| p.vram_mib.is_some() && p.gpu_load.is_some() && p.power_w.is_some()));
        eprintln!("Vulkan GPU: {:?}", recording.facts.gpu);
    }

    #[test]
    fn maps_graphics_and_mixed_contexts_without_selecting_compute_only_processes() {
        let xml = r#"<?xml version="1.0"?><nvidia_smi_log><gpu><uuid>compute</uuid><processes><process_info><pid>42</pid><type>C</type></process_info></processes></gpu><gpu><uuid>graphics</uuid><processes><process_info><pid>42</pid><type>C+G</type></process_info></processes></gpu><gpu><uuid>idle</uuid></gpu></nvidia_smi_log>"#;
        assert_eq!(graphics_gpu(xml, &[42]).as_deref(), Some("graphics"));
        assert_eq!(graphics_gpu(xml, &[43]), None);
        assert_eq!(graphics_gpu("broken XML", &[42]), None);
    }

    #[test]
    fn hardware_sampling_selects_the_game_gpu_and_preserves_missing_counters() {
        let mut env = crate::testing::EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("nvidia-smi");
        let pid = std::process::id();
        let padding = "x".repeat(96 * 1024);
        fs::write(&executable, format!("#!/bin/sh\ncase \"$1\" in\n-q) echo '<nvidia_smi_log><gpu><uuid>GPU-other</uuid><processes><process_info><pid>1</pid><type>G</type></process_info></processes></gpu><gpu><uuid>GPU-game</uuid><processes><process_info><pid>{pid}</pid><type>G</type><process_name>{padding}</process_name></process_info></processes></gpu></nvidia_smi_log>';;\n--query-gpu=*) echo 'GPU-other, 5, 100, 10, 40, other, 1'; echo 'GPU-game, 85, 6000, 90, 63, Test GPU, 615';;\n*) exit 1;;\nesac\n")).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        env.set("PATH", dir.path());
        let mut recording = recording(dir.path());
        recording.origin = Some(Instant::now() - Duration::from_secs(6));
        recording.sample(&[pid]).unwrap();
        let point: Point = serde_json::from_str(
            fs::read_to_string(recording.directory.join("hardware.jsonl"))
                .unwrap()
                .trim(),
        )
        .unwrap();
        assert_eq!(point.gpu_load, Some(85.0));
        assert_eq!(point.vram_mib, Some(6000.0));
        assert_eq!(point.power_w, Some(90.0));
        assert!(point.rss_mib.unwrap() > 0.0);
        assert_eq!(recording.facts.gpu.unwrap().uuid, "GPU-game");
    }
}
