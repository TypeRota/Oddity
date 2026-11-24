//! 游戏内角色状态的只读网络同步.

mod capture;
mod projection;
mod protocol;
pub use projection::Recovery;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};

use oddity_runtime::HostContext;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::attendance::GameRecord;

pub(crate) const COMMANDS: &[&str] = &["game_state_status", "game_state_start", "game_state_stop"];

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerStateStatus {
    pub state: &'static str,
    pub enabled: bool,
    pub observation: Option<GameRecord>,
    pub capture: CaptureStatus,
    pub message: Option<String>,
    pub record: Option<GameRecord>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatus {
    pub pid: Option<u32>,
    pub ports: usize,
    pub packets: u64,
    pub records: u64,
    pub started_at: Option<u64>,
    pub last_packet_at: Option<u64>,
    pub last_record_at: Option<u64>,
    pub retry_at: Option<u64>,
    pub last_error: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct StoredRecord {
    distribution: String,
    #[serde(default)]
    enabled: bool,
    record: Option<GameRecord>,
    #[serde(default)]
    observation: Option<GameRecord>,
}

struct Worker {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

struct Manager {
    status: PlayerStateStatus,
    worker: Option<Worker>,
}

static MANAGER: OnceLock<Mutex<Manager>> = OnceLock::new();
static CONTROL: Mutex<()> = Mutex::new(());

fn cache_file() -> PathBuf {
    if cfg!(test) {
        return std::env::temp_dir()
            .join(format!("oddity-player-state-tests-{}", std::process::id()))
            .join("state.json");
    }
    crate::attendance::default_dir()
        .parent()
        .expect("签到配置目录包含父目录")
        .join("player-state-cn.json")
}

fn manager() -> &'static Mutex<Manager> {
    MANAGER.get_or_init(|| {
        let stored = std::fs::read(cache_file())
            .ok()
            .and_then(|raw| serde_json::from_slice::<StoredRecord>(&raw).ok())
            .filter(|stored| {
                stored.distribution == "cn"
                    && stored
                        .record
                        .as_ref()
                        .is_none_or(|record| record.role_id.parse::<u64>().is_ok_and(|id| id != 0))
            });
        let enabled = stored.as_ref().is_some_and(|stored| stored.enabled);
        let observation = stored
            .as_ref()
            .and_then(|stored| stored.observation.clone())
            .filter(|record| record.role_id.is_empty());
        let mut record = stored.and_then(|stored| stored.record);
        if let Some(record) = &mut record {
            adopt_snapshot_anchor(record);
        }
        Mutex::new(Manager {
            status: PlayerStateStatus {
                state: "disabled",
                enabled,
                observation,
                capture: CaptureStatus::default(),
                message: None,
                record,
            },
            worker: None,
        })
    })
}

/// 旧缓存只有快照时间时用它充当恢复锚点, 否则周本和活跃度都会缺一个起点.
fn adopt_snapshot_anchor(record: &mut GameRecord) {
    if record.updated_at == 0 {
        return;
    }
    let recovery = record.recovery.get_or_insert_default();
    let anchor = Some(record.updated_at * 1000);
    if record.weekly_remaining.is_some() {
        recovery.weekly_at = recovery.weekly_at.or(anchor);
    }
    if record.activity.is_some() {
        recovery.activity_at = recovery.activity_at.or(anchor);
    }
}

fn status() -> PlayerStateStatus {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let mut manager = manager().lock().expect("角色同步状态锁中毒");
    let state = &mut manager.status;
    if state.enabled
        && matches!(
            state.state,
            "reading" | "unidentified" | "capturing" | "stale"
        )
    {
        let capture = &state.capture;
        if capture
            .started_at
            .is_some_and(|started| now.saturating_sub(started) >= 15_000)
        {
            let quiet = capture
                .last_packet_at
                .is_none_or(|last| now.saturating_sub(last) >= 15_000);
            if capture.ports == 0 || quiet {
                state.state = "stale";
                state.message = Some(
                    if capture.ports == 0 {
                        "未找到游戏网络连接, 已有读数继续按时间计算."
                    } else {
                        "未收到游戏网络数据, 已有读数继续按时间计算."
                    }
                    .into(),
                );
            } else if capture.records == 0 || state.state == "stale" {
                state.message = Some("尚未读取到新的游戏资源, 已有读数继续按时间计算.".into());
            }
        }
    }
    let mut status = state.clone();
    drop(manager);
    status.record = status.record.map(|record| record.projected(now));
    status.observation = status.observation.map(|record| record.projected(now));
    status
}

fn change(state: &'static str, message: Option<&str>) {
    let mut manager = manager().lock().expect("角色同步状态锁中毒");
    manager.status.state = state;
    manager.status.message = message.map(str::to_owned);
    if state == "waiting" {
        manager.status.capture = CaptureStatus::default();
    }
}

fn install_record(mut record: GameRecord, destination: &std::path::Path) {
    let mut manager = manager().lock().expect("角色同步状态锁中毒");
    let status = &mut manager.status;
    if record.role_id.is_empty() {
        status.observation = Some(record);
        status.state = "unidentified";
        status.message = Some("已读取当前游戏资源, 正在等待角色身份.".into());
    } else {
        if let Some(previous) = status
            .record
            .as_ref()
            .filter(|previous| previous.role_id == record.role_id)
        {
            record.fill_missing(previous);
        }
        status.record = Some(record);
        status.observation = None;
        status.state = "capturing";
        status.message = None;
    }
    status.capture.records += 1;
    status.capture.last_record_at = Some(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    );
    status.capture.retry_at = None;
    status.capture.last_error = None;
    if !save(status, destination) {
        status.message = Some("资源数据已读取, 暂未保存到本机.".into());
    }
}

fn save(status: &PlayerStateStatus, destination: &std::path::Path) -> bool {
    serde_json::to_vec(&StoredRecord {
        distribution: "cn".into(),
        enabled: status.enabled,
        record: status.record.clone(),
        observation: status.observation.clone(),
    })
    .ok()
    .and_then(|raw| {
        use std::io::Write;
        std::fs::create_dir_all(destination.parent()?).ok()?;
        let mut file = oddity_platform::paths::TemporaryFile::with_mode(destination, 0o600).ok()?;
        file.file.write_all(&raw).ok()?;
        file.commit(destination, &|| false).ok()
    })
    .is_some()
}

pub(crate) fn initialize(host: &dyn HostContext) {
    if status().enabled {
        if let Ok(settings) = host.settings() {
            if settings.distribution == "cn" {
                start(
                    settings.install_root,
                    oddity_platform::process::find_executable("dumpcap"),
                );
            }
        }
    }
}

fn start(install_root: Option<String>, dumpcap: Option<PathBuf>) -> PlayerStateStatus {
    let _control = CONTROL.lock().expect("角色同步控制锁中毒");
    let mut manager = manager().lock().expect("角色同步状态锁中毒");
    if manager
        .worker
        .as_ref()
        .is_some_and(|worker| !worker.thread.is_finished())
    {
        return manager.status.clone();
    }
    manager.worker.take();
    manager.status.enabled = true;
    save(&manager.status, &cache_file());
    manager.status.state = if dumpcap.is_some() {
        "waiting"
    } else {
        "error"
    };
    manager.status.message = Some(
        if dumpcap.is_some() {
            "等待游戏启动."
        } else {
            capture::MISSING_DUMPCAP
        }
        .into(),
    );
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let thread = thread::spawn(move || {
        capture::run(&worker_stop, install_root.as_deref(), dumpcap.as_deref())
    });
    manager.worker = Some(Worker { stop, thread });
    manager.status.clone()
}

pub(crate) fn stop() {
    let _control = CONTROL.lock().expect("角色同步控制锁中毒");
    let worker = {
        let mut manager = manager().lock().expect("角色同步状态锁中毒");
        manager.status.state = "disabled";
        manager.status.message = None;
        manager.worker.take()
    };
    if let Some(worker) = worker {
        worker.stop.store(true, Ordering::Release);
        let _ = worker.thread.join();
    }
    let mut manager = manager().lock().expect("角色同步状态锁中毒");
    manager.status.state = "disabled";
    manager.status.message = None;
}

pub(crate) async fn command(
    host: &dyn HostContext,
    command: &str,
    _args: Value,
) -> Result<Value, Value> {
    let settings = host.settings().map_err(|message| json!(message))?;
    if settings.distribution != "cn" && command == "game_state_start" {
        return Err(json!("游戏内同步暂不支持当前区服."));
    }
    match command {
        "game_state_status" => serde_json::to_value(status()),
        "game_state_start" => {
            start(
                settings.install_root,
                oddity_platform::process::find_executable("dumpcap"),
            );
            serde_json::to_value(status())
        }
        "game_state_stop" => {
            {
                let mut manager = manager().lock().expect("角色同步状态锁中毒");
                manager.status.enabled = false;
                save(&manager.status, &cache_file());
            }
            tokio::task::spawn_blocking(stop)
                .await
                .map_err(|_| json!("无法停止游戏内同步."))?;
            serde_json::to_value(status())
        }
        _ => return Err(json!("无法识别的角色同步命令.")),
    }
    .map_err(|error| json!(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::block_on;
    use std::os::unix::fs::PermissionsExt;

    /// 角色同步状态是进程级单例, 测试之间必须串行以隔离采集线程.
    pub(super) static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn cached_snapshots_without_anchors_clear_activity_after_the_server_date_changes() {
        // 昨天 21:00 (北京时间) 的快照: 只有 updatedAt, 没有任何恢复锚点.
        let snapshot = 1_795_007_129u64;
        let mut stored: GameRecord = serde_json::from_value(json!({
            "roleId":"218023631055","roleName":"","serverName":"国服",
            "activity":100,"weeklyRemaining":2,"updatedAt":snapshot
        }))
        .unwrap();
        adopt_snapshot_anchor(&mut stored);
        let recovery = stored.recovery.as_ref().unwrap();
        assert_eq!(recovery.activity_at, Some(snapshot * 1000));
        assert_eq!(recovery.weekly_at, Some(snapshot * 1000));
        // 同一天内保留读数, 跨过 05:00 清零.
        assert_eq!(stored.projected(1_795_035_599_000).activity, Some(100));
        assert_eq!(stored.projected(1_795_035_601_000).activity, Some(0));
    }

    #[test]
    fn snapshot_anchor_never_overrides_an_observed_anchor() {
        let mut stored: GameRecord = serde_json::from_value(json!({
            "roleId":"218023631055","roleName":"","serverName":"国服",
            "activity":100,"weeklyRemaining":2,"updatedAt":1_795_007_129u64,
            "recovery":{"activityAt":1_795_020_000_000u64,"weeklyAt":1_795_021_000_000u64}
        }))
        .unwrap();
        adopt_snapshot_anchor(&mut stored);
        let recovery = stored.recovery.as_ref().unwrap();
        assert_eq!(recovery.activity_at, Some(1_795_020_000_000));
        assert_eq!(recovery.weekly_at, Some(1_795_021_000_000));
    }

    #[test]
    fn scene_snapshots_keep_known_values_only_for_the_same_role() {
        let _test = TEST_LOCK.lock().unwrap();
        let previous = status();
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("state.json");
        let complete: GameRecord = serde_json::from_value(json!({
            "roleId":"100000001", "roleName":"测试角色", "serverName":"国服", "updatedAt":1,
            "stamina":360, "staminaMax":360, "cityStamina":121, "cityStaminaMax":700,
            "weeklyRemaining":2, "staminaBackup":170, "cityStaminaBackup":0
        }))
        .unwrap();
        install_record(complete.clone(), &destination);
        let mut changed = complete;
        changed.stamina = Some(300);
        changed.weekly_remaining = Some(1);
        changed.stamina_backup = None;
        changed.city_stamina_backup = None;
        changed.role_name.clear();
        install_record(changed.clone(), &destination);
        let record = status().record.unwrap();
        assert_eq!(record.stamina, Some(300));
        assert_eq!(record.weekly_remaining, Some(1));
        assert_eq!(record.stamina_backup, Some(170));
        assert_eq!(record.city_stamina_backup, Some(0));
        assert_eq!(record.role_name, "测试角色");
        assert_eq!(
            std::fs::metadata(&destination)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        changed.role_id = "100000002".into();
        install_record(changed, &destination);
        assert_eq!(status().record.unwrap().stamina_backup, None);
        manager().lock().unwrap().status = previous;
    }

    #[test]
    fn commands_report_and_toggle_the_capture_worker() {
        let _test = TEST_LOCK.lock().unwrap();
        let host = crate::testing::TestHost::new(None);
        let status = block_on(command(&host, "game_state_status", json!({}))).unwrap();
        assert!(status["state"].is_string());

        let started = serde_json::to_value(start(
            Some("/nonexistent-install-root".into()),
            Some(PathBuf::from("/usr/bin/false")),
        ))
        .unwrap();
        assert_eq!(started["state"], json!("waiting"));
        let again =
            serde_json::to_value(start(None, Some(PathBuf::from("/usr/bin/false")))).unwrap();
        assert_eq!(again["state"], json!("waiting"));

        let stopped = block_on(command(&host, "game_state_stop", json!({}))).unwrap();
        assert_eq!(stopped["state"], json!("disabled"));
    }

    #[test]
    fn unidentified_values_are_saved_separately_from_the_previous_role() {
        let _test = TEST_LOCK.lock().unwrap();
        let previous = manager().lock().unwrap().status.clone();
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("state.json");
        let known: GameRecord = serde_json::from_value(json!({"roleId":"900000001","roleName":"测试角色","serverName":"国服","stamina":24,"staminaMax":360,"cityStamina":null,"cityStaminaMax":null,"activity":null,"weeklyRemaining":2,"updatedAt":1})).unwrap();
        manager().lock().unwrap().status.record = Some(known.clone());
        let mut observed = known.clone();
        observed.role_id.clear();
        observed.role_name.clear();
        observed.stamina = Some(129);
        observed.weekly_remaining = None;
        install_record(observed, &destination);
        let reading = status();
        assert_eq!(reading.state, "unidentified");
        assert_eq!(reading.record.unwrap().stamina, Some(24));
        assert_eq!(reading.observation.unwrap().stamina, Some(129));
        let saved: StoredRecord =
            serde_json::from_slice(&std::fs::read(destination).unwrap()).unwrap();
        assert_eq!(saved.record.unwrap().role_id, "900000001");
        assert!(saved.observation.unwrap().role_id.is_empty());
        let mut identified = known;
        identified.role_id = "900000002".into();
        identified.stamina = Some(80);
        install_record(identified, &directory.path().join("next.json"));
        let live = status();
        assert!(live.observation.is_none());
        assert_eq!(live.record.unwrap().role_id, "900000002");
        manager().lock().unwrap().status = previous;
    }

    #[test]
    fn missing_packets_and_unrecognized_traffic_keep_projecting_the_observed_snapshot() {
        let _test = TEST_LOCK.lock().unwrap();
        let previous = manager().lock().unwrap().status.clone();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        {
            let mut manager = manager().lock().unwrap();
            manager.status.enabled = true;
            manager.status.state = "capturing";
            manager.status.capture = CaptureStatus {
                pid: Some(1),
                ports: 3,
                packets: 7,
                records: 1,
                started_at: Some(now - 20_000),
                last_packet_at: Some(now - 20_000),
                ..Default::default()
            };
        }
        let quiet = status();
        assert_eq!(quiet.state, "stale");
        assert!(quiet.message.unwrap().contains("未收到游戏网络数据"));
        {
            let mut manager = manager().lock().unwrap();
            manager.status.state = "reading";
            manager.status.capture.last_packet_at = Some(now);
            manager.status.capture.records = 0;
        }
        let unrecognized = status();
        assert_eq!(unrecognized.capture.packets, 7);
        assert!(unrecognized
            .message
            .unwrap()
            .contains("尚未读取到新的游戏资源"));
        manager().lock().unwrap().status = previous;
    }

    #[test]
    fn stored_resources_are_projected_in_every_capture_state_without_changing_the_snapshot() {
        let _test = TEST_LOCK.lock().unwrap();
        let previous = manager().lock().unwrap().status.clone();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("state.json");
        let observed: GameRecord = serde_json::from_value(json!({
            "roleId":"900000001", "roleName":"测试角色", "serverName":"国服", "updatedAt":now / 1000,
            "stamina":100, "staminaMax":360, "staminaBackup":169,
            "cityStamina":190, "cityStaminaMax":700, "cityStaminaBackup":0, "weeklyRemaining":2,
            "recovery":{"staminaAt":now - 360000, "cityAt":now, "weeklyAt":now}
        })).unwrap();
        install_record(observed.clone(), &destination);
        for state in [
            "waiting", "disabled", "starting", "reading", "stale", "error",
        ] {
            change(state, None);
            let projected = status().record.unwrap();
            assert_eq!(projected.stamina, Some(101));
            assert_eq!(projected.city_stamina, Some(190));
            assert_eq!(projected.city_stamina_backup, Some(0));
            assert_eq!(projected.weekly_remaining, Some(2));
        }
        start(
            Some("/nonexistent-install-root".into()),
            Some(PathBuf::from("/usr/bin/false")),
        );
        let started = block_on(command(
            &crate::testing::TestHost::new(None),
            "game_state_start",
            json!({}),
        ))
        .unwrap();
        assert_eq!(started["record"]["stamina"], 101);
        assert_eq!(started["record"]["cityStaminaBackup"], 0);
        stop();
        let stored: StoredRecord =
            serde_json::from_slice(&std::fs::read(&destination).unwrap()).unwrap();
        assert_eq!(stored.record.as_ref().unwrap().stamina, Some(100));
        let mut unidentified = observed;
        unidentified.role_id.clear();
        install_record(unidentified, &destination);
        change("waiting", None);
        assert_eq!(status().observation.unwrap().stamina, Some(101));
        let stored: StoredRecord =
            serde_json::from_slice(&std::fs::read(destination).unwrap()).unwrap();
        assert_eq!(stored.observation.unwrap().city_stamina_backup, Some(0));
        manager().lock().unwrap().status = previous;
    }

    #[test]
    fn synchronization_preference_survives_worker_shutdown() {
        let _test = TEST_LOCK.lock().unwrap();
        let previous = status();
        start(
            Some("/nonexistent-install-root".into()),
            Some(PathBuf::from("/usr/bin/false")),
        );
        stop();
        assert!(status().enabled);
        let saved: StoredRecord =
            serde_json::from_slice(&std::fs::read(cache_file()).unwrap()).unwrap();
        assert!(saved.enabled);
        let host = crate::testing::TestHost::new(None);
        block_on(command(&host, "game_state_stop", json!({}))).unwrap();
        let saved: StoredRecord =
            serde_json::from_slice(&std::fs::read(cache_file()).unwrap()).unwrap();
        assert!(!saved.enabled);
        manager().lock().unwrap().status = previous;
    }

    #[test]
    fn missing_dumpcap_preserves_cache_and_keeps_a_retry_worker() {
        let _test = TEST_LOCK.lock().unwrap();
        let previous = status();
        let missing = start(Some("/nonexistent-install-root".into()), None);
        assert_eq!(missing.state, "error");
        assert_eq!(missing.message.as_deref(), Some(capture::MISSING_DUMPCAP));
        assert_eq!(
            serde_json::to_value(missing.record).unwrap(),
            serde_json::to_value(&previous.record).unwrap()
        );
        assert!(manager().lock().unwrap().worker.is_some());
        stop();
        manager().lock().unwrap().status = previous;
    }

    #[test]
    fn commands_reject_unsupported_distributions_and_unknown_requests() {
        let _test = TEST_LOCK.lock().unwrap();
        let error = block_on(command(
            &crate::testing::TestHost::with_settings("nte", "global"),
            "game_state_start",
            json!({}),
        ))
        .unwrap_err();
        assert_eq!(error, json!("游戏内同步暂不支持当前区服."));

        let error = block_on(command(
            &crate::testing::TestHost::failing_settings(),
            "game_state_status",
            json!({}),
        ))
        .unwrap_err();
        assert_eq!(error, json!("测试设置不可读"));

        let error = block_on(command(
            &crate::testing::TestHost::new(None),
            "unknown",
            json!({}),
        ))
        .unwrap_err();
        assert_eq!(error, json!("无法识别的角色同步命令."));
    }
}
