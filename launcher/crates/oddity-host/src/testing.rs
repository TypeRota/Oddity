//! 测试用的临时目录与本地 HTTP 服务。
//!
//! 迁移到游戏包的测试带走了部分夹具, 这里保留其余用例需要的最小集合.
//!
//! 网络相关的代码（Version.ini、config.xml、ResList.bin.zip、资源下载）需要
//! 真实走一遍请求和响应，这里用一个只监听回环地址的最小 HTTP 服务代替外网，
//! 测试因此完全离线，也不依赖真实 CDN。

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::Duration;

use oddity_runtime::{
    ChannelDescriptor, GamePackage, HostContext, InstallLocator, InstallQuery, LauncherInfo,
    PackageDescriptor, PackageGate, PackageRuntime, Registry,
};

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// 进程环境锁；宿主单元测试的夹具都从这里取同一把锁.
pub(crate) fn env_lock() -> MutexGuard<'static, ()> {
    // 之前的测试 panic 过也不影响后续用例
    ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner())
}

/// 独占进程环境变量，并在离开作用域时恢复原值。
///
/// 互斥锁只保证并行测试不互相踩，恢复原值才能保证测试结束后进程环境不变，
/// 也不会清除使用者原有的环境变量。
pub struct EnvGuard {
    saved: Vec<(&'static str, Option<OsString>)>,
    _lock: MutexGuard<'static, ()>,
}

impl EnvGuard {
    pub fn new() -> Self {
        Self {
            saved: Vec::new(),
            _lock: env_lock(),
        }
    }

    pub fn set(&mut self, key: &'static str, value: impl AsRef<OsStr>) {
        self.remember(key);
        std::env::set_var(key, value);
    }

    /// 同一个变量只记第一次的原始值，之后覆盖都还原到它。
    fn remember(&mut self, key: &'static str) {
        if !self.saved.iter().any(|(saved, _)| *saved == key) {
            self.saved.push((key, std::env::var_os(key)));
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..) {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

/// 析构时自动删除的临时目录。
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("nte-test-{tag}-{}-{unique}", std::process::id()));
        fs::create_dir_all(&path).expect("创建临时目录");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn join(&self, rel: impl AsRef<Path>) -> PathBuf {
        self.path.join(rel)
    }

    pub fn write(&self, rel: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> PathBuf {
        let full = self.path.join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).expect("创建父目录");
        }
        fs::write(&full, contents).expect("写入文件");
        full
    }

    pub fn read(&self, rel: impl AsRef<Path>) -> Vec<u8> {
        fs::read(self.path.join(rel)).expect("读取文件")
    }

    pub fn read_text(&self, rel: impl AsRef<Path>) -> String {
        fs::read_to_string(self.path.join(rel)).expect("读取文本文件")
    }

    pub fn is_file(&self, rel: impl AsRef<Path>) -> bool {
        self.path.join(rel).is_file()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct ResponseGate {
    entered: std::sync::mpsc::Sender<()>,
    resume: std::sync::mpsc::Receiver<()>,
}

#[derive(Clone)]
struct Route {
    body: Vec<u8>,
    gate: Option<Arc<Mutex<ResponseGate>>>,
}

#[derive(Default)]
struct ServerState {
    routes: HashMap<String, Route>,
    targets: Vec<String>,
    ranges: Vec<String>,
}

/// 只服务已注册路径的回环 HTTP 服务。
pub struct HttpServer {
    addr: SocketAddr,
    state: Arc<Mutex<ServerState>>,
    stop: Arc<AtomicBool>,
}

impl HttpServer {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定测试服务端口");
        let addr = listener.local_addr().expect("读取监听地址");
        listener.set_nonblocking(true).expect("设置非阻塞");
        let state = Arc::new(Mutex::new(ServerState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_state = Arc::clone(&state);
        let worker_stop = Arc::clone(&stop);
        thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let state = Arc::clone(&worker_state);
                        thread::spawn(move || {
                            let _ = serve(stream, &state);
                        });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });
        Self { addr, state, stop }
    }

    pub fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base(), path)
    }

