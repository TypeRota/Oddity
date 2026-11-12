//! 宿主句柄: 游戏包命令组通过它接触宿主能力.
//!
//! 事件经 Service 的 EventSink 出口发送; 设置与活动状态只读; 后台任务经
//! 宿主执行器登记, 关闭时由宿主统一取消.

use std::path::PathBuf;
use std::sync::Arc;

use oddity_runtime::{
    ActivitySnapshot, CapabilityContext, HostContext, MeteredPolicy, Registry, SettingsSnapshot,
    Task, TaskJob, UpdatePreferences, UpdateWindow,
};
use serde_json::Value;

use crate::service::{Service, ServiceHandle};
use crate::settings::Settings;
use oddity_runtime::EventSink;

/// 宿主句柄: 包命令组拿到的唯一宿主入口.
pub struct HostHandle(pub ServiceHandle);

impl HostContext for HostHandle {
    fn settings(&self) -> Result<SettingsSnapshot, String> {
        let settings = self
            .0
            .state
            .settings
            .lock()
            .map_err(|error| error.to_string())?;
        let current = settings.current();
        let updates = &current.updates;
        let packages = &self.0.state.packages;
        Ok(SettingsSnapshot {
            game: settings.active_game.as_str().to_string(),
            distribution: crate::channel::effective_id(
                packages,
                settings.active_game.as_str(),
                current.distribution.as_ref(),
            ),
            language: settings.language.clone(),
            reduce_motion: settings.reduce_motion,
            theme: settings.theme.clone(),
            install_root: crate::install::configured_install_root(packages, &settings)
                .map(expanded_root),
            preload_root: current.preload_root.as_deref().map(expanded_root),
            update: UpdatePreferences {
                auto_download: updates.auto_download,
                metered: match updates.metered {
                    crate::updates::MeteredPolicy::Ask => MeteredPolicy::Ask,
                    crate::updates::MeteredPolicy::Pause => MeteredPolicy::Pause,
                    crate::updates::MeteredPolicy::Allow => MeteredPolicy::Allow,
                },
                limit_bytes_per_second: updates.limit_bytes_per_second,
                window: updates.window.as_ref().map(|window| UpdateWindow {
                    start: window.start.clone(),
                    end: window.end.clone(),
                }),
            },
        })
    }

    fn activity(&self) -> Result<ActivitySnapshot, String> {
        activity_snapshot(&self.0)
    }

    fn capability_context(&self) -> Result<CapabilityContext, String> {
        let settings = self
            .0
            .state
            .settings
            .lock()
            .map_err(|error| error.to_string())?
            .clone();
        let activity = self.activity()?;
        Ok(build_capability_context(
            &self.0.state.packages,
            &settings,
            &activity,
        ))
    }

    fn official_launcher_root(&self) -> Option<PathBuf> {
        // 探测会读取文件系统, 先拷出设置再释放锁.
        let settings = self.0.state.settings.lock().ok()?.clone();
        crate::install::launcher_root(&self.0.state.packages, &settings)
    }

    fn emit(&self, id: &'static str, payload: Value) -> Result<(), String> {
        let event =
            oddity_contract::event(id).ok_or_else(|| format!("事件未在契约里声明: {id}"))?;
        self.0.deliver(event, payload)
    }

    fn submit(&self, job: TaskJob) -> Task {
        // 进度与宿主其它事件走同一个出口.
        let service = self.0.clone();
        let task = Task::new(Arc::new(move |payload| {
            service.emit("task-progress", payload)
        }));
        // 登记后返回原句柄, 宿主关闭时按注册表统一取消.
        let task = self.0.tasks.track(task);
        // 结果经句柄的完成状态取回, 执行器的 JoinHandle 丢弃后任务继续运行.
        drop(crate::service::spawn_blocking(task.runner(job)));
        task
    }

    fn cancel_task(&self, id: &str) -> bool {
        self.0.tasks.cancel(id)
    }
}

/// 构造可交给游戏包的宿主能力句柄.
///
/// 下载域的后台任务在命令返回后仍需发事件, 因此需要一份 'static 句柄.
pub fn host_handle(app: ServiceHandle) -> Arc<dyn HostContext> {
    Arc::new(HostHandle(app))
}

/// 读取宿主当前的活动状态.
pub(crate) fn activity_snapshot(app: &Service) -> Result<ActivitySnapshot, String> {
    let game = app.state.game.lock().map_err(|error| error.to_string())?;
    Ok(ActivitySnapshot {
        game_starting: game.starting,
        game_running: game
            .session
            .as_ref()
            .is_some_and(crate::session::session_alive),
        game_closing: game.session.as_ref().is_some_and(|session| session.closing),
        environment_busy: game.environment_busy,
        migration_busy: game.migration_busy,
        migration_repair_pending: crate::migration_commands::repair_pending(),
    })
}

