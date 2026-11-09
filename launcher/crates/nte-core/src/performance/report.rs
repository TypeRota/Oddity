use super::statistics::aggregate;
use super::{atomic_write, Comparison, Point};
use anyhow::{Context, Result};
use std::fmt::Write as _;
use std::path::Path;

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub fn write(comparison: &Comparison, directory: &Path) -> Result<()> {
    let svg = svg(comparison);
    atomic_write(
        &directory.join("report.json"),
        &serde_json::to_vec_pretty(comparison)?,
    )?;
    atomic_write(&directory.join("report.svg"), svg.as_bytes())?;
    static FONTS: std::sync::OnceLock<std::sync::Arc<resvg::usvg::fontdb::Database>> =
        std::sync::OnceLock::new();
    let fonts = FONTS.get_or_init(|| {
        let mut fonts = resvg::usvg::fontdb::Database::new();
        fonts.load_system_fonts();
        fonts.load_font_data(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../ui/public/assets/fonts/SpaceGrotesk.ttf"
            ))
            .to_vec(),
        );
        let family = [
            "Noto Sans CJK SC",
            "Source Han Sans SC",
            "WenQuanYi Micro Hei",
        ]
        .into_iter()
        .find(|name| {
            fonts
                .query(&resvg::usvg::fontdb::Query {
                    families: &[resvg::usvg::fontdb::Family::Name(name)],
                    ..Default::default()
                })
                .is_some()
        })
        .unwrap_or("Space Grotesk");
        fonts.set_sans_serif_family(family);
        std::sync::Arc::new(fonts)
    });
    let options = resvg::usvg::Options {
        fontdb: fonts.clone(),
        ..Default::default()
    };
    let tree = resvg::usvg::Tree::from_str(&svg, &options)?;
    let size = tree.size().to_int_size();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size.width(), size.height())
        .context("无法分配性能图表画布")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::default(),
        &mut pixmap.as_mut(),
    );
    atomic_write(&directory.join("report.png"), &pixmap.encode_png()?)?;
    Ok(())
}