    /// 注册一个 200 响应，按 Range 头返回部分内容或完整内容。
    pub fn route(&self, path: impl AsRef<str>, body: impl AsRef<[u8]>) {
        let mut state = self.state.lock().expect("锁定服务状态");
        state.routes.insert(
            path.as_ref().to_string(),
            Route {
                body: body.as_ref().to_vec(),
                gate: None,
            },
        );
    }

    /// 暂停指定响应, 让并发测试控制完成顺序; 超时防止失败测试挂起。
    pub fn route_blocked(
        &self,
        path: &str,
        body: &str,
    ) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (entered, started) = std::sync::mpsc::channel();
        let (release, resume) = std::sync::mpsc::channel();
        self.state.lock().unwrap().routes.insert(
            path.into(),
            Route {
                body: body.as_bytes().to_vec(),
                gate: Some(Arc::new(Mutex::new(ResponseGate { entered, resume }))),
            },
        );
        (started, release)
    }

    /// 记录到的请求目标（含查询串），按到达顺序排列。
    pub fn targets(&self) -> Vec<String> {
        self.state.lock().expect("锁定服务状态").targets.clone()
    }

    /// 记录到的 Range 请求头，与 `targets` 一一对应；没有 Range 时为空串。
    pub fn ranges(&self) -> Vec<String> {
        self.state.lock().expect("锁定服务状态").ranges.clone()
    }

    /// 某个路径被请求的次数。
    pub fn hits(&self, path: &str) -> usize {
        self.targets()
            .iter()
            .filter(|target| {
                target
                    .split(['?', '#'])
                    .next()
                    .map(|value| value == path)
                    .unwrap_or(false)
            })
            .count()
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn serve(mut stream: TcpStream, state: &Arc<Mutex<ServerState>>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(());
    }
    let target = line
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_string();
    let mut range = None;
    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            break;
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.trim().eq_ignore_ascii_case("range") {
                range = Some(value.trim().to_string());
            } else if name.trim().eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    // 读完请求体再响应, 否则关闭连接时未读数据会触发 RST, 客户端偶尔读不到响应。
    if content_length > 0 {
        if content_length > 16 * 1024 * 1024 {
            return Ok(());
        }
        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body)?;
    }

    let path = target
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .to_string();
    let route = {
        let mut state = state.lock().expect("锁定服务状态");
        state.targets.push(target);
        state.ranges.push(range.clone().unwrap_or_default());
        state.routes.get(&path).cloned()
    };

    match route {
        Some(route) => write_route(&mut stream, &route, range.as_deref()),
        None => write_simple(&mut stream, 404, b"not found"),
    }
}

