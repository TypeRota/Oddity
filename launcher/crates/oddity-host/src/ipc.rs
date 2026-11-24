//! 会话服务的本机传输: Unix 域套接字上的行分隔 JSON.
//!
//! 会话服务不依附界面进程, 界面连上来读取权威状态, 断开后服务继续持有运行中
//! 的会话. 每个连接一条写线程: 事件广播到全部连接, 回复只回到发起请求的那条
//! 连接, 因此同一条连接的字节顺序始终由写线程串行产生.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use crate::service::ServiceHandle;

/// 单个连接的待发上限; 界面线程读得比事件产生快, 留足余量即可.
const CLIENT_QUEUE: usize = 256;

/// 套接字文件名.
const SOCKET_NAME: &str = "ipc.sock";

/// 数据目录里的入口文件名, 内容为真实套接字路径.
const SOCKET_ENTRY: &str = "ipc-path";

/// 套接字文件的存放目录.
///
/// Unix 域套接字路径有 108 字节的硬上限, 数据目录可能很深 (CI 的临时目录,
/// 或用户自定的 XDG_DATA_HOME), 直接放进去会绑定失败. 因此放进运行目录,
/// 并按数据目录的摘要分开放: 同一用户的多个数据目录互不干扰.
fn socket_dir(data_dir: &Path) -> PathBuf {
    let key = digest(data_dir);
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        let runtime = PathBuf::from(runtime);
        if !runtime.as_os_str().is_empty() && runtime.is_dir() {
            return runtime.join("oddity").join(&key);
        }
    }
    // 运行目录缺失时退到 /tmp 下按用户隔离的目录, 仍然足够短.
    let uid = unsafe { libc::geteuid() };
    PathBuf::from("/tmp")
        .join(format!("oddity-{uid}"))
        .join(key)
}

/// 数据目录的稳定摘要, 只用于区分不同的数据目录.
fn digest(data_dir: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data_dir.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// 查询类请求的读取超时; 服务卡住时不要让调用方无限等待.
const QUERY_TIMEOUT: Duration = Duration::from_secs(5);

/// 服务实际监听的套接字路径.
///
/// Unix 域套接字路径有 108 字节的硬上限, 客户端的 connect 同样受限, 所以
/// 套接字只能放在短路径下; 数据目录只保存指向它的入口文件.
pub fn socket_path(data_dir: &Path) -> PathBuf {
    socket_dir(data_dir).join(SOCKET_NAME)
}

/// 数据目录里的入口文件, 记录真实套接字路径.
///
/// 客户端读这个文件拿到短路径再连接; 文件本身不参与套接字寻址, 因此数据
/// 目录多深都不影响连接.
pub fn entry_path(data_dir: &Path) -> PathBuf {
    data_dir.join(SOCKET_ENTRY)
}

fn write_entry(entry: &Path, target: &Path) -> Result<(), String> {
    if let Some(parent) = entry.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("创建会话服务数据目录失败: {error}"))?;
    }
    std::fs::write(entry, target.as_os_str().as_encoded_bytes())
        .map_err(|error| format!("写入会话服务入口失败: {error}"))?;
    if let Err(error) = std::fs::set_permissions(entry, std::fs::Permissions::from_mode(0o600)) {
        let _ = std::fs::remove_file(entry);
        return Err(format!("设置会话服务入口权限失败: {error}"));
    }
    Ok(())
}

/// 读取入口文件里的套接字路径; 没有入口时返回 None.
pub fn read_entry(data_dir: &Path) -> Option<PathBuf> {
    let bytes = std::fs::read(entry_path(data_dir)).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    let path = PathBuf::from(text.trim());
    (!path.as_os_str().is_empty()).then_some(path)
}

/// 绑定监听套接字并写出入口; 地址被占用时先确认没有活着的服务, 再清理残留.
pub fn listen(data_dir: &Path, _entry: &Path) -> Result<UnixListener, String> {
    let path = socket_path(data_dir);
    let listener = bind(&path)?;
    write_entry(&entry_path(data_dir), &path)?;
    Ok(listener)
}

/// 清除入口文件与套接字.
pub fn release(data_dir: &Path, _entry: &Path) {
    let _ = std::fs::remove_file(entry_path(data_dir));
    let _ = std::fs::remove_file(socket_path(data_dir));
}

