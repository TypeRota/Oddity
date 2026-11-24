//! 宿主会话服务: 界面进程的权威状态来源.
//!
//! 服务不依附界面进程: 界面只是连上来的客户端, 退出后服务继续持有运行中的会话,
//! 重新打开时接回同一份状态. 只有没有客户端, 没有运行中的会话, 也没有在途任务
//! 时服务才退出.

use crate::host::HostHandle;
use crate::ipc::Clients;
use crate::paths;
use crate::session::Registry;
use crate::settings as host_settings;
use crate::settings::Settings;
use oddity_runtime::Registry as Packages;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub type ServiceHandle = Arc<Service>;

/// 交给主线程执行的输入任务.
pub type InputTask = Box<dyn FnOnce() + Send>;

/// 没有客户端且没有会话时, 服务等待多久再退出.
///
/// 留一段宽限时间, 界面重启期间不会来回创建服务.
const IDLE_EXIT: Duration = Duration::from_millis(1500);

/// 启动后等第一个界面接入的时间.
///
/// 界面拉起服务后要等套接字就绪再连接, 两者之间存在窗口; 负载高时这个窗口
/// 会明显变长, 服务不能在这之前把自己判成空闲. 一旦有过连接, 就只按空闲
/// 宽限判断, 不必再等这么久.
const STARTUP_GRACE: Duration = Duration::from_secs(10);