fn write_route(stream: &mut TcpStream, route: &Route, range: Option<&str>) -> std::io::Result<()> {
    if let Some(gate) = &route.gate {
        let gate = gate.lock().unwrap();
        let _ = gate.entered.send(());
        let _ = gate.resume.recv_timeout(Duration::from_secs(5));
    }
    let Some((start, end)) = range.and_then(|value| parse_range(value, route.body.len())) else {
        return write_simple(stream, 200, &route.body);
    };
    if start >= route.body.len() {
        let response = format!(
            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            route.body.len()
        );
        stream.write_all(response.as_bytes())?;
        return stream.flush();
    }
    let end = end.min(route.body.len() - 1);
    let slice = &route.body[start..=end];
    let response = format!(
        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
        slice.len(),
        start,
        end,
        route.body.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.write_all(slice)?;
    stream.flush()
}

/// 解析 `bytes=start-end` 或 `bytes=start-`，返回闭区间端点。
fn parse_range(value: &str, total: usize) -> Option<(usize, usize)> {
    let spec = value.trim().strip_prefix("bytes=")?;
    let (start, end) = spec.split_once('-')?;
    let start: usize = start.trim().parse().ok()?;
    let end = if end.trim().is_empty() {
        total.saturating_sub(1)
    } else {
        end.trim().parse().ok()?
    };
    Some((start, end))
}

fn write_simple(stream: &mut TcpStream, status: u16, body: &[u8]) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        206 => "Partial Content",
        404 => "Not Found",
        416 => "Range Not Satisfiable",
        500 => "Internal Server Error",
        _ => "Status",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

/// 重试因并发 fork 短暂继承写句柄而失败的 exec。
///
/// 测试线程写脚本时，同进程其他测试的 spawn 会 fork 并复制这个写句柄；子进程
/// exec 之前内核认为脚本仍被写入，此时执行它就会得到 `ETXTBSY`。句柄随子进程
/// exec 立即释放，所以有限次重试即可；真实存在的长期占用仍会如实报错。
// 集成测试通过 `#[path]` 引入本模块，库内测试用不到这个重试。
#[allow(dead_code)]
pub fn retry_text_file_busy<T>(mut action: impl FnMut() -> anyhow::Result<T>) -> anyhow::Result<T> {
    const ATTEMPTS: u32 = 32;

    fn text_file_busy(error: &anyhow::Error) -> bool {
        error.chain().any(|cause| {
            cause
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::ExecutableFileBusy)
        })
    }

    let mut delay = Duration::from_millis(1);
    let mut remaining = ATTEMPTS;
    loop {
        let result = action();
        remaining -= 1;
        match result {
            Err(error) if remaining > 0 && text_file_busy(&error) => {
                thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_millis(20));
            }
            result => return result,
        }
    }
}
/// 测试安装域的目录与文件名; 生产宿主不出现这些常量.
pub(crate) const LAUNCHER_DIR: &str = "NTELauncher";
pub(crate) const GLOBAL_LAUNCHER_DIR: &str = "NTEGlobal";
pub(crate) const CLIENT_EXE: &str = "NTEGame.exe";
pub(crate) const GLOBAL_CLIENT_EXE: &str = "NTEGlobalGame.exe";
pub(crate) const DEFAULT_INSTALL_NAME: &str = "Neverness To Everness";
pub(crate) const DEFAULT_GLOBAL_INSTALL_NAME: &str = "Neverness To Everness Global";
pub(crate) const GAME_EXE_NAME: &str = "HTGame";
pub(crate) const PREFIX_LINK_NAME: &str = "Program Files (x86)/Neverness To Everness";
pub(crate) const PREFIX_LINK: &str = "drive_c/Program Files (x86)/Neverness To Everness";

/// 宿主测试用的包长驻域: 只注入"正在写盘/阻塞变更"两个开关.
///
/// 真实包在宿主之外; 宿主只按 [`oddity_runtime::PackageRuntime`] 使用它.
static TEST_PACKAGES: OnceLock<Arc<Registry>> = OnceLock::new();
static PACKAGE_BUSY: AtomicBool = AtomicBool::new(false);
static PACKAGE_BLOCKS: AtomicBool = AtomicBool::new(false);
static PACKAGE_LOCK: Mutex<()> = Mutex::new(());

/// 串行化会改动注入状态的用例与真实包调度共用同一把锁的历史用例.
pub(crate) fn serial() -> MutexGuard<'static, ()> {
    PACKAGE_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

/// 恢复注入状态; 用例开始与结束都调用.
pub(crate) fn reset() {
    PACKAGE_BUSY.store(false, Ordering::Release);
    PACKAGE_BLOCKS.store(false, Ordering::Release);
}

/// 注入"资源任务进行中"的状态; blocks 决定是否阻塞设置/启动.
pub(crate) fn set_package_busy(busy: bool, blocks: bool) {
    PACKAGE_BUSY.store(busy, Ordering::Release);
    PACKAGE_BLOCKS.store(blocks, Ordering::Release);
}

/// 只有一个长驻域的测试注册表.
pub(crate) fn packages() -> Arc<Registry> {
    TEST_PACKAGES
        .get_or_init(|| {
            let mut registry = Registry::new();
            install(&mut registry);
            Arc::new(registry)
        })
        .clone()
}

/// 把宿主测试包装进给定注册表; 需要额外包的用例可以再装自己的包.
pub(crate) fn install(registry: &mut Registry) {
    registry.install(&TestPackage);
}

struct TestPackage;

/// 与测试安装域一致的渠道声明.
pub(crate) static TEST_CHANNELS: &[ChannelDescriptor] = &[
    ChannelDescriptor {
        id: "cn",
        label: "国服",
        is_default: true,
        install_root_key: "cn",
    },
    ChannelDescriptor {
        id: "global",
        label: "国际服",
        is_default: false,
        install_root_key: "global",
    },
];