/// 在命令边界构造能力求值上下文.
pub(crate) fn build_capability_context(
    packages: &Registry,
    settings: &Settings,
    activity: &ActivitySnapshot,
) -> CapabilityContext {
    let current = settings.current();
    CapabilityContext {
        game: settings.active_game.as_str().to_string(),
        distribution: crate::channel::effective_id(
            packages,
            settings.active_game.as_str(),
            current.distribution.as_ref(),
        ),
        account_id: None,
        session_running: activity.game_starting || activity.game_running || activity.game_closing,
        launcher_present: crate::install::launcher_root(packages, settings).is_some(),
    }
}

/// 设置里的 `~` 在映射时展开; 相对路径保持原样, 由游戏包按当前工作目录解释.
fn expanded_root(raw: &str) -> String {
    crate::paths::expand_path(raw).display().to_string()
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::ChannelId;
    use crate::service::test_service;
    use oddity_runtime::{TaskOutcome, TaskProgress};
    use serde_json::{json, Value};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[test]
    fn host_handle_forwards_to_the_service_sink() {
        // 输出端要保持存活, 否则事件发送会失败.
        let (app, _events) = crate::service::test_service_with_output();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
        let expected = seen.clone();
        app.subscribe("host-show", move |payload| {
            *expected.lock().unwrap() = Some(payload);
        });
        HostHandle(app)
            .emit("host-show", Value::Null)
            .expect("已声明的事件应能发送");
        assert_eq!(*seen.lock().unwrap(), Some(Value::Null));
    }

    #[test]
    fn host_handle_exposes_a_read_only_settings_snapshot() {
        let (app, _events) = crate::service::test_service_with_output();
        {
            let mut settings = app.state.settings.lock().unwrap();
            settings.active_game = crate::identity::GameId::from("nte");
            settings.current_mut().distribution = Some(ChannelId::new("global"));
            settings.language = "ja-JP".into();
            settings.reduce_motion = true;
            settings.theme = "dark".into();
        }

        let snapshot = HostHandle(app.clone()).settings().expect("读取设置快照");
        assert_eq!(snapshot.game, "nte");
        assert_eq!(snapshot.distribution, "global");
        assert_eq!(snapshot.language, "ja-JP");
        assert!(snapshot.reduce_motion);
        assert_eq!(snapshot.theme, "dark");
        assert_eq!(snapshot.install_root, None);
        assert_eq!(snapshot.preload_root, None);
        assert_eq!(snapshot.update, UpdatePreferences::default());

        // 快照是拷贝; 后续设置变化不会回写已经取出的快照.
        app.state.settings.lock().unwrap().language = "zh-CN".into();
        assert_eq!(snapshot.language, "ja-JP");
    }

    #[test]
    fn host_handle_reports_settings_read_failures() {
        let (app, _events) = crate::service::test_service_with_output();
        let host = HostHandle(app.clone());
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = app.state.settings.lock().unwrap();
            panic!("poison settings");
        }));
        assert!(host.settings().is_err());
    }

    #[test]
    fn host_handle_emits_declared_events() {
        let (app, events) = crate::service::test_service_with_output();
        let host = HostHandle(app);
        host.emit("game-download-state", json!({"phase": "ready"}))
            .expect("事件应写入宿主输出");

        let message = events
            .recv_timeout(Duration::from_secs(2))
            .expect("宿主应转发事件");
        assert_eq!(
            message,
            json!({"event": "game-download-state", "payload": {"phase": "ready"}})
        );
    }

    #[test]
    fn host_handle_runs_tasks_with_progress_and_results() {
        let (app, events) = crate::service::test_service_with_output();
        let host = HostHandle(app);

        let task = host.submit(Box::new(|context| {
            context
                .progress(TaskProgress::new(1, 2))
                .expect("进度应写入宿主输出");
            context
                .progress(TaskProgress::new(2, 2))
                .expect("进度应写入宿主输出");
            TaskOutcome::Done(json!({"accounts": 2}))
        }));
        assert_eq!(
            crate::service::block_on(task.wait()),
            TaskOutcome::Done(json!({"accounts": 2}))
        );

        for done in [1, 2] {
            let message = events
                .recv_timeout(Duration::from_secs(2))
                .expect("宿主应转发进度事件");
            assert_eq!(message["event"], json!("task-progress"));
            assert_eq!(message["payload"]["done"], json!(done));
            assert_eq!(message["payload"]["total"], json!(2));
        }
    }

    #[test]
    fn host_handle_cancels_running_tasks() {
        let host = HostHandle(crate::service::test_service());
        let (started_tx, started_rx) = mpsc::channel();
        let task = host.submit(Box::new(move |context| {
            started_tx.send(()).unwrap();
            while !context.is_cancelled() {
                std::thread::yield_now();
            }
            TaskOutcome::Cancelled
        }));
        started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("任务应已启动");
        assert!(!task.is_cancelled());
        task.cancel();
        assert!(task.is_cancelled());
        assert_eq!(
            crate::service::block_on(task.wait()),
            TaskOutcome::Cancelled
        );
    }

    #[test]
    fn host_handle_maps_install_roots_and_update_preferences() {
        let (app, _events) = crate::service::test_service_with_output();
        {
            let mut settings = app.state.settings.lock().unwrap();
            let current = settings.current_mut();
            current
                .install_roots
                .insert("cn".into(), "/games/nte".into());
            current
                .install_roots
                .insert("global".into(), "/games/global".into());
            current.preload_root = Some("/games/preload".into());
            current.updates = crate::updates::Options {
                auto_download: true,
                metered: crate::updates::MeteredPolicy::Allow,
                limit_bytes_per_second: 8192,
                window: Some(crate::updates::Window {
                    start: "01:00".into(),
                    end: "05:00".into(),
                }),
            };
        }

        let snapshot = HostHandle(app).settings().expect("读取设置快照");
        assert_eq!(snapshot.install_root.as_deref(), Some("/games/nte"));
        assert_eq!(snapshot.preload_root.as_deref(), Some("/games/preload"));
        assert_eq!(
            snapshot.update,
            UpdatePreferences {
                auto_download: true,
                metered: MeteredPolicy::Allow,
                limit_bytes_per_second: 8192,
                window: Some(UpdateWindow {
                    start: "01:00".into(),
                    end: "05:00".into(),
                }),
            }
        );
    }

    /// 隔离影响安装目录与数据目录探测的环境变量.
    fn isolated_env(tag: &str) -> (crate::tests::TempDir, crate::tests::EnvGuard) {
        let directory = crate::tests::TempDir::new(tag);
        let mut env = crate::tests::EnvGuard::new();
        env.set("HOME", directory.join("home"));
        env.set("XDG_CONFIG_HOME", directory.join("config"));
        env.set("XDG_DATA_HOME", directory.join("data"));
        std::fs::create_dir_all(directory.join("home")).expect("create fake home");
        (directory, env)
    }

    /// 一个已经结束启动流程的会话; 是否存活与正在关闭由参数决定.
    fn session(closing: bool, alive: bool) -> crate::session::Session {
        crate::session::Session {
            performance: std::sync::Arc::new(std::sync::Mutex::new(None)),
            id: 1,
            pid: 0,
            pgid: 0,
            exe: String::new(),
            started_at: 0,
            log: String::new(),
            mode: "background".into(),
            distribution: ChannelId::new("cn"),
            game: crate::identity::GameId::from("nte"),
            closing,
            exit_notified: false,
            game_alive: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(alive)),
            monitor_done: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            finalized: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            _install_lock: None,
            _prefix_lock: None,
            game_mode: None,
            snapshot: None,
            snapshot_success_after_ms: 0,
            launcher_root: None,
            monitor_state: Arc::new(Mutex::new(crate::session::monitor::MonitorState::default())),
            control: crate::session::ControlSlot::default(),
        }
    }

    #[test]
    fn host_handle_expands_home_in_install_roots() {
        let (app, _events) = crate::service::test_service_with_output();
        let (directory, _env) = isolated_env("assembly-expand");
        {
            let mut settings = app.state.settings.lock().unwrap();
            let current = settings.current_mut();
            current
                .install_roots
                .insert("cn".into(), "~/games/nte".into());
            current
                .install_roots
                .insert("global".into(), "../global".into());
            current.preload_root = Some("~".into());
        }

        let snapshot = HostHandle(app).settings().expect("读取设置快照");
        let home = directory.join("home");
        assert_eq!(
            snapshot.install_root,
            Some(home.join("games/nte").display().to_string())
        );
        assert_eq!(snapshot.preload_root, Some(home.display().to_string()));
    }

    #[test]
    fn host_handle_reports_an_idle_activity() {
        let (app, _events) = crate::service::test_service_with_output();
        let (_directory, _env) = isolated_env("assembly-activity-idle");

        assert_eq!(
            HostHandle(app).activity().expect("读取活动状态"),
            ActivitySnapshot::default()
        );
    }

    #[test]
    fn host_handle_reports_host_tasks_as_busy() {
        let (app, _events) = crate::service::test_service_with_output();
        let (_directory, _env) = isolated_env("assembly-activity-busy");
        {
            let mut game = app.state.game.lock().unwrap();
            game.starting = true;
            game.environment_busy = true;
            game.migration_busy = true;
        }

        let activity = HostHandle(app).activity().expect("读取活动状态");
        assert!(activity.game_starting);
        assert!(!activity.game_running);
        assert!(!activity.game_closing);
        assert!(activity.environment_busy);
        assert!(activity.migration_busy);
        assert!(!activity.migration_repair_pending);
    }

    #[test]
    fn host_handle_reports_running_and_closing_sessions() {
        let (app, _events) = crate::service::test_service_with_output();
        let (_directory, _env) = isolated_env("assembly-activity-session");

        app.state.game.lock().unwrap().session = Some(session(false, true));
        let running = HostHandle(app.clone()).activity().expect("读取活动状态");
        assert!(running.game_running);
        assert!(!running.game_closing);

        app.state.game.lock().unwrap().session = Some(session(true, false));
        let closing = HostHandle(app).activity().expect("读取活动状态");
        assert!(!closing.game_running);
        assert!(closing.game_closing);
    }

    #[test]
    fn host_handle_reports_a_pending_migration_repair() {
        let (app, _events) = crate::service::test_service_with_output();
        let (_directory, _env) = isolated_env("assembly-activity-repair");
        let migrations = crate::paths::data_dir().join("migrations");
        std::fs::create_dir_all(&migrations).expect("create migrations dir");
        std::fs::write(
            migrations.join("current.json"),
            r#"{"id":"m1","phase":"committed","source":"/old","target":"/new","staging":null,
                "distribution":"cn","mode":"move","nextSettings":{},"createdAt":0}"#,
        )
        .expect("write migration journal");

        assert!(
            HostHandle(app)
                .activity()
                .expect("读取活动状态")
                .migration_repair_pending
        );
    }

    #[test]
    fn host_handle_reports_activity_read_failures() {
        let (app, _events) = crate::service::test_service_with_output();
        let host = HostHandle(app.clone());
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = app.state.game.lock().unwrap();
            panic!("poison game state");
        }));
        assert!(host.activity().is_err());
    }

    #[test]
    fn host_handle_maps_every_metered_policy() {
        let (app, _events) = crate::service::test_service_with_output();
        for (source, expected) in [
            (crate::updates::MeteredPolicy::Ask, MeteredPolicy::Ask),
            (crate::updates::MeteredPolicy::Pause, MeteredPolicy::Pause),
            (crate::updates::MeteredPolicy::Allow, MeteredPolicy::Allow),
        ] {
            app.state
                .settings
                .lock()
                .unwrap()
                .current_mut()
                .updates
                .metered = source;
            let snapshot = HostHandle(app.clone()).settings().expect("读取设置快照");
            assert_eq!(snapshot.update.metered, expected);
        }
    }

    #[test]
    fn host_handle_cancels_registered_tasks_by_id() {
        let (app, _events) = crate::service::test_service_with_output();
        let host = HostHandle(app.clone());
        let (started_tx, started_rx) = mpsc::channel();
        let task = host.submit(Box::new(move |context| {
            started_tx.send(()).unwrap();
            while !context.is_cancelled() {
                std::thread::yield_now();
            }
            TaskOutcome::Cancelled
        }));
        started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("任务应已启动");

        let id = task.id().to_string();
        assert!(host.cancel_task(&id));
        assert_eq!(
            crate::service::block_on(task.wait()),
            TaskOutcome::Cancelled
        );
        assert!(!host.cancel_task(&id));
        assert!(!host.cancel_task("task-unknown"));
    }

    #[test]
    fn host_shutdown_cancels_every_registered_task() {
        let (app, _events) = crate::service::test_service_with_output();
        let host = HostHandle(app.clone());
        let mut tasks = Vec::new();
        for _ in 0..2 {
            let (started_tx, started_rx) = mpsc::channel();
            let task = host.submit(Box::new(move |context| {
                started_tx.send(()).unwrap();
                while !context.is_cancelled() {
                    std::thread::yield_now();
                }
                TaskOutcome::Cancelled
            }));
            started_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("任务应已启动");
            tasks.push(task);
        }
        assert_eq!(app.tasks.len(), 2);

        // service::run 收尾与下载、启动取消并列, 统一取消登记任务.
        app.tasks.cancel_all();
        for task in tasks {
            assert_eq!(
                crate::service::block_on(task.wait()),
                TaskOutcome::Cancelled
            );
        }
        assert!(app.tasks.is_empty());
    }

    #[test]
    fn service_holds_the_injected_packages() {
        // 服务构造时注入装配表; 之后所有探测与命令组都读同一份注册表.
        let app = test_service();
        assert_eq!(app.state.packages.packages().len(), 1);
        assert!(app.state.packages.runtime("nte").is_some());
    }
}