fn bind(path: &Path) -> Result<UnixListener, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("创建会话服务目录失败: {error}"))?;
    }
    match UnixListener::bind(path) {
        Ok(listener) => {
            set_private(path)?;
            Ok(listener)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            if UnixStream::connect(path).is_ok() {
                return Err("会话服务已在运行.".into());
            }
            // 连不上说明是上次异常退出留下的套接字文件.
            std::fs::remove_file(path).map_err(|error| format!("清理残留套接字失败: {error}"))?;
            let listener = UnixListener::bind(path)
                .map_err(|error| format!("绑定会话服务套接字失败: {error}"))?;
            set_private(path)?;
            Ok(listener)
        }
        Err(error) => Err(format!("绑定会话服务套接字失败: {error}")),
    }
}

fn set_private(path: &Path) -> Result<(), String> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("设置套接字权限失败: {error}"))
}

/// 已连接的界面进程.
///
/// 广播持锁阻塞等待写线程收下消息: 事件顺序不能丢, 而界面卡死时最多阻塞事件
/// 发送方, 不会破坏连接内的字节顺序.
#[derive(Default)]
pub struct Clients {
    next: AtomicU64,
    connected: Mutex<Vec<(u64, SyncSender<Value>)>>,
}

impl Clients {
    fn register(&self) -> (u64, SyncSender<Value>, Receiver<Value>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (writer, reader) = mpsc::sync_channel(CLIENT_QUEUE);
        if let Ok(mut connected) = self.connected.lock() {
            connected.push((id, writer.clone()));
        }
        (id, writer, reader)
    }

    fn unregister(&self, id: u64) {
        if let Ok(mut connected) = self.connected.lock() {
            connected.retain(|(current, _)| *current != id);
        }
    }

    /// 当前连接数.
    pub fn count(&self) -> usize {
        self.connected
            .lock()
            .map(|connected| connected.len())
            .unwrap_or(0)
    }

    /// 测试用登记, 返回连接 id 与消息接收端.
    #[cfg(test)]
    pub fn register_for_test(&self) -> (u64, Receiver<Value>) {
        let (id, _writer, reader) = self.register();
        (id, reader)
    }

    /// 测试用注销.
    #[cfg(test)]
    pub fn unregister_for_test(&self, id: u64) {
        self.unregister(id);
    }

    /// 广播一条事件; 写不出去的连接直接摘掉.
    ///
    /// 先取一份发送端快照再发送: 卡死的界面只会堵住这一条事件, 不会连累其他
    /// 连接, 也不会让注销请求排在后面. 队列满时视为该连接跟不上, 直接断开.
    /// 一条连接都发不出去时返回错误, 与没有接收方的语义一致.
    pub fn broadcast(&self, value: Value) -> Result<(), String> {
        let targets: Vec<(u64, SyncSender<Value>)> = self
            .connected
            .lock()
            .map(|connected| connected.clone())
            .unwrap_or_default();
        if targets.is_empty() {
            return Err("没有已连接的界面".into());
        }
        let mut gone = Vec::new();
        let mut delivered = 0;
        for (id, writer) in targets {
            if writer.try_send(value.clone()).is_ok() {
                delivered += 1;
            } else {
                gone.push(id);
            }
        }
        for id in gone {
            self.unregister(id);
        }
        if delivered == 0 {
            return Err("所有界面连接都已不可用".into());
        }
        Ok(())
    }
}

/// 接受一条连接并服务到对端关闭.
pub fn serve(app: ServiceHandle, clients: Arc<Clients>, stream: UnixStream) -> Result<(), String> {
    let writer = stream
        .try_clone()
        .map_err(|error| format!("复制会话连接失败: {error}"))?;
    let (id, sender, messages) = clients.register();
    thread::Builder::new()
        .name("oddity-ipc-writer".into())
        .spawn(move || write_messages(writer, messages))
        .map_err(|error| format!("启动会话写线程失败: {error}"))?;
    let result = read_requests(app, stream, &sender);
    clients.unregister(id);
    result
}

/// 写线程: 串行写出回复与事件, 直到对端关闭.
fn write_messages(mut stream: UnixStream, messages: Receiver<Value>) {
    for message in messages {
        if serde_json::to_writer(&mut stream, &message).is_err()
            || stream.write_all(b"\n").is_err()
            || stream.flush().is_err()
        {
            return;
        }
    }
}

/// 读线程: 逐行读取请求并回复; 对端关闭时结束.
fn read_requests(
    app: ServiceHandle,
    stream: UnixStream,
    sender: &SyncSender<Value>,
) -> Result<(), String> {
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else {
            break;
        };
        let Ok(request) = serde_json::from_str::<crate::protocol::Request>(&line) else {
            eprintln!("无效宿主请求");
            continue;
        };
        let id = request.id;
        let reply_sender = sender.clone();
        // 窗口状态要同步落到输入设备, 与其余命令一样交给执行器, 但排在同一连接
        // 的请求顺序上: 读线程本身不阻塞, 否则界面会等不到后续回复.
        let worker = app.clone();
        crate::service::spawn(async move {
            let result =
                crate::protocol::dispatch(worker, request.command, request.game, request.args)
                    .await;
            let _ = reply_sender.send(reply_of(id, result));
        });
    }
    Ok(())
}