static TEST_RUNTIME: TestRuntime = TestRuntime;

impl GamePackage for TestPackage {
    fn descriptor(&self) -> PackageDescriptor {
        PackageDescriptor {
            id: "nte",
            name: "异环",
        }
    }

    fn register(&self, _registry: &mut Registry) {}

    fn install_locator(&self) -> Option<&'static dyn InstallLocator> {
        Some(&TEST_LOCATOR)
    }

    fn channels(&self) -> &'static [ChannelDescriptor] {
        TEST_CHANNELS
    }

    fn runtime(&self) -> Option<&'static dyn PackageRuntime> {
        Some(&TEST_RUNTIME)
    }
}

struct TestRuntime;

impl PackageRuntime for TestRuntime {
    fn start(&self, _host: Arc<dyn HostContext>, _stop: Arc<AtomicBool>) {}

    fn busy(&self) -> bool {
        PACKAGE_BUSY.load(Ordering::Acquire)
    }

    fn blocks_changes(&self) -> bool {
        PACKAGE_BLOCKS.load(Ordering::Acquire)
    }

    fn gate(&self) -> Result<Box<dyn PackageGate>, String> {
        Ok(Box::new(TestGate))
    }
}

struct TestGate;

impl PackageGate for TestGate {
    fn blocks_changes(&self) -> bool {
        PACKAGE_BLOCKS.load(Ordering::Acquire)
    }
}
/// 测试用的安装域: 复刻《异环》国服/国际服的官方布局.
///
/// 宿主用例需要一份真实的安装目录知识; 产品代码不持有这些常量, 这里只为
/// 测试固定布局 (H4 身份去枚举后由包提供).
static TEST_LOCATOR: TestLocator = TestLocator;

struct TestLocator;

/// 区服与安装布局的测试镜像.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestRegion {
    Cn,
    Global,
}

impl TestRegion {
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "cn" => Some(Self::Cn),
            "global" => Some(Self::Global),
            _ => None,
        }
    }

    fn profile(self) -> &'static TestProfile {
        match self {
            Self::Cn => &CN,
            Self::Global => &GLOBAL,
        }
    }

    fn other(self) -> Self {
        match self {
            Self::Cn => Self::Global,
            Self::Global => Self::Cn,
        }
    }
}

struct TestProfile {
    app_id: i64,
    display_name: &'static str,
    default_install_name: &'static str,
    launcher_dir: &'static str,
    client_exe: &'static str,
}

static CN: TestProfile = TestProfile {
    app_id: 1289,
    display_name: "异环",
    default_install_name: "Neverness To Everness",
    launcher_dir: "NTELauncher",
    client_exe: "NTEGame.exe",
};

static GLOBAL: TestProfile = TestProfile {
    app_id: 3_000_001,
    display_name: "NTE",
    default_install_name: "Neverness To Everness Global",
    launcher_dir: "NTEGlobal",
    client_exe: "NTEGlobalGame.exe",
};

const ENV_KEYS: [&str; 2] = ["NTE_ROOT", "NTE_INSTALL"];
const LEGACY_INSTALL_NAMES: [&str; 2] = ["Neverness To Everness", "Neverness to Everness"];
const CHINESE_HOME_SEED: &str = "异环";
const MOUNT_ROOTS: [&str; 3] = ["/run/media", "/media", "/mnt"];
pub(crate) const GAME_EXE: &str = "Client/WindowsNoEditor/HT/Binaries/Win64/HTGame.exe";
const SDK_DLL: &str = "WPGameClientSDK_64.dll";
const LAUNCHER_CONFIG: &str = "GameLauncher.config";

struct Probe {
    home: PathBuf,
    cwd: Option<PathBuf>,
    vars: Vec<(&'static str, OsString)>,
    mounts: Vec<PathBuf>,
}

impl Probe {
    fn process() -> Self {
        Self {
            home: std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/tmp")),
            cwd: std::env::current_dir().ok(),
            vars: ENV_KEYS
                .iter()
                .filter_map(|key| std::env::var_os(key).map(|value| (*key, value)))
                .collect(),
            mounts: MOUNT_ROOTS.iter().map(PathBuf::from).collect(),
        }
    }