pub fn svg(comparison: &Comparison) -> String {
    let a: Vec<_> = comparison
        .sessions
        .iter()
        .filter(|s| s.side == "A")
        .collect();
    let b: Vec<_> = comparison
        .sessions
        .iter()
        .filter(|s| s.side == "B")
        .collect();
    let metrics = [aggregate(&a), aggregate(&b)];
    let latest = [a.last().copied(), b.last().copied()];
    let colors = ["#2463a3", "#c76d20"];
    type PointField = fn(&Point) -> Option<f64>;
    let fields: [(&str, PointField); 6] = [
        ("FPS (10 s)", |p| p.fps),
        ("帧时间 (ms, 10 s)", |p| p.frametime_ms),
        ("显存 (MiB)", |p| p.vram_mib),
        ("GPU 负载 (%)", |p| p.gpu_load),
        ("GPU 功耗 (W)", |p| p.power_w),
        ("RSS (MiB)", |p| p.rss_mib),
    ];
    let panels: Vec<_> = fields
        .iter()
        .filter(|(_, field)| {
            latest
                .iter()
                .flatten()
                .flat_map(|s| &s.points)
                .any(|p| field(p).is_some())
        })
        .collect();
    let height = if panels.is_empty() {
        900
    } else {
        1130 + (panels.len() - 1) * 208
    };
    let mut svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1440\" height=\"{height}\" viewBox=\"0 0 1440 {height}\"><rect width=\"1440\" height=\"{height}\" fill=\"#fff\"/><g font-family=\"sans-serif\" fill=\"#20272e\" font-size=\"18\">");
    let _ = write!(
        svg,
        "<text x=\"60\" y=\"46\" font-size=\"27\">{}</text>",
        escape(&comparison.name.chars().take(48).collect::<String>())
    );
    for (i, environment) in [&comparison.a, &comparison.b].iter().enumerate() {
        let version = environment
            .proton
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let sessions = if i == 0 { &a } else { &b };
        let _ = write!(
            svg,
            "<text x=\"60\" y=\"{}\" fill=\"{}\">{} | {} | {} | n={} | {:.0} s</text>",
            82 + i * 30,
            colors[i],
            if i == 0 { "A" } else { "B" },
            escape(&environment.name.chars().take(32).collect::<String>()),
            escape(&version.chars().take(48).collect::<String>()),
            sessions.len(),
            sessions.iter().map(|s| s.duration_seconds).sum::<f64>()
        );
    }
    svg.push_str("<text x=\"60\" y=\"174\">指标</text><text x=\"620\" y=\"174\">A</text><text x=\"850\" y=\"174\">B</text><text x=\"1080\" y=\"174\">Δ (B - A)</text><text x=\"1260\" y=\"174\">Δ%</text>");
    type MetricField = fn(&super::Metrics) -> Option<f64>;
    let rows: [(&str, MetricField); 9] = [
        ("平均 FPS", |m| Some(m.fps)),
        ("1% Low FPS", |m| Some(m.low1)),
        ("P95 帧时间 (ms)", |m| Some(m.p95)),
        ("P99 帧时间 (ms)", |m| Some(m.p99)),
        ("帧时间 >50 ms 占比 (%)", |m| Some(m.stall_percent)),
        ("平均显存 (MiB)", |m| m.vram_mean_mib),
        ("平均 RSS (MiB)", |m| m.rss_mean_mib),
        ("平均 GPU 负载 (%)", |m| m.gpu_load_mean),
        ("平均 GPU 功耗 (W)", |m| m.power_mean_w),
    ];
    for (index, (label, field)) in rows.iter().enumerate() {
        let y = 208 + index * 32;
        let _ = write!(svg, "<text x=\"60\" y=\"{y}\">{label}</text>");
        for (side, metric) in metrics.iter().enumerate() {
            let value = metric
                .as_ref()
                .and_then(field)
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "—".into());
            let _ = write!(
                svg,
                "<text x=\"{}\" y=\"{y}\" fill=\"{}\">{value}</text>",
                620 + side * 230,
                colors[side]
            );
        }
        if let (Some(a), Some(b)) = (
            metrics[0].as_ref().and_then(field),
            metrics[1].as_ref().and_then(field),
        ) {
            let delta = b - a;
            let relative = if a == 0.0 {
                "—".into()
            } else {
                format!("{:+.1}%", delta / a * 100.0)
            };
            let _ = write!(svg,"<text x=\"1080\" y=\"{y}\">{delta:+.2}</text><text x=\"1260\" y=\"{y}\">{relative}</text>");
        }
    }
    svg.push_str("<text x=\"60\" y=\"516\" font-size=\"22\">帧时间分布 (%)</text>");
    for (side, sessions) in [&a, &b].iter().enumerate() {
        if sessions.is_empty() {
            continue;
        }
        for bin in 0..11 {
            let percentage =
                sessions.iter().map(|s| s.histogram[bin]).sum::<f64>() / sessions.len() as f64;
            let height = percentage * 1.5;
            let x = 95 + bin * 116 + side * 35;
            let _ = write!(
                svg,
                "<rect x=\"{x}\" y=\"{}\" width=\"32\" height=\"{height}\" fill=\"{}\"/>",
                710.0 - height,
                colors[side]
            );
            if percentage >= 0.1 {
                let _ = write!(
                    svg,
                    "<text x=\"{x}\" y=\"{}\" font-size=\"12\">{percentage:.1}</text>",
                    700.0 - height
                );
            }
        }
    }
    for bin in 0..11 {
        let label = if bin == 10 {
            "100+".into()
        } else {
            format!("{}-{}", bin * 10, (bin + 1) * 10)
        };
        let _ = write!(
            svg,
            "<text x=\"{}\" y=\"740\" font-size=\"14\">{label} ms</text>",
            95 + bin * 116
        );
    }
    if !panels.is_empty() {
        svg.push_str("<text x=\"60\" y=\"790\" font-size=\"22\">时间线</text>");
        for (side, session) in latest.iter().enumerate() {
            if let Some(s) = session {
                let seconds = (s.started_at / 1000) as libc::time_t;
                let mut time = std::mem::MaybeUninit::<libc::tm>::uninit();
                // gmtime_r 成功时初始化整个 tm, 报告明确标注 UTC.
                let timestamp = if unsafe { libc::gmtime_r(&seconds, time.as_mut_ptr()) }.is_null()
                {
                    s.started_at.to_string()
                } else {
                    let time = unsafe { time.assume_init() };
                    format!(
                        "{:04}-{:02}-{:02} {:02}:{:02} UTC",
                        time.tm_year + 1900,
                        time.tm_mon + 1,
                        time.tm_mday,
                        time.tm_hour,
                        time.tm_min
                    )
                };
                let _ = write!(svg,"<text x=\"60\" y=\"{}\" fill=\"{}\" font-size=\"16\">{} | {} | {} | {} | {:.0} s</text>",820+side*25,colors[side],s.side,timestamp,escape(s.facts.game_version.as_deref().unwrap_or("—")),escape(&s.facts.display_backend),s.duration_seconds);
            }
        }
    }
    let max_time = latest
        .iter()
        .flatten()
        .flat_map(|s| &s.points)
        .map(|p| p.seconds)
        .fold(1.0, f64::max);
    for (index, (label, field)) in panels.iter().enumerate() {
        let top = 906.0 + index as f64 * 208.0;
        let max_value = latest
            .iter()
            .flatten()
            .flat_map(|s| &s.points)
            .filter_map(field)
            .fold(1.0, f64::max);
        let _ = write!(
            svg,
            "<text x=\"120\" y=\"{}\" font-size=\"18\">{label}</text>",
            top - 26.0
        );
        for tick in 0..=2 {
            let y = top + 130.0 - tick as f64 * 65.0;
            let _ = write!(svg,"<path d=\"M120 {y} H1370\" stroke=\"#e1e5e9\"/><text x=\"100\" y=\"{}\" text-anchor=\"end\" font-size=\"14\">{:.0}</text>",y+5.0,max_value*tick as f64/2.0);
        }
        for tick in 0..=4 {
            let x = 120.0 + tick as f64 * 312.5;
            let _ = write!(svg, "<path d=\"M{x} {top} v130\" stroke=\"#e1e5e9\"/>");
            if index == panels.len() - 1 {
                let _ = write!(
                    svg,
                    "<text x=\"{x}\" y=\"{}\" text-anchor=\"middle\" font-size=\"14\">{:.0}</text>",
                    top + 158.0,
                    max_time * tick as f64 / 4.0
                );
            }
        }
        for (side, session) in latest.iter().enumerate() {
            let Some(session) = session else {
                continue;
            };
            let mut path = String::new();
            for p in &session.points {
                if let Some(value) = field(p) {
                    let x = 120.0 + p.seconds / max_time * 1250.0;
                    let y = top + 130.0 - value / max_value * 130.0;
                    let _ = write!(
                        path,
                        "{} {x:.1} {y:.1} ",
                        if path.is_empty() { "M" } else { "L" }
                    );
                }
            }
            let _ = write!(
                svg,
                "<path d=\"{path}\" fill=\"none\" stroke=\"{}\" stroke-width=\"2\"/>",
                colors[side]
            );
        }
    }
    if !panels.is_empty() {
        let _ = write!(
            svg,
            "<text x=\"745\" y=\"{}\" text-anchor=\"middle\" font-size=\"16\">t (s)</text>",
            height - 28
        );
    }
    if let Some(error) = &comparison.error {
        let _ = write!(
            svg,
            "<text x=\"60\" y=\"{}\" fill=\"#a43b31\" font-size=\"15\">{}</text>",
            height - 6,
            escape(error)
        );
    }
    svg.push_str("</g></svg>");
    svg
}