fn reply_of(id: u64, result: Result<Value, Value>) -> Value {
    match result {
        Ok(value) => json!({"id": id, "result": value}),
        Err(error) => json!({"id": id, "error": error}),
    }
}

/// 一条到会话服务的连接; 请求与事件都在同一条连接上按序到达.
pub struct Client {
    stream: UnixStream,
    next: u64,
    timeout: Duration,
}

impl Client {
    /// 连接会话服务; 服务未运行时返回 None.
    ///
    /// 参数是数据目录: 真实套接字路径从入口文件读取, 以免受路径长度限制.
    pub fn connect(data_dir: &Path) -> Result<Option<Self>, String> {
        let Some(path) = read_entry(data_dir) else {
            return Ok(None);
        };
        let Ok(stream) = UnixStream::connect(&path) else {
            return Ok(None);
        };
        stream
            .set_read_timeout(Some(QUERY_TIMEOUT))
            .map_err(|error| format!("设置查询超时失败: {error}"))?;
        Ok(Some(Self {
            stream,
            next: 0,
            timeout: QUERY_TIMEOUT,
        }))
    }

    /// 调整单次请求的等待上限.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        // 过短的超时会让长时间任务误判, 读写共用同一条连接的设定.
        let _ = self.stream.set_read_timeout(Some(timeout));
        self
    }

    /// 发一条命令并等待它的回复, 途中的事件交给回调.
    pub fn call<E>(&mut self, command: &str, args: Value, mut event: E) -> Result<Value, String>
    where
        E: FnMut(Value),
    {
        self.next += 1;
        let id = self.next;
        let request = json!({"id": id, "command": command, "args": args});
        let mut writer = self
            .stream
            .try_clone()
            .map_err(|error| format!("复制查询连接失败: {error}"))?;
        serde_json::to_writer(&mut writer, &request)
            .map_err(|error| format!("发送查询失败: {error}"))?;
        writer
            .write_all(b"\n")
            .and_then(|()| writer.flush())
            .map_err(|error| format!("发送查询失败: {error}"))?;
        self.stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|error| format!("设置查询超时失败: {error}"))?;
        let mut reader = BufReader::new(
            self.stream
                .try_clone()
                .map_err(|error| format!("复制查询连接失败: {error}"))?,
        );
        loop {
            let mut line = String::new();
            let count = reader
                .read_line(&mut line)
                .map_err(|error| format!("读取查询结果失败: {error}"))?;
            if count == 0 {
                return Err("会话服务提前关闭了查询连接.".into());
            }
            let value: Value =
                serde_json::from_str(&line).map_err(|error| format!("查询结果无效: {error}"))?;
            if value.get("event").is_some() {
                event(value);
                continue;
            }
            if value.get("id") != Some(&json!(id)) {
                continue;
            }
            if let Some(error) = value.get("error") {
                return Err(match error.as_str() {
                    Some(text) => text.to_string(),
                    None => error.to_string(),
                });
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

/// 向会话服务发一条查询并取回结果; 连不上表示服务未运行.
///
/// 只用于一次性查询; 需要连续调用时复用 [`Client`], 避免每次重新连接.
pub fn query(data_dir: &Path, command: &str, args: Value) -> Result<Option<Value>, String> {
    let Some(mut client) = Client::connect(data_dir)? else {
        return Ok(None);
    };
    client.call(command, args, |_| {}).map(Some)
}
/// 终止信号处理器向主循环投递的唯一通道.
static CHANNEL: Mutex<Option<SyncSender<()>>> = Mutex::new(None);

/// 收到终止信号时置停止位, 让服务走正常收尾.
///
/// 终止信号只做置位: 清理, 释放锁与删除套接字都在主循环退出后完成.
pub fn exit_on_termination(app: ServiceHandle) {
    let (sender, receiver) = mpsc::sync_channel::<()>(1);
    if let Ok(mut slot) = CHANNEL.lock() {
        *slot = Some(sender.clone());
    }
    unsafe {
        let handler = handle_termination as *const () as libc::sighandler_t;
        libc::signal(libc::SIGTERM, handler);
        libc::signal(libc::SIGINT, handler);
    }
    let _ = std::thread::Builder::new()
        .name("oddity-termination".into())
        .spawn(move || {
            if receiver.recv().is_ok() {
                app.stopping.store(true, Ordering::Release);
                // 主循环靠输入队列唤醒; 空任务只用于打断等待.
                let _ = app.input_task(|| {});
            }
        });
}

extern "C" fn handle_termination(_signal: libc::c_int) {
    let sender = CHANNEL.lock().ok().and_then(|slot| slot.as_ref().cloned());
    if let Some(sender) = sender {
        let _ = sender.try_send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{test_service, ServiceHandle};
    use std::os::unix::fs::MetadataExt;

    /// 绑定一个临时套接字.
    /// 绑定监听套接字并写出入口; 返回监听器与真实套接字路径.
    fn listener(directory: &Path) -> (UnixListener, PathBuf) {
        let entry = entry_path(directory);
        let listener = listen(directory, &entry).expect("绑定套接字");
        (listener, socket_path(directory))
    }

    /// 接受连接直到监听套接字被丢弃, 每条连接一个线程.
    fn accept_all(
        app: ServiceHandle,
        clients: Arc<Clients>,
        listener: UnixListener,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let app = app.clone();
                let clients = Arc::clone(&clients);
                std::thread::spawn(move || {
                    let _ = serve(app, clients, stream);
                });
            }
        })
    }

    /// 连接套接字, 返回写端与读端.
    fn connect(path: &Path) -> (UnixStream, BufReader<UnixStream>) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(stream) = UnixStream::connect(path) {
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .expect("设置读取超时");
                let reader = BufReader::new(stream.try_clone().expect("复制连接"));
                return (stream, reader);
            }
            assert!(std::time::Instant::now() < deadline, "连接套接字超时");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// 读一条消息; 超时视为失败.
    fn read_message(reader: &mut BufReader<UnixStream>) -> Value {
        let mut line = String::new();
        reader.read_line(&mut line).expect("读取消息");
        serde_json::from_str(&line).expect("消息是 JSON")
    }

    fn send(stream: &UnixStream, value: Value) {
        let mut writer = stream.try_clone().expect("复制连接");
        writeln!(writer, "{value}").expect("发送请求");
        writer.flush().expect("刷新请求");
    }

    #[test]
    fn socket_is_private_and_stale_files_are_replaced() {
        let directory = crate::tests::TempDir::new("ipc-bind");
        let (active, path) = listener(directory.path());
        let mode = std::fs::metadata(&path)
            .expect("读取套接字状态")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);

        // 活着的服务再次绑定会被拒绝, 不能悄悄抢走地址.
        assert!(bind(&path).is_err());

        drop(active);
        // 没有进程监听的残留文件要能被清理并重新绑定: 新 inode 说明换掉了旧文件.
        let stale = std::fs::metadata(&path).expect("残留文件");
        let (rebound, _) = listener(directory.path());
        let fresh = std::fs::metadata(&path).expect("新套接字");
        assert_ne!(stale.ino(), fresh.ino());
        drop(rebound);
    }

    #[test]
    fn replies_return_to_the_asking_connection_and_events_reach_everyone() {
        let app = test_service();
        let clients = Arc::new(Clients::default());
        let directory = crate::tests::TempDir::new("ipc-broadcast");
        let (listener, path) = listener(directory.path());
        accept_all(app, Arc::clone(&clients), listener);

        let (first, mut first_reader) = connect(&path);
        let (second, mut second_reader) = connect(&path);
        send(
            &first,
            json!({"id": 11, "command": "game_status", "args": {}}),
        );
        send(
            &second,
            json!({"id": 22, "command": "game_status", "args": {}}),
        );
        assert_eq!(read_message(&mut first_reader)["id"], json!(11));
        assert_eq!(read_message(&mut second_reader)["id"], json!(22));

        // 广播的事件两条连接都能读到.
        clients
            .broadcast(json!({"event": "host-show", "payload": null}))
            .expect("广播事件");
        assert_eq!(read_message(&mut first_reader)["event"], json!("host-show"));
        assert_eq!(
            read_message(&mut second_reader)["event"],
            json!("host-show")
        );
        assert_eq!(clients.count(), 2);

        // 读端也是连接的一份副本, 全部释放后服务端才会看到 EOF.
        drop(first);
        drop(second);
        drop(first_reader);
        drop(second_reader);
        // 连接全部关闭后登记表清空.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while clients.count() > 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(clients.count(), 0);
    }

    #[test]
    fn invalid_requests_are_skipped_without_closing_the_connection() {
        let app = test_service();
        let clients = Arc::new(Clients::default());
        let directory = crate::tests::TempDir::new("ipc-invalid");
        let (listener, path) = listener(directory.path());
        accept_all(app, Arc::clone(&clients), listener);

        let (stream, mut reader) = connect(&path);
        let mut writer = stream.try_clone().expect("复制连接");
        writeln!(writer, "{{not json").expect("发送无效请求");
        writeln!(
            writer,
            "{}",
            json!({"id": 7, "command": "game_status", "args": {}})
        )
        .expect("发送请求");
        assert_eq!(read_message(&mut reader)["id"], json!(7));

        drop(writer);
        drop(stream);
        drop(reader);
    }

    #[test]
    fn socket_path_fits_the_unix_limit_for_deep_data_directories() {
        // Unix 域套接字路径上限 108 字节, connect 同样受限, 因此套接字只能放短路径.
        let deep = PathBuf::from("/tmp")
            .join("a".repeat(60))
            .join("b".repeat(30));
        let socket = socket_path(&deep);
        assert!(
            socket.as_os_str().len() < 100,
            "套接字路径过长: {}",
            socket.display()
        );
        // 不同数据目录不能共用一个套接字文件.
        let other = PathBuf::from("/tmp").join("c".repeat(60));
        assert_ne!(socket_path(&other), socket);
    }

    #[test]
    fn data_directory_entry_records_the_real_socket() {
        let directory = crate::tests::TempDir::new("ipc-entry");
        let entry = entry_path(directory.path());
        let socket = socket_path(directory.path());
        let listener = listen(directory.path(), &entry).expect("绑定套接字");

        assert_eq!(read_entry(directory.path()), Some(socket.clone()));
        assert!(UnixStream::connect(&socket).is_ok(), "客户端可以连接");

        drop(listener);
        release(directory.path(), &entry);
        assert!(!entry.exists(), "退出后入口必须清理");
        assert!(!socket.exists(), "退出后套接字必须清理");
    }

    #[test]
    fn entry_survives_a_data_directory_deeper_than_the_socket_limit() {
        // 数据目录比套接字上限更深时, 客户端仍能经入口文件连上.
        let base = crate::tests::TempDir::new("ipc-deep");
        let deep = base
            .path()
            .join("a".repeat(40))
            .join("b".repeat(30))
            .join("c".repeat(20));
        let entry = entry_path(&deep);
        let listener = listen(&deep, &entry).expect("绑定套接字");
        assert!(entry.as_os_str().len() > 108, "入口路径本身可以很长");

        // 请求要有人应答, 否则客户端只能等到超时.
        let app = test_service();
        let clients = Arc::new(Clients::default());
        accept_all(app, Arc::clone(&clients), listener);

        let mut client = Client::connect(&deep).expect("连接").expect("服务在线");
        let status = client.call("game_status", json!({}), |_| {}).expect("查询");
        assert_eq!(status["running"], json!(false));

        release(&deep, &entry);
    }

    #[test]
    fn broadcasting_without_any_connection_reports_an_error() {
        // 与 stdio 模式的"发送端已关闭"对齐: 调用方据此知道没有接收方.
        let clients = Clients::default();
        assert!(clients.broadcast(json!({"event": "host-show"})).is_err());
    }

    #[test]
    fn query_reports_a_missing_service_instead_of_failing() {
        let directory = crate::tests::TempDir::new("ipc-missing");
        assert_eq!(
            query(directory.path(), "game_status", json!({})).unwrap(),
            None
        );
        assert!(Client::connect(directory.path()).expect("连接").is_none());
    }

    #[test]
    fn query_returns_results_and_surfaces_errors() {
        let app = test_service();
        let clients = Arc::new(Clients::default());
        let directory = crate::tests::TempDir::new("ipc-query");
        let (listener, _socket) = listener(directory.path());
        accept_all(app, Arc::clone(&clients), listener);

        let status = query(directory.path(), "game_status", json!({}))
            .expect("查询成功")
            .expect("服务在线");
        assert_eq!(status["running"], json!(false));
        assert!(query(directory.path(), "unregistered", json!({})).is_err());
    }

    #[test]
    fn client_reuses_one_connection_and_reports_events_in_order() {
        let app = test_service();
        let clients = Arc::new(Clients::default());
        let directory = crate::tests::TempDir::new("ipc-client");
        let (listener, _socket) = listener(directory.path());
        accept_all(app, Arc::clone(&clients), listener);

        let mut client = Client::connect(directory.path())
            .expect("连接")
            .expect("服务在线");
        let mut events = Vec::new();
        for _ in 0..2 {
            let status = client
                .call("game_status", json!({}), |event| events.push(event))
                .expect("查询");
            assert_eq!(status["running"], json!(false));
        }
        let _ = events;
    }
}