#[cfg(any(test, feature = "testing"))]
type Listener = (String, Box<dyn Fn(Value) + Send + 'static>);

/// 事件出口: 测试用进程内通道, 服务模式下广播到已连接的界面.
#[derive(Clone)]
pub enum EventBus {
    Channel(mpsc::SyncSender<Value>),
    Clients(Arc<Clients>),
}

impl EventBus {
    /// 发送一条事件; 没有接收方时返回错误, 调用方按现有约定忽略.
    fn send(&self, value: Value) -> Result<(), String> {
        match self {
            Self::Channel(sender) => sender.send(value).map_err(|error| error.to_string()),
            Self::Clients(clients) => clients.broadcast(value),
        }
    }
}

/// 宿主服务状态: 设置, 当前会话注册表与注入的装配注册表.
pub struct AppState {
    pub settings: Mutex<Settings>,
    pub game: Mutex<Registry>,
    pub packages: Arc<Packages>,
}

/// 构造服务所需的宿主状态; 装配注册表与事件出口由调用方注入.
pub struct ServiceConfig {
    pub settings: Settings,
    pub data_dir: std::path::PathBuf,
    pub registry: Arc<Packages>,
    pub events: EventBus,
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
    events: EventBus,
    input: mpsc::SyncSender<InputTask>,
    #[cfg(any(test, feature = "testing"))]
    listeners: Mutex<Vec<Listener>>,
    #[cfg(test)]
    _directory: Option<crate::tests::TempDir>,
}
impl Service {
    /// 构造服务句柄; 调用方保留输入通道的另一端.
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
            events,
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
    /// 当前连接的界面数量; 只有服务模式下才可能大于零.
    pub fn connections(&self) -> usize {
        match &self.events {
            EventBus::Channel(_) => 1,
            EventBus::Clients(clients) => clients.count(),
        }
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
        self.events
            .send(json!({"event": event.id, "payload": payload}))
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
    test_service_with_registry(crate::testing::packages())
}

/// 测试用宿主, 由调用方给定注册表.
///
/// 需要宿主能力参与求值的用例用它注入带宿主能力的装配.
#[cfg(test)]
pub fn test_service_with_registry(
    registry: Arc<Packages>,
) -> (ServiceHandle, mpsc::Receiver<Value>) {
    let (output, events) = mpsc::sync_channel(256);
    let (input, _) = mpsc::sync_channel(256);
    let directory = crate::tests::TempDir::new("service");
    let mut service = Service::new(ServiceConfig {
        settings: Settings::default(),
        data_dir: directory.path().to_path_buf(),
        registry,
        events: EventBus::Channel(output),
        input,
    });
    // 刚构造的句柄独占一份引用, 可以把临时目录交给服务, 随服务一起释放.
    Arc::get_mut(&mut service).expect("独占句柄")._directory = Some(directory);
    (service, events)
}

/// 会话服务: 单实例, 恢复, 启动包的长驻域, 套接字循环与关闭序列.
///
/// 启动顺序里的参数判定, 装配校验与无头入口由 bin 负责.
pub fn run(registry: Arc<Packages>) {
    paths::migrate_legacy_dirs();
    let paths = host_settings::UserPaths::discover();
    let _instance = crate::recovery::LauncherLock::launcher(&paths).unwrap_or_else(|error| {
        eprintln!("{error:#}");
        std::process::exit(1)
    });
    crate::migration_commands::recover(&registry, &paths.data_dir(), &paths.settings());
    let settings = host_settings::load_with_schemas(&registry);
    for environment in &settings.runtime_environments {
        if let Err(error) = crate::environment::recover_pending(&paths::data_dir(), environment) {
            eprintln!("运行环境恢复失败: {error:#}");
        }
    }
    let data_dir = paths.data_dir();
    let entry = crate::ipc::entry_path(&data_dir);
    let listener = crate::ipc::listen(&data_dir, &entry).unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(1)
    });
    let clients = Arc::new(Clients::default());
    let (input, jobs) = mpsc::sync_channel(256);
    let app = Service::new(ServiceConfig {
        settings,
        data_dir: paths.data_dir(),
        registry,
        events: EventBus::Clients(Arc::clone(&clients)),
        input,
    });
    // 每个包的长驻域自己决定要不要启动调度, 要不要在打开启动器时触发领域动作.
    let runtimes = app
        .state
        .packages
        .runtimes()
        .map(|(game, runtime)| {
            let host: Arc<dyn oddity_runtime::HostContext> = Arc::new(HostHandle::for_package(
                app.clone(),
                crate::identity::GameId::from(game),
            ));
            runtime.start(host.clone(), Arc::new(AtomicBool::new(false)));
            (runtime, host)
        })
        .collect::<Vec<_>>();
    for (runtime, host) in &runtimes {
        runtime.on_launch(host.as_ref());
    }
    crate::integrity_commands::start(app.clone());
    accept_loop(app.clone(), listener, clients);
    // 收到 SIGTERM 也要走同一条收尾路径, 否则套接字文件会留在数据目录里.
    crate::ipc::exit_on_termination(Arc::clone(&app));
    // 主线程只跑输入任务, 同时判断服务该不该继续存活.
    let started = Instant::now();
    let mut idle_since = started;
    // 界面接入过之后, 退出只按空闲宽限判断.
    let mut seen_client = false;
    while !app.stopping.load(Ordering::Acquire) {
        match jobs.recv_timeout(Duration::from_millis(20)) {
            Ok(job) => job(),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => break,
        }
        if service_busy(&app, app.connections()) {
            seen_client |= app.connections() > 0;
            idle_since = Instant::now();
        } else if (seen_client || started.elapsed() >= STARTUP_GRACE)
            && idle_since.elapsed() >= IDLE_EXIT
        {
            break;
        }
    }
    crate::ipc::release(&data_dir, &entry);
    crate::gamepad_commands::stop();
    // 先停自动调度, 再取消当前下载任务, 避免调度线程在关闭期间重新启动任务.
    for (runtime, _) in runtimes {
        runtime.stop();
    }
    app.tasks.cancel_all();
    if let Ok(game) = app.state.game.lock() {
        if let Some(cancel) = &game.launch_cancel {
            cancel.store(true, Ordering::Release);
        }
    };
}

/// 服务是否还有必须留在内存里的工作: 已连接的界面或运行中的会话.
pub(crate) fn service_busy(app: &ServiceHandle, connections: usize) -> bool {
    if connections > 0 {
        return true;
    }
    if app
        .state
        .game
        .lock()
        .map(|game| {
            game.starting || game.session.is_some() || game.environment_busy || game.migration_busy
        })
        .unwrap_or(true)
    {
        return true;
    }
    if app.state.packages.busy() {
        return true;
    }
    if !app.tasks.is_empty() {
        return true;
    }
    false
}

/// 接受连接; 每条连接一个线程, 服务到对端关闭.
fn accept_loop(
    app: ServiceHandle,
    listener: std::os::unix::net::UnixListener,
    clients: Arc<Clients>,
) {
    std::thread::Builder::new()
        .name("oddity-ipc-accept".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let app = app.clone();
                let clients = Arc::clone(&clients);
                let _ = std::thread::Builder::new()
                    .name("oddity-ipc-client".into())
                    .spawn(move || {
                        if let Err(error) = crate::ipc::serve(app, clients, stream) {
                            eprintln!("会话连接结束: {error}");
                        }
                    });
            }
        })
        .expect("启动会话监听线程失败");
}
