//! Electron 的私有 stdio 服务. 输入设备始终由进程主线程持有.
//!
//! 服务只依赖注入的装配注册表; 游戏包的名字只出现在 bin crate 的装配表里.

use crate::host::host_handle;
use crate::paths;
use crate::session::Registry;
use crate::settings as host_settings;
use crate::settings::Settings;
use oddity_runtime::Registry as Packages;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};

pub type ServiceHandle = Arc<Service>;

/// 交给主线程执行的输入任务.
pub type InputTask = Box<dyn FnOnce() + Send>;

#[cfg(any(test, feature = "testing"))]
type Listener = (String, Box<dyn Fn(Value) + Send + 'static>);

/// 宿主服务状态: 设置, 当前会话注册表与注入的装配注册表.
pub struct AppState {
    pub settings: Mutex<Settings>,
    pub game: Mutex<Registry>,
    pub packages: Arc<Packages>,
}

/// 构造服务所需的宿主状态; 装配注册表由调用方注入.
pub struct ServiceConfig {
    pub settings: Settings,
    pub data_dir: std::path::PathBuf,
    pub registry: Arc<Packages>,
    pub events: mpsc::SyncSender<Value>,
    pub input: mpsc::SyncSender<InputTask>,
}

pub struct Service {
    pub state: AppState,
    pub audio: crate::audio_commands::Audio,
    pub integrity: crate::integrity_commands::IntegrityMonitor,
    pub playtime: crate::playtime_commands::Playtime,
    pub performance: Arc<crate::performance_commands::Performance>,
    pub snapshots: crate::snapshot_commands::Snapshots,
    pub exports: crate::log_commands::LogExports,
    pub migration: Mutex<crate::migration_commands::MigrationManager>,
    pub migration_op: Mutex<()>,
    pub runtime_setup: Mutex<crate::runtime_setup_commands::Setup>,
    pub tasks: oddity_runtime::TaskRegistry,
    pub focused: AtomicBool,
    pub visible: AtomicBool,
    pub stopping: AtomicBool,
    output: mpsc::SyncSender<Value>,
    input: mpsc::SyncSender<InputTask>,
    #[cfg(any(test, feature = "testing"))]
    listeners: Mutex<Vec<Listener>>,
    #[cfg(test)]
    _directory: Option<crate::tests::TempDir>,
}
impl Service {
    /// 构造服务句柄; 调用方保留事件与输入通道的另一端.
    pub fn new(config: ServiceConfig) -> ServiceHandle {
        let ServiceConfig {
            settings,
            data_dir,
            registry,
            events,
            input,
        } = config;
        let audio = crate::audio_commands::Audio::default();
        audio.preferences(settings.current().audio.clone());
        Arc::new(Self {
            audio,
            state: AppState {
                settings: Mutex::new(settings),
                game: Mutex::new(Registry::default()),
                packages: registry,
            },
            integrity: crate::integrity_commands::IntegrityMonitor::new(&data_dir),
            playtime: crate::playtime_commands::Playtime::load(&data_dir),
            performance: Arc::new(crate::performance_commands::Performance::load(&data_dir)),
            snapshots: crate::snapshot_commands::Snapshots::load(&data_dir),
            exports: Default::default(),
            migration: Mutex::new(crate::migration_commands::MigrationManager::default()),
            migration_op: Mutex::new(()),
            runtime_setup: Default::default(),
            tasks: Default::default(),
            focused: AtomicBool::new(false),
            visible: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            output: events,
            input,
            #[cfg(any(test, feature = "testing"))]
            listeners: Mutex::new(Vec::new()),
            #[cfg(test)]
            _directory: None,
        })
    }
    pub fn input_task(&self, task: impl FnOnce() + Send + 'static) -> Result<(), String> {
        self.input.send(Box::new(task)).map_err(|e| e.to_string())
    }
    #[cfg(any(test, feature = "testing"))]
    pub fn subscribe(&self, event: &str, callback: impl Fn(Value) + Send + 'static) {
        self.listeners
            .lock()
            .unwrap()
            .push((event.into(), Box::new(callback)));
    }
}

impl oddity_runtime::EventSink for Service {
    fn deliver(
        &self,
        event: &'static oddity_contract::Event,
        payload: Value,
    ) -> Result<(), String> {
        #[cfg(any(test, feature = "testing"))]
        for (name, callback) in self.listeners.lock().unwrap().iter() {
            if name == event.id {
                callback(payload.clone());
            }
        }
        self.output
            .send(json!({"event": event.id, "payload": payload}))
            .map_err(|error| error.to_string())
    }
}
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("启动后台执行器失败")
    })
}
pub fn spawn_blocking<F, T>(job: F) -> tokio::task::JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    runtime().spawn_blocking(job)
}
pub fn spawn<F>(job: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    runtime().spawn(job)
}
/// 在服务的后台执行器上阻塞等待一个 future; 命令入口与测试共用.
pub fn block_on<F: std::future::Future>(job: F) -> F::Output {
    runtime().block_on(job)
}
#[cfg(test)]
pub fn test_service() -> ServiceHandle {
    test_service_with_output().0
}

