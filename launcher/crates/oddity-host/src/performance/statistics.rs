use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metrics {
    pub fps: f64,
    pub low1: f64,
    pub p95: f64,
    pub p99: f64,
    pub stall_percent: f64,
    pub vram_mean_mib: Option<f64>,
    pub rss_mean_mib: Option<f64>,
    pub gpu_load_mean: Option<f64>,
    pub power_mean_w: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    pub seconds: f64,
    pub fps: Option<f64>,
    pub frametime_ms: Option<f64>,
    pub gpu_load: Option<f64>,
    pub vram_mib: Option<f64>,
    pub power_w: Option<f64>,
    pub temperature_c: Option<f64>,
    pub rss_mib: Option<f64>,
    pub swap_mib: Option<f64>,
    pub system_available_mib: Option<f64>,
}

pub struct Analysis {
    pub metrics: Metrics,
    pub histogram: Vec<f64>,
    pub points: Vec<Point>,
    pub frames: usize,
    pub duration_seconds: f64,
}

/// MangoHud elapsed 为纳秒, frametime 为毫秒. 必须输入逐帧日志.
pub fn analyze(input: impl Read) -> Result<Analysis> {
    let mut reader = BufReader::new(input);
    let mut header = String::new();
    loop {
        header.clear();
        if reader.read_line(&mut header)? == 0 {
            bail!("未找到 MangoHud 帧时间表头")
        }
        if header.trim_start().starts_with("fps,frametime,") {
            break;
        }
    }
    let columns: Vec<_> = header.trim().split(',').collect();
    let time_index = columns
        .iter()
        .position(|c| *c == "elapsed")
        .context("帧日志缺少 elapsed")?;
    let mut csv = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(reader);
    let mut frames = Vec::new();
    let mut bins: BTreeMap<u64, (usize, f64, f64)> = BTreeMap::new();
    let mut previous = None;
    let mut first = None;
    let mut end: f64 = 0.0;
    for row in csv.records() {
        let row = row.context("帧日志 CSV 无效")?;
        let frametime: f64 = row
            .get(1)
            .context("帧时间缺失")?
            .parse()
            .context("帧时间无效")?;
        let seconds: f64 = row
            .get(time_index)
            .context("elapsed 缺失")?
            .parse::<f64>()
            .context("elapsed 无效")?
            / 1e9;
        if !frametime.is_finite() || frametime <= 0.0 || !seconds.is_finite() || seconds < 0.0 {
            bail!("帧日志包含无效数值");
        }
        if previous.is_some_and(|p| seconds <= p) {
            bail!("帧日志时间未递增")
        }
        previous = Some(seconds);
        if seconds < 5.0 {
            continue;
        }
        first.get_or_insert(seconds);
        end = seconds;
        frames.push(frametime);
        let bin = bins.entry((seconds / 10.0) as u64).or_default();
        bin.0 += 1;
        bin.1 += frametime;
        bin.2 += seconds;
    }
    let duration = end - first.unwrap_or(end);
    if frames.len() < 60 || duration < 5.0 {
        bail!("有效采样不足, 请至少游玩 10 秒")
    }
    // 100 ms 抽样不能计算逐帧 1% Low, 防止旧日志混入新报告.
    let total: f64 = frames.iter().sum();
    if (total / 1000.0 - duration).abs() > duration * 0.25 + 2.0 {
        bail!("日志不是连续逐帧记录, 无法计算逐帧统计");
    }
    frames.sort_unstable_by(f64::total_cmp);
    let percentile = |p: f64| frames[((frames.len() as f64 * p).ceil() as usize).saturating_sub(1)];
    let slow = (frames.len() as f64 * 0.01).ceil() as usize;
    let metrics = Metrics {
        vram_mean_mib: None,
        rss_mean_mib: None,
        gpu_load_mean: None,
        power_mean_w: None,
        fps: frames.len() as f64 * 1000.0 / total,
        low1: slow as f64 * 1000.0 / frames[frames.len() - slow..].iter().sum::<f64>(),
        p95: percentile(0.95),
        p99: percentile(0.99),
        stall_percent: frames
            .iter()
            .filter(|v| **v > 50.0)
            .fold(0.0, |sum, v| sum + v)
            / total
            * 100.0,
    };
    let mut histogram = vec![0.0; 11];
    for value in &frames {
        histogram[((*value / 10.0) as usize).min(10)] += 100.0 / frames.len() as f64;
    }
    let points = bins
        .into_iter()
        .map(|(_, (count, sum, seconds))| Point {
            seconds: seconds / count as f64,
            fps: Some(count as f64 * 1000.0 / sum),
            frametime_ms: Some(sum / count as f64),
            ..Point::default()
        })
        .collect();
    Ok(Analysis {
        metrics,
        histogram,
        points,
        frames: frames.len(),
        duration_seconds: duration,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn log(times: &[f64], step: bool) -> String {
        let mut log = String::from("os,cpu,gpu\nLinux,CPU,GPU\nfps,frametime,elapsed\n");
        let mut elapsed = 0.0;
        for t in times {
            elapsed += if step { 100.0 } else { *t };
            log.push_str(&format!("60,{t},{}\n", elapsed * 1e6));
        }
        log
    }
    #[test]
    fn calculates_frame_weighted_average_tail_and_stall_duration() {
        let mut times = vec![10.0; 1500];
        times.extend(vec![100.0; 10]);
        let Analysis {
            metrics: m,
            histogram,
            frames,
            ..
        } = analyze(log(&times, false).as_bytes()).unwrap();
        assert_eq!(frames, 1011);
        assert!((m.fps - 1_011_000.0 / 11_010.0).abs() < 1e-9);
        assert_eq!(m.p99, 10.0);
        assert!((m.low1 - 10.89).abs() < 0.1);
        assert!((m.stall_percent - 100_000.0 / 11_010.0).abs() < 1e-9);
        assert!((histogram.iter().sum::<f64>() - 100.0).abs() < 1e-6);
    }
    #[test]
    fn rejects_sampled_logs_nonfinite_and_short_runs() {
        assert!(analyze(log(&vec![16.0; 2000], true).as_bytes()).is_err());
        assert!(analyze(log(&[16.0; 5], false).as_bytes()).is_err());
        assert!(analyze(b"fps,frametime,elapsed\n60,NaN,6000000000\n".as_slice()).is_err());
    }
}
