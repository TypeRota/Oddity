//! 包内测试辅助: 临时目录, 环境变量, 回环 HTTP 服务与最小宿主.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use oddity_runtime::{
    ActivitySnapshot, EventSink, HostContext, SettingsSnapshot, Task, TaskJob, TaskRegistry,
};
use serde_json::Value;

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// 析构时自动删除的临时目录.
pub(crate) struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub(crate) fn new(tag: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "oddity-game-nte-{tag}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("创建临时目录");
        Self { path }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn join(&self, rel: impl AsRef<Path>) -> PathBuf {
        self.path.join(rel)
    }

    pub(crate) fn write(&self, rel: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> PathBuf {
        let full = self.path.join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).expect("创建父目录");
        }
        fs::write(&full, contents).expect("写入文件");
        full
    }

    pub(crate) fn read(&self, rel: impl AsRef<Path>) -> Vec<u8> {
        fs::read(self.path.join(rel)).expect("读取文件")
    }

    pub(crate) fn read_text(&self, rel: impl AsRef<Path>) -> String {
        fs::read_to_string(self.path.join(rel)).expect("读取文本文件")
    }

    pub(crate) fn is_file(&self, rel: impl AsRef<Path>) -> bool {
        self.path.join(rel).is_file()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// 独占进程环境变量, 并在离开作用域时恢复原值.
///
/// 互斥锁只保证并行测试不互相踩, 恢复原值才能保证测试结束后进程环境不变,
/// 也不会清除使用者原有的环境变量.
pub(crate) struct EnvGuard {
    saved: Vec<(&'static str, Option<OsString>)>,
    _lock: MutexGuard<'static, ()>,
}

impl EnvGuard {
    pub(crate) fn new() -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        Self {
            saved: Vec::new(),
            _lock: lock,
        }
    }

    pub(crate) fn set(&mut self, key: &'static str, value: impl AsRef<OsStr>) {
        self.remember(key);
        std::env::set_var(key, value);
    }

    pub(crate) fn remove(&mut self, key: &'static str) {
        self.remember(key);
        std::env::remove_var(key);
    }

    /// 同一个变量只记第一次的原始值, 之后覆盖都还原到它.
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

/// 只服务已注册路径的回环 HTTP 服务.
pub(crate) struct HttpServer {
    addr: SocketAddr,
    state: Arc<Mutex<ServerState>>,
    stop: Arc<AtomicBool>,
}

impl HttpServer {
    pub(crate) fn start() -> Self {
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

    pub(crate) fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}{}", self.base(), path)
    }

    /// 注册一个 200 响应, 按 Range 头返回部分内容或完整内容.
    pub(crate) fn route(&self, path: impl AsRef<str>, body: impl AsRef<[u8]>) {
        let mut state = self.state.lock().expect("锁定服务状态");
        state.routes.insert(
            path.as_ref().to_string(),
            Route {
                body: body.as_ref().to_vec(),
                gate: None,
            },
        );
    }

    /// 暂停指定响应, 让并发测试控制完成顺序; 超时防止失败测试挂起.
    pub(crate) fn route_blocked(
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

    /// 记录到的请求目标（含查询串），按到达顺序排列.
    pub(crate) fn targets(&self) -> Vec<String> {
        self.state.lock().expect("锁定服务状态").targets.clone()
    }

    /// 记录到的 Range 请求头，与 `targets` 一一对应；没有 Range 时为空串.
    pub(crate) fn ranges(&self) -> Vec<String> {
        self.state.lock().expect("锁定服务状态").ranges.clone()
    }

    /// 某个路径被请求的次数.
    pub(crate) fn hits(&self, path: &str) -> usize {
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
            "HTTP/1.1 416 Range Not Satisfiable
Content-Range: bytes */{}
Content-Length: 0
Connection: close

",
            route.body.len()
        );
        stream.write_all(response.as_bytes())?;
        return stream.flush();
    }
    let end = end.min(route.body.len() - 1);
    let slice = &route.body[start..=end];
    let response = format!(
        "HTTP/1.1 206 Partial Content
Content-Length: {}
Content-Range: bytes {}-{}/{}
Connection: close

",
        slice.len(),
        start,
        end,
        route.body.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.write_all(slice)?;
    stream.flush()
}

/// 解析 `bytes=start-end` 或 `bytes=start-`, 返回闭区间端点.
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

/// 把可执行脚本写到临时目录，用于测试外部命令调用。
pub(crate) fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    fs::write(&path, body).expect("写入脚本");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("设置脚本权限");
    path
}

/// 记录事件的测试出口; 任务进度与命令事件都落在这里.
#[derive(Default)]
pub(crate) struct RecordingSink(Mutex<Vec<(&'static str, Value)>>);

impl RecordingSink {
    pub(crate) fn record(&self, id: &'static str, payload: Value) {
        self.0.lock().unwrap().push((id, payload));
    }

    pub(crate) fn recorded(&self) -> Vec<(&'static str, Value)> {
        self.0.lock().unwrap().clone()
    }
}

impl EventSink for RecordingSink {
    fn deliver(
        &self,
        event: &'static oddity_contract::Event,
        payload: Value,
    ) -> Result<(), String> {
        self.record(event.id, payload);
        Ok(())
    }
}

/// 最小宿主: 只提供包内命令用到的能力.
pub(crate) struct TestHost {
    pub(crate) launcher_root: Option<PathBuf>,
    pub(crate) game: String,
    pub(crate) distribution: String,
    /// 设置里配置的安装根, 与宿主展开后的快照一致.
    pub(crate) install_root: Option<String>,
    pub(crate) settings_error: bool,
    pub(crate) activity: ActivitySnapshot,
    pub(crate) events: Arc<RecordingSink>,
    tasks: Arc<TaskRegistry>,
}

impl TestHost {
    pub(crate) fn new(launcher_root: Option<PathBuf>) -> Self {
        Self {
            launcher_root,
            game: "nte".into(),
            distribution: "cn".into(),
            install_root: None,
            settings_error: false,
            activity: ActivitySnapshot::default(),
            events: Arc::default(),
            tasks: Arc::default(),
        }
    }

    pub(crate) fn with_settings(game: &str, distribution: &str) -> Self {
        Self {
            game: game.into(),
            distribution: distribution.into(),
            ..Self::new(None)
        }
    }

    pub(crate) fn failing_settings() -> Self {
        Self {
            settings_error: true,
            ..Self::new(None)
        }
    }
}

impl HostContext for TestHost {
    fn settings(&self) -> Result<SettingsSnapshot, String> {
        if self.settings_error {
            return Err("测试设置不可读".into());
        }
        Ok(SettingsSnapshot {
            game: self.game.clone(),
            distribution: self.distribution.clone(),
            language: "zh-CN".into(),
            reduce_motion: false,
            theme: "light".into(),
            install_root: self.install_root.clone(),
            preload_root: None,
            update: Default::default(),
        })
    }

    fn activity(&self) -> Result<ActivitySnapshot, String> {
        Ok(self.activity.clone())
    }

    fn official_launcher_root(&self) -> Option<PathBuf> {
        self.launcher_root.clone()
    }

    fn emit(&self, id: &'static str, payload: Value) -> Result<(), String> {
        self.events.record(id, payload);
        Ok(())
    }

    fn submit(&self, job: TaskJob) -> Task {
        let events = Arc::clone(&self.events);
        let task = Task::new(Arc::new(move |payload| {
            events.record("task-progress", payload);
            Ok(())
        }));
        let task = self.tasks.track(task);
        std::thread::spawn(task.runner(job));
        task
    }

    fn cancel_task(&self, id: &str) -> bool {
        self.tasks.cancel(id)
    }
}

/// 在当前线程运行时上驱动命令 future.
pub(crate) fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("构建测试运行时")
        .block_on(future)
}
