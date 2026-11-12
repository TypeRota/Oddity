//! 把 comparisons 索引升级为记录索引.

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;

use super::{EnvironmentSnapshot, Facts, Metrics, Overview, Point, Record, Samples};
use crate::environment::RuntimeEnvironment;

/// 旧索引留下的待迁移数据: 会话目录搬进 `records/` 之后, 采样数据再按这里的对应关系写入.
#[derive(Clone, Default)]
pub(super) struct Legacy {
    pub comparisons: Vec<LegacyComparison>,
}

impl Legacy {
    pub fn is_empty(&self) -> bool {
        self.comparisons.is_empty()
    }
}

/// 一个旧比较目录, 以及其中会话的采样数据.
#[derive(Clone)]
pub(super) struct LegacyComparison {
    pub id: String,
    pub sessions: Vec<LegacySession>,
}

/// 一条旧会话的采样数据.
#[derive(Clone)]
pub(super) struct LegacySession {
    pub id: String,
    pub samples: Samples,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyOverview {
    #[serde(default)]
    comparisons: Vec<LegacyJsonComparison>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyJsonComparison {
    id: String,
    a: RuntimeEnvironment,
    b: RuntimeEnvironment,
    #[serde(default)]
    sessions: Vec<LegacyJsonSession>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyJsonSession {
    id: String,
    side: String,
    started_at: u64,
    duration_seconds: f64,
    frames: usize,
    metrics: Metrics,
    histogram: Vec<f64>,
    #[serde(default)]
    points: Vec<Point>,
    #[serde(default)]
    facts: Facts,
    #[serde(default)]
    warnings: Vec<String>,
}

pub(super) fn overview(value: Value) -> Result<(Overview, Legacy)> {
    let legacy: LegacyOverview = serde_json::from_value(value).context("性能记录索引无效")?;
    let mut records = Vec::new();
    let mut comparisons = Vec::new();
    for comparison in legacy.comparisons {
        let game = comparison.a.game.clone();
        let mut sessions = Vec::new();
        for session in comparison.sessions {
            let environment = if session.side == "A" {
                &comparison.a
            } else {
                &comparison.b
            };
            sessions.push(LegacySession {
                id: session.id.clone(),
                samples: Samples {
                    histogram: session.histogram,
                    points: session.points,
                },
            });
            records.push(Record {
                id: session.id,
                game: game.clone(),
                started_at: session.started_at,
                duration_seconds: session.duration_seconds,
                frames: session.frames,
                metrics: session.metrics,
                facts: session.facts,
                environment: EnvironmentSnapshot::capture(environment),
                warnings: session.warnings,
                // 迁移来的记录一律固定, 不参与保留策略的淘汰.
                pinned: true,
            });
        }
        comparisons.push(LegacyComparison {
            id: comparison.id,
            sessions,
        });
    }
    // 记录列表按启动时间倒序, 与后续新增记录的顺序一致.
    records.sort_by_key(|record| std::cmp::Reverse(record.started_at));
    Ok((
        Overview {
            recording_enabled: false,
            records,
        },
        Legacy { comparisons },
    ))
}