    fn var(&self, key: &str) -> Option<&OsStr> {
        self.vars
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value.as_os_str())
    }

    fn expand(&self, raw: &OsStr) -> PathBuf {
        expand(raw.to_string_lossy().as_ref(), &self.home)
    }
}

fn expand(raw: &str, home: &Path) -> PathBuf {
    if raw == "~" {
        return home.to_path_buf();
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return home.join(rest);
    }
    PathBuf::from(raw)
}

fn detect(probe: &Probe, query: &InstallQuery) -> Option<PathBuf> {
    let region = TestRegion::parse(&query.distribution)?;
    if let Some(configured) = &query.configured_install_root {
        return normalize(configured, region);
    }
    for key in ENV_KEYS {
        if let Some(raw) = probe.var(key) {
            if let Some(root) = normalize(&probe.expand(raw), region) {
                return Some(root);
            }
        }
    }
    let games = probe.home.join("Games");
    let mut seeds = vec![games
        .join(region.profile().default_install_name)
        .join(region.profile().launcher_dir)];
    install_seeds(&games, region, &mut seeds);
    if region == TestRegion::Cn {
        seeds.push(
            games
                .join(CHINESE_HOME_SEED)
                .join(region.profile().launcher_dir),
        );
    }
    if let Some(cwd) = &probe.cwd {
        for ancestor in cwd.ancestors() {
            install_seeds(ancestor, region, &mut seeds);
        }
    }
    for mount_root in &probe.mounts {
        let Ok(entries) = std::fs::read_dir(mount_root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            install_seeds(&path, region, &mut seeds);
            if let Ok(children) = std::fs::read_dir(&path) {
                for child in children.flatten() {
                    install_seeds(&child.path(), region, &mut seeds);
                }
            }
        }
    }
    seeds.into_iter().find_map(|seed| normalize(&seed, region))
}

fn install_seeds(base: &Path, region: TestRegion, seeds: &mut Vec<PathBuf>) {
    for name in [
        region.profile().default_install_name,
        LEGACY_INSTALL_NAMES[0],
        LEGACY_INSTALL_NAMES[1],
    ] {
        for prefix in [
            base.join("Program Files (x86)"),
            base.join("Program Files"),
            base.to_path_buf(),
        ] {
            let candidate = prefix.join(name).join(region.profile().launcher_dir);
            if !seeds.contains(&candidate) {
                seeds.push(candidate);
            }
        }
    }
    seeds.push(base.join(region.profile().launcher_dir));
}

fn normalize(path: &Path, region: TestRegion) -> Option<PathBuf> {
    let root = if path.join("Config").join("Config.ini").is_file() {
        path.to_path_buf()
    } else {
        let nested = path.join(region.profile().launcher_dir);
        if !nested.join("Config").join("Config.ini").is_file() {
            return None;
        }
        nested
    };
    if root
        .file_name()
        .is_some_and(|name| name == region.other().profile().launcher_dir)
    {
        return None;
    }
    match config_game_id(&root) {
        Some(game_id) if game_id != 0 && game_id != region.profile().app_id => None,
        _ => Some(root),
    }
}

fn config_game_id(root: &Path) -> Option<i64> {
    let text = std::fs::read_to_string(root.join("Config").join("Config.ini")).ok()?;
    Some(ini_value(&text, "General", "GameID").map_or(0, |value| value.parse().unwrap_or(0)))
}

fn ini_value<'a>(text: &'a str, section: &str, key: &str) -> Option<&'a str> {
    let mut current: &str = "General";
    let mut value = None;
    for raw in text.trim_start_matches('\u{feff}').lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            current = name.trim();
            continue;
        }
        if current != section {
            continue;
        }
        if let Some((name, found)) = line.split_once('=') {
            if name.trim() == key {
                value = Some(found.trim());
            }
        }
    }
    value
}