/// 测试用宿主: 保留事件接收端, 用来断言 emit 的完整输出.
#[cfg(test)]
pub fn test_service_with_output() -> (ServiceHandle, mpsc::Receiver<Value>) {
    let (output, events) = mpsc::sync_channel(256);
    let (input, _) = mpsc::sync_channel(256);
    let directory = crate::tests::TempDir::new("service");
    let mut service = Service::new(ServiceConfig {
        settings: Settings::default(),
        data_dir: directory.path().to_path_buf(),
        registry: crate::testing::packages(),
        events: output,
        input,
    });
    // 刚构造的句柄独占一份引用, 可以把临时目录交给它, 随服务一起释放.
    Arc::get_mut(&mut service).expect("独占句柄")._directory = Some(directory);
    (service, events)
}

/// 常驻服务: 单实例, 恢复, 启动包的长驻域, stdin 循环与关闭序列.
///
/// 启动顺序里的参数判定, 装配校验与无头入口由 bin 负责.
pub fn run(registry: Arc<Packages>, compatible: bool) {
    paths::migrate_legacy_dirs();
    let paths = host_settings::UserPaths::discover();
    let _instance = crate::recovery::LauncherLock::launcher(&paths).unwrap_or_else(|error| {
        eprintln!("{error:#}");
        std::process::exit(1)
    });
    crate::migration_commands::recover(&registry, &paths.data_dir(), &paths.settings());
    let mut settings = host_settings::load_with_schemas(&registry);
    for environment in &settings.runtime_environments {
        if let Err(error) = crate::environment::recover_pending(&paths::data_dir(), environment) {
            eprintln!("运行环境恢复失败: {error:#}");
        }
    }
    if compatible {
        settings.hero_media = "photo".into();
        settings.reduce_motion = true;
        settings.minimize_to_tray_on_close = false;
    }
    let (output, events) = mpsc::sync_channel(256);
    std::thread::spawn(move || {
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        for message in events {
            if serde_json::to_writer(&mut out, &message).is_err()
                || out.write_all(b"\n").is_err()
                || out.flush().is_err()
            {
                break;
            }
        }
    });
    let (input, jobs) = mpsc::sync_channel(256);
    let app = Service::new(ServiceConfig {
        settings,
        data_dir: paths::data_dir(),
        registry,
        events: output,
        input,
    });
    // 每个包的长驻域自己决定要不要启动调度, 要不要在打开启动器时触发领域动作.
    let host = host_handle(app.clone());
    let auto_stop = Arc::new(AtomicBool::new(false));
    for runtime in app.state.packages.runtimes() {
        runtime.start(Arc::clone(&host), Arc::clone(&auto_stop));
    }
    if !compatible {
        for runtime in app.state.packages.runtimes() {
            runtime.on_launch(host.as_ref());
        }
    }
    crate::integrity_commands::start(app.clone());
    let reader = app.clone();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            let request = serde_json::from_str::<crate::protocol::Request>(&line);
            let app = reader.clone();
            match request {
                Ok(request) => {
                    if request.command == "host_state" {
                        let result = runtime().block_on(crate::protocol::dispatch(
                            app.clone(),
                            request.command,
                            request.game,
                            request.args,
                        ));
                        let reply = match result {
                            Ok(value) => json!({"id":request.id,"result":value}),
                            Err(error) => json!({"id":request.id,"error":error}),
                        };
                        let _ = app.output.send(reply);
                        continue;
                    }
                    spawn(async move {
                        let result = crate::protocol::dispatch(
                            app.clone(),
                            request.command,
                            request.game,
                            request.args,
                        )
                        .await;
                        let reply = match result {
                            Ok(value) => json!({"id":request.id,"result":value}),
                            Err(error) => json!({"id":request.id,"error":error}),
                        };
                        let _ = app.output.send(reply);
                    });
                }
                Err(error) => {
                    eprintln!("无效宿主请求: {error}");
                    continue;
                }
            }
        }
        reader.stopping.store(true, Ordering::Release);
        let _ = reader.input_task(|| {});
    });
    while !app.stopping.load(Ordering::Acquire) {
        match jobs.recv_timeout(std::time::Duration::from_millis(20)) {
            Ok(job) => job(),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => break,
        }
    }
    crate::gamepad_commands::stop();
    // 先停自动调度, 再取消当前下载任务, 避免调度线程在关闭期间重新启动任务.
    for runtime in app.state.packages.runtimes() {
        runtime.stop();
    }
    app.tasks.cancel_all();
    if let Ok(game) = app.state.game.lock() {
        if let Some(cancel) = &game.launch_cancel {
            cancel.store(true, Ordering::Release);
        }
    };
}