fn selected_install_root(probe: &Probe, query: &InstallQuery) -> PathBuf {
    let Some(region) = TestRegion::parse(&query.distribution) else {
        return probe.home.join("Games");
    };
    let path = query.configured_install_root.clone().unwrap_or_else(|| {
        probe
            .home
            .join("Games")
            .join(region.profile().default_install_name)
    });
    if path
        .file_name()
        .is_some_and(|name| name == region.profile().launcher_dir)
    {
        path.parent().unwrap_or(&path).to_path_buf()
    } else {
        path
    }
}

fn effective_install_root(
    probe: &Probe,
    query: &InstallQuery,
    launcher_root: Option<&Path>,
) -> PathBuf {
    if query.configured_install_root.is_some() {
        return selected_install_root(probe, query);
    }
    launcher_root
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| selected_install_root(probe, query))
}

fn find_relative_ci(base: &Path, relative: &str) -> Option<PathBuf> {
    let components: Vec<&str> = relative
        .split('/')
        .filter(|component| !component.is_empty())
        .collect();
    find_ci(base, &components)
}

fn find_ci(base: &Path, components: &[&str]) -> Option<PathBuf> {
    let mut current = base.to_path_buf();
    for component in components {
        let entries = std::fs::read_dir(&current).ok()?;
        let mut found = None;
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(component)
            {
                found = Some(entry.path());
                break;
            }
        }
        current = found?;
    }
    Some(current)
}

impl InstallLocator for TestLocator {
    fn launcher_root(&self, query: &InstallQuery) -> Option<PathBuf> {
        detect(&Probe::process(), query)
    }

    fn launcher_dir(&self, query: &InstallQuery) -> Option<&'static str> {
        TestRegion::parse(&query.distribution).map(|region| region.profile().launcher_dir)
    }

    fn selected_install_root(&self, query: &InstallQuery) -> PathBuf {
        selected_install_root(&Probe::process(), query)
    }

    fn effective_install_root(
        &self,
        query: &InstallQuery,
        launcher_root: Option<&Path>,
    ) -> PathBuf {
        effective_install_root(&Probe::process(), query, launcher_root)
    }

    fn game_exe(&self, launcher_root: &Path) -> Option<PathBuf> {
        find_relative_ci(launcher_root.parent()?, GAME_EXE)
    }

    fn game_version(&self, launcher_root: &Path) -> String {
        let path = launcher_root
            .join("UserData")
            .join("Config")
            .join("Config.ini");
        let Ok(text) = std::fs::read_to_string(&path) else {
            return String::new();
        };
        ini_value(&text, "Game", "Version_")
            .unwrap_or_default()
            .to_string()
    }

    fn launcher_info(&self, launcher_root: &Path) -> Option<LauncherInfo> {
        let text = std::fs::read_to_string(launcher_root.join("Config").join("Config.ini")).ok()?;
        Some(LauncherInfo {
            version: ini_value(&text, "VERSION", "Version")
                .unwrap_or_default()
                .to_string(),
            build: ini_value(&text, "VERSION", "Build")
                .unwrap_or_default()
                .to_string(),
        })
    }

    fn game_exe_hint(&self) -> String {
        match Path::new(GAME_EXE).file_name() {
            Some(name) => format!("未找到 {}（注意 Win64 大小写）", name.to_string_lossy()),
            None => "未找到游戏可执行文件".to_string(),
        }
    }

    fn sdk_dir(&self, launcher_root: &Path) -> Option<PathBuf> {
        let component = find_ci(launcher_root, &[SDK_DLL])?;
        component.parent().map(Path::to_path_buf)
    }

    fn official_client_exe(&self, launcher_root: &Path) -> Option<PathBuf> {
        let region = match launcher_root.file_name()?.to_str()? {
            "NTELauncher" => TestRegion::Cn,
            "NTEGlobal" => TestRegion::Global,
            _ => return None,
        };
        Some(launcher_root.join(region.profile().client_exe))
    }

    fn prefix_link_name(&self) -> Option<&'static str> {
        Some("Program Files (x86)/Neverness To Everness")
    }

    fn launcher_config_path(&self, install_root: &Path) -> Option<PathBuf> {
        let config = install_root.join(GAME_EXE).with_file_name(LAUNCHER_CONFIG);
        config.is_file().then_some(config)
    }
}
