//! 子进程与 /proc 原语: 进程组信号, 快照, 固定身份的句柄, 环境变量标记匹配.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 单进程命令行读取上限.
const CMDLINE_LIMIT: u64 = 64 * 1024;
/// 单进程环境变量读取上限, 防止超大 environ 拖垮扫描.
const ENVIRON_LIMIT: u64 = 1024 * 1024;

/// 结束进程组时等待 SIGTERM 生效的时间.
pub const TERMINATE_GRACE: Duration = Duration::from_secs(8);
/// SIGKILL 之后等待进程组消失的时间.
pub const TERMINATE_FORCE: Duration = Duration::from_secs(3);

/// 在 PATH 中查找第一个可执行普通文件.
///
/// PATH 未设置或没有可执行文件时返回 None.
pub fn find_executable(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|path| path.join(name))
        .find(|path| {
            path.metadata().is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
}

// ---------------------------------------------------------------------------
// 进程组
// ---------------------------------------------------------------------------

/// 进程组是否仍有成员.
///
/// 僵尸成员算存活, 直到监控线程回收组长: 回收前整个进程组不应视为已退出.
pub fn group_alive(pgid: i32) -> bool {
    if pgid <= 0 {
        return false;
    }
    let result = unsafe { libc::kill(-pgid, 0) };
    if result == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

/// 向进程组发送信号; 组已经消失时视为成功.
pub fn signal_group(pgid: i32, signal: libc::c_int) -> Result<(), String> {
    if pgid <= 0 {
        return Ok(());
    }
    let result = unsafe { libc::kill(-pgid, signal) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        // 存活检查与发信号之间组刚好消失.
        return Ok(());
    }
    Err(format!("向游戏进程组发送信号失败：{error}"))
}

/// 先请整个进程组退出, 再强杀剩余成员.
///
/// 监控线程需要并发回收组长, 进程组才可能真正消失.
pub fn terminate_group(pgid: i32) -> Result<(), String> {
    terminate_group_with(pgid, TERMINATE_GRACE, TERMINATE_FORCE)
}

/// 使用自定义等待时长的 terminate_group.
pub fn terminate_group_with(pgid: i32, grace: Duration, force: Duration) -> Result<(), String> {
    if !group_alive(pgid) {
        return Ok(());
    }
    signal_group(pgid, libc::SIGTERM)?;
    let graceful_deadline = std::time::Instant::now() + grace;
    while std::time::Instant::now() < graceful_deadline {
        if !group_alive(pgid) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(120));
    }

    signal_group(pgid, libc::SIGKILL)?;
    let forced_deadline = std::time::Instant::now() + force;
    while std::time::Instant::now() < forced_deadline {
        if !group_alive(pgid) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(80));
    }
    Err("游戏进程未响应结束请求，请稍后重试。".to_string())
}

// ---------------------------------------------------------------------------
// /proc 快照
// ---------------------------------------------------------------------------

/// /proc/<pid> 快照中的一项.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub ppid: u32,
    pub pgid: u32,
    /// 进程状态字符, 见 proc(5); Z 是僵尸.
    pub state: char,
    /// 自开机起的启动时间 (clock ticks), 用于区分 pid 复用.
    pub start_time: u64,
    /// /proc/<pid>/comm, 可能带结尾换行.
    pub comm: String,
    /// /proc/<pid>/cmdline, 按 NUL 切分; 空命令行时为空.
    pub cmdline: Vec<String>,
}

impl ProcessInfo {
    /// 僵尸进程不再算运行中的进程.
    pub fn is_zombie(&self) -> bool {
        self.state == 'Z'
    }
}

/// 读取一次 /proc 快照.
///
/// 单个进程消失或不可读时跳过; 只有目录本身不可读才返回错误.
pub fn snapshot() -> io::Result<Vec<ProcessInfo>> {
    let mut processes = Vec::new();
    for entry in fs::read_dir("/proc")? {
        let Ok(entry) = entry else {
            continue;
        };
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        if let Some(info) = info(pid) {
            processes.push(info);
        }
    }
    Ok(processes)
}

/// 读取单个进程的当前快照; 进程已消失或 stat 不可读时为 None.
pub fn info(pid: u32) -> Option<ProcessInfo> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, rest) = stat.rsplit_once(')')?;
    let fields: Vec<_> = rest.split_whitespace().collect();
    let state = fields.first()?.chars().next()?;
    let ppid = fields.get(1)?.parse().ok()?;
    let pgid = fields.get(2)?.parse().ok()?;
    let start_time = fields.get(19)?.parse().ok()?;
    Some(ProcessInfo {
        pid,
        ppid,
        pgid,
        state,
        start_time,
        comm: fs::read(format!("/proc/{pid}/comm"))
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default(),
        cmdline: read_cmdline(pid),
    })
}

fn read_cmdline(pid: u32) -> Vec<String> {
    let Ok(file) = File::open(format!("/proc/{pid}/cmdline")) else {
        return Vec::new();
    };
    let mut bytes = Vec::new();
    if file.take(CMDLINE_LIMIT).read_to_end(&mut bytes).is_err() || bytes.is_empty() {
        return Vec::new();
    }
    bytes
        .split(|byte| *byte == 0)
        .map(|argument| String::from_utf8_lossy(argument).into_owned())
        .collect()
}

/// 按进程树过滤快照: 根进程本身, 其全部后代, 以及根进程组的成员 (含各自后代).
///
/// 调用方通常就是组长, 即 root 同时是 pid 与 pgid.
pub fn process_tree(processes: &[ProcessInfo], root: u32) -> Vec<&ProcessInfo> {
    if root == 0 {
        return Vec::new();
    }
    let mut by_pid: HashMap<u32, &ProcessInfo> = HashMap::with_capacity(processes.len());
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut stack = vec![root];
    for process in processes {
        by_pid.insert(process.pid, process);
        children.entry(process.ppid).or_default().push(process.pid);
        if process.pgid == root {
            stack.push(process.pid);
        }
    }
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    while let Some(pid) = stack.pop() {
        if !seen.insert(pid) {
            continue;
        }
        if let Some(process) = by_pid.get(&pid) {
            result.push(*process);
        }
        if let Some(descendants) = children.get(&pid) {
            stack.extend(descendants);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// 进程身份
// ---------------------------------------------------------------------------

/// 固定身份的本用户进程句柄.
///
/// 优先用 pidfd 锁定进程身份; 内核不支持 pidfd 时回退到 pid + 启动时间,
/// 每次操作前重新核对, 避免 pid 复用后误伤别的进程.
#[derive(Debug)]
pub struct ProcessHandle {
    pid: libc::pid_t,
    target: HandleTarget,
}

#[derive(Debug)]
enum HandleTarget {
    PidFd(OwnedFd),
    Pid { start_time: u64 },
}

impl ProcessHandle {
    /// 打开进程句柄; 进程已经不存在时返回 None.
    pub fn open(pid: libc::pid_t) -> Result<Option<Self>, String> {
        if pid <= 0 {
            return Ok(None);
        }
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if raw >= 0 {
            let fd = unsafe { OwnedFd::from_raw_fd(raw as i32) };
            return Ok(Some(Self {
                pid,
                target: HandleTarget::PidFd(fd),
            }));
        }
        let error = std::io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::ESRCH) | Some(libc::ENOENT) => Ok(None),
            // 老内核没有 pidfd, 退回 pid + 启动时间核对身份.
            Some(libc::ENOSYS) => Self::open_by_pid(pid),
            _ => Err(format!("固定后台会话进程身份失败: {error}")),
        }
    }

    /// pidfd 不可用时的回退: 用启动时间固定身份.
    fn open_by_pid(pid: libc::pid_t) -> Result<Option<Self>, String> {
        match info(pid as u32) {
            Some(info) if !info.is_zombie() => Ok(Some(Self {
                pid,
                target: HandleTarget::Pid {
                    start_time: info.start_time,
                },
            })),
            _ => Ok(None),
        }
    }

    /// 句柄对应的进程 id.
    pub fn pid(&self) -> libc::pid_t {
        self.pid
    }

    /// 进程是否仍存活; 僵尸进程不算存活.
    pub fn alive(&self) -> bool {
        match &self.target {
            HandleTarget::PidFd(fd) => {
                let mut descriptor = libc::pollfd {
                    fd: fd.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                // 被信号中断时保守地视为仍存活, 避免把活进程误报为已清理.
                unsafe { libc::poll(&mut descriptor, 1, 0) <= 0 }
            }
            HandleTarget::Pid { .. } => self.matches(),
        }
    }

    /// 是否仍指向打开时确认的那个进程.
    pub fn matches(&self) -> bool {
        match &self.target {
            HandleTarget::PidFd(_) => self.alive(),
            HandleTarget::Pid { start_time } => info(self.pid as u32)
                .is_some_and(|info| !info.is_zombie() && info.start_time == *start_time),
        }
    }

    /// 向句柄对应的进程发信号; 进程已经消失时视为成功.
    pub fn signal(&self, signal: libc::c_int) -> Result<(), String> {
        match &self.target {
            HandleTarget::PidFd(fd) => {
                let result = unsafe {
                    libc::syscall(
                        libc::SYS_pidfd_send_signal,
                        fd.as_raw_fd(),
                        signal,
                        std::ptr::null::<libc::siginfo_t>(),
                        0,
                    )
                };
                if result >= 0
                    || std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
                {
                    Ok(())
                } else {
                    Err(format!(
                        "结束本轮后台会话进程失败: {}",
                        std::io::Error::last_os_error()
                    ))
                }
            }
            HandleTarget::Pid { .. } => {
                if unsafe { libc::kill(self.pid, signal) } == 0 {
                    return Ok(());
                }
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::ESRCH) {
                    Ok(())
                } else {
                    Err(format!("结束本轮后台会话进程失败: {error}"))
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 环境标记集合
// ---------------------------------------------------------------------------

/// 按环境变量标记圈定的一组进程.
///
/// 标记匹配只逐块比较 KEY=VALUE, 不保留也不输出其余环境变量.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSet {
    key: String,
    value: String,
}

impl ProcessSet {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }

    /// 该进程是否属于本集合: 当前用户所有, 且环境变量带标记.
    pub fn contains(&self, pid: libc::pid_t) -> bool {
        if pid <= 0 {
            return false;
        }
        if !owned_by_current_user(pid) {
            return false;
        }
        environment_matches(
            Path::new(&format!("/proc/{pid}/environ")),
            &self.key,
            &self.value,
        )
    }

    /// 扫描 /proc, 返回带标记且被 classify 认出的进程.
    ///
    /// 当前进程不会包含在结果里; 只有先被 classify 认出的进程才会固定身份,
    /// 打开句柄后再核对一次标记与类别, 期间退出的进程会被跳过.
    pub fn scoped<K: PartialEq>(
        &self,
        classify: impl Fn(&ProcessInfo) -> Option<K>,
    ) -> Result<Vec<ScopedProcess<K>>, String> {
        let processes = snapshot().map_err(|error| format!("检查后台会话进程失败: {error}"))?;
        let own_pid = std::process::id();
        let mut result = Vec::new();
        for process in &processes {
            if process.pid == own_pid {
                continue;
            }
            if !owned_by_current_user(process.pid as libc::pid_t) {
                continue;
            }
            let Some(kind) = classify(process) else {
                continue;
            };
            if !environment_matches(
                Path::new(&format!("/proc/{}/environ", process.pid)),
                &self.key,
                &self.value,
            ) {
                continue;
            }
            let Some(handle) = ProcessHandle::open(process.pid as libc::pid_t)? else {
                continue;
            };
            if !handle.alive() {
                continue;
            }
            // 打开句柄后重新确认身份: 期间进程可能已退出, 也可能换了环境标记.
            if !self.contains(process.pid as libc::pid_t) {
                continue;
            }
            let Some(current) = info(process.pid) else {
                continue;
            };
            if classify(&current).as_ref() != Some(&kind) {
                continue;
            }
            result.push(ScopedProcess {
                pid: process.pid as libc::pid_t,
                kind,
                handle,
                set: self.clone(),
            });
        }
        Ok(result)
    }
}

/// 已固定身份的会话进程.
#[derive(Debug)]
pub struct ScopedProcess<K> {
    pid: libc::pid_t,
    kind: K,
    handle: ProcessHandle,
    set: ProcessSet,
}

impl<K: PartialEq> ScopedProcess<K> {
    /// 进程 id.
    pub fn pid(&self) -> u32 {
        self.pid as u32
    }

    /// 首次识别出的类别.
    pub fn kind(&self) -> &K {
        &self.kind
    }

    /// 仍存活, 且环境标记与类别未变.
    pub fn matches(&self, classify: impl Fn(&ProcessInfo) -> Option<K>) -> Result<bool, String> {
        if !self.handle.matches() {
            return Ok(false);
        }
        if !self.set.contains(self.pid) {
            return Ok(false);
        }
        Ok(info(self.pid as u32)
            .and_then(|info| classify(&info))
            .is_some_and(|kind| kind == self.kind))
    }

    /// 向该进程发信号; 进程已经消失时视为成功.
    pub fn signal(&self, signal: libc::c_int) -> Result<(), String> {
        self.handle.signal(signal)
    }
}

fn owned_by_current_user(pid: libc::pid_t) -> bool {
    fs::metadata(format!("/proc/{pid}"))
        .ok()
        .is_some_and(|metadata| metadata.uid() == unsafe { libc::geteuid() })
}

/// 读取块用完清零, 避免环境变量内容留在内存里.
struct WipeBuffer([u8; 4096]);

impl Drop for WipeBuffer {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

/// 读取 environ 文件, 判断是否带有 KEY=VALUE 标记.
///
/// 逐块比较而不保留内容; 读取失败或超过 1 MiB 时视为不匹配.
pub fn environment_matches(path: &Path, key: &str, value: &str) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    let expected = format!("{key}={value}");
    let expected = expected.as_bytes();
    let mut reader = file.take(ENVIRON_LIMIT);
    let mut buffer = WipeBuffer([0_u8; 4096]);
    let mut index = 0;
    let mut matching = true;
    // 只比较本轮标记, 不保留或输出其余环境变量.
    loop {
        let Ok(count) = reader.read(&mut buffer.0) else {
            return false;
        };
        if count == 0 {
            return false;
        }
        for byte in &buffer.0[..count] {
            if *byte == 0 {
                if matching && index == expected.len() {
                    return true;
                }
                index = 0;
                matching = true;
            } else {
                matching &= expected.get(index) == Some(byte);
                index += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{write_script, EnvGuard, TempDir};
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command, Stdio};
    use std::time::Instant;

    #[test]
    fn only_executable_files_on_path_are_found() {
        let mut env = EnvGuard::new();
        let dir = TempDir::new("find-executable");
        let script = write_script(dir.path(), "umu-run", "#!/bin/sh\nexit 0\n");
        dir.write("plain-file", b"x");
        std::fs::create_dir_all(dir.join("a-directory")).unwrap();

        env.set("PATH", dir.path());
        assert_eq!(find_executable("umu-run"), Some(script));
        assert_eq!(find_executable("plain-file"), None);
        assert_eq!(find_executable("a-directory"), None);
        assert_eq!(find_executable("missing"), None);

        env.remove("PATH");
        assert_eq!(find_executable("umu-run"), None);
    }

    fn spawn_sleep() -> Child {
        Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .spawn()
            .expect("spawn sleep")
    }

    fn spawn_group_sleep() -> Child {
        Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .process_group(0)
            .spawn()
            .expect("spawn sleep")
    }

    fn wait_until(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !condition() {
            assert!(Instant::now() < deadline, "condition timed out");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// 等子进程的 cmdline 可读.
    ///
    /// spawn 之后立刻读 /proc 可能碰上 exec 还没落完, 偶发拿到空命令行;
    /// 轮询到参数出现再让用例断言. 超时打印 /proc 诊断, 方便区分进程没起来和命令行读不到.
    fn wait_for_sleep_command_line(pid: u32) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if info(pid)
                .is_some_and(|process| process.cmdline.iter().any(|argument| argument == "sleep"))
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!(
            "子进程 {pid} 的 cmdline 未在超时内出现 sleep\n{}",
            process_diagnostics(pid)
        );
    }

    fn process_diagnostics(pid: u32) -> String {
        ["stat", "cmdline", "comm"]
            .into_iter()
            .map(|name| {
                let path = format!("/proc/{pid}/{name}");
                let content = match fs::read(&path) {
                    // cmdline 以 NUL 分隔, 换成空格后按参数连起来看.
                    Ok(bytes) => String::from_utf8_lossy(&bytes).replace('\0', " "),
                    Err(error) => format!("<读取失败: {error}>"),
                };
                format!("{path} = {content:?}")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn process_diagnostics_include_proc_files_and_read_failures() {
        let pid = std::process::id();
        let diagnostics = process_diagnostics(pid);
        for name in ["stat", "cmdline", "comm"] {
            assert!(
                diagnostics.contains(&format!("/proc/{pid}/{name} = ")),
                "{diagnostics}"
            );
        }
        assert!(!diagnostics.contains("读取失败"), "{diagnostics}");

        // 进程消失时也要留下读取失败的信息, 而不是让诊断一起失败.
        let missing = process_diagnostics(u32::MAX);
        assert_eq!(missing.matches("<读取失败:").count(), 3, "{missing}");
    }

    #[test]
    fn rejects_non_process_groups() {
        assert!(!group_alive(0));
        assert!(!group_alive(-1));
        assert_eq!(signal_group(0, libc::SIGTERM), Ok(()));
        assert_eq!(signal_group(-1, libc::SIGTERM), Ok(()));
        terminate_group(0).expect("nothing to terminate");
    }

    #[test]
    fn signals_and_terminates_a_live_process_group() {
        // PATH 会被别的用例改写, 启动外部命令前先拿到环境锁.
        let _env = EnvGuard::new();
        let mut child = spawn_group_sleep();
        let pgid = i32::try_from(child.id()).expect("pid fits i32");
        assert!(group_alive(pgid));

        signal_group(pgid, libc::SIGTERM).expect("signal group");
        let _ = child.wait();

        // 组长被回收后进程组也不该再被认为存活
        wait_until(|| !group_alive(pgid));
        // 已经消失的进程组再发信号仍然算成功
        signal_group(pgid, libc::SIGTERM).expect("signal dead group");
        terminate_group(pgid).expect("terminate dead group");
    }

    #[test]
    fn terminates_a_process_group_and_lets_the_leader_reap() {
        // PATH 会被别的用例改写, 启动外部命令前先拿到环境锁.
        let _env = EnvGuard::new();
        let mut child = spawn_group_sleep();
        let pgid = i32::try_from(child.id()).expect("pid fits i32");
        assert!(group_alive(pgid));

        // 应用里由监控线程回收组长; 没有并发的 wait, 组长会一直是僵尸进程,
        // 整个进程组也就永远不会消失.
        let waiter = std::thread::spawn(move || child.wait());
        terminate_group(pgid).expect("terminate group");
        let _ = waiter.join().expect("join waiter").expect("wait for child");
        wait_until(|| !group_alive(pgid));
    }

    #[test]
    fn terminates_a_group_with_members_beyond_the_leader() {
        // PATH 会被别的用例改写, 启动外部命令前先拿到环境锁.
        let _env = EnvGuard::new();
        // 用 shell 起一个后台子进程: SIGTERM 只发给组长时组内还有成员
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("sleep 30 & wait")
            .stdin(Stdio::null())
            .process_group(0)
            .spawn()
            .expect("spawn shell");
        let pgid = i32::try_from(child.id()).expect("pid fits i32");
        let waiter = std::thread::spawn(move || child.wait());
        terminate_group(pgid).expect("terminate group");
        let _ = waiter.join().expect("join waiter");
        wait_until(|| !group_alive(pgid));
    }

    #[test]
    fn terminate_group_escalates_to_sigkill_when_sigterm_is_ignored() {
        // PATH 会被别的用例改写, 启动外部命令前先拿到环境锁.
        let _env = EnvGuard::new();
        // 忽略 SIGTERM 的进程组只能强杀; 宽限时间用短值让用例尽快结束.
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("trap '' TERM; sleep 30 & wait")
            .stdin(Stdio::null())
            .process_group(0)
            .spawn()
            .expect("spawn shell");
        let pgid = i32::try_from(child.id()).expect("pid fits i32");
        let waiter = std::thread::spawn(move || child.wait());
        terminate_group_with(pgid, Duration::from_millis(200), Duration::from_secs(2))
            .expect("force kill group");
        let _ = waiter.join().expect("join waiter");
        wait_until(|| !group_alive(pgid));
    }

    fn synthetic(pid: u32, ppid: u32, pgid: u32) -> ProcessInfo {
        ProcessInfo {
            pid,
            ppid,
            pgid,
            state: 'S',
            start_time: u64::from(pid),
            comm: String::new(),
            cmdline: Vec::new(),
        }
    }

    #[test]
    fn snapshot_reports_stat_identity_and_command_line() {
        let _env = EnvGuard::new();
        let mut child = spawn_group_sleep();
        let pid = child.id();
        // 子进程刚 spawn 时 /proc 的 cmdline 可能短暂为空, 等它就绪再取快照.
        wait_for_sleep_command_line(pid);
        let processes = snapshot().expect("snapshot /proc");
        let entry = processes
            .iter()
            .find(|process| process.pid == pid)
            .expect("child appears in snapshot");
        assert_eq!(entry.ppid, std::process::id());
        assert_eq!(entry.pgid, pid);
        assert!(matches!(entry.state, 'S' | 'R'));
        assert!(!entry.is_zombie());
        assert_ne!(entry.start_time, 0);
        assert!(entry.cmdline.iter().any(|argument| argument == "sleep"));
        assert_eq!(info(pid).expect("single process info"), *entry);
        assert!(info(u32::MAX).is_none());

        child.kill().expect("kill child");
        child.wait().expect("reap child");
    }

    #[test]
    fn process_tree_walks_the_root_group_and_descendants() {
        let processes = [
            synthetic(10, 1, 10),
            synthetic(11, 10, 10),
            synthetic(12, 11, 10),
            // 同一进程组但与根进程不直接相连
            synthetic(20, 1, 10),
            // 组员的子进程
            synthetic(21, 20, 20),
            synthetic(30, 1, 30),
        ];
        let mut pids: Vec<_> = process_tree(&processes, 10)
            .iter()
            .map(|process| process.pid)
            .collect();
        pids.sort_unstable();
        assert_eq!(pids, vec![10, 11, 12, 20, 21]);
        assert!(process_tree(&processes, 99).is_empty());
        assert!(process_tree(&processes, 0).is_empty());
    }

    #[test]
    fn process_handle_tracks_identity_and_signals() {
        let _env = EnvGuard::new();
        let mut child = spawn_sleep();
        let pid = libc::pid_t::try_from(child.id()).expect("pid fits i32");
        let handle = ProcessHandle::open(pid)
            .expect("open handle")
            .expect("live process has a handle");
        assert_eq!(handle.pid(), pid);
        assert!(handle.alive());
        assert!(handle.matches());

        handle.signal(libc::SIGTERM).expect("signal handle");
        child.wait().expect("reap child");
        assert!(!handle.alive());
        assert!(!handle.matches());
        // 已经消失的进程再发信号仍然算成功
        handle.signal(libc::SIGTERM).expect("signal dead process");

        assert!(ProcessHandle::open(0).expect("open pid 0").is_none());
        assert!(ProcessHandle::open(-1).expect("open pid -1").is_none());
        assert!(ProcessHandle::open(libc::pid_t::MAX)
            .expect("open unused pid")
            .is_none());
    }

    #[test]
    fn process_handle_does_not_count_zombies_as_alive() {
        let _env = EnvGuard::new();
        let mut child = spawn_sleep();
        let pid = libc::pid_t::try_from(child.id()).expect("pid fits i32");
        let handle = ProcessHandle::open(pid)
            .expect("open handle")
            .expect("live process has a handle");
        child.kill().expect("kill child");
        // 尚未回收的组长是僵尸: 进程已经结束, 但进程组还没消失.
        wait_until(|| !handle.alive());
        assert!(!handle.matches());
        child.wait().expect("reap child");
    }

    #[test]
    fn pid_fallback_handle_verifies_start_time() {
        let _env = EnvGuard::new();
        let mut child = spawn_sleep();
        let pid = libc::pid_t::try_from(child.id()).expect("pid fits i32");
        let handle = ProcessHandle::open_by_pid(pid)
            .expect("open fallback handle")
            .expect("live process has a fallback handle");
        assert!(handle.alive());
        assert!(handle.matches());
        handle
            .signal(libc::SIGTERM)
            .expect("signal fallback handle");
        child.wait().expect("reap child");
        assert!(!handle.alive());
        assert!(!handle.matches());
        assert!(ProcessHandle::open_by_pid(libc::pid_t::MAX)
            .expect("open unused pid")
            .is_none());
    }

    #[test]
    fn environment_markers_match_whole_entries_across_chunks() {
        let dir = TempDir::new("process-environment");
        let path = dir.write("suffixed", b"OTHER=value\0MARKER=token-extra\0");
        assert!(!environment_matches(&path, "MARKER", "token"));
        let path = dir.write("prefixed", b"MARKER2=token\0MARKER=token\0");
        assert!(environment_matches(&path, "MARKER", "token"));
        let path = dir.write("exact", b"OTHER=value\0MARKER=token\0OTHER2=value\0");
        assert!(environment_matches(&path, "MARKER", "token"));
        assert!(!environment_matches(&path, "MARKER", "token2"));
        assert!(!environment_matches(&path, "OTHER", "value2"));
        assert!(!environment_matches(
            &dir.join("missing"),
            "MARKER",
            "token"
        ));

        // 标记跨过 4 KiB 读取块边界时仍按整体匹配
        let token = "t".repeat(10_000);
        let path = dir.write("chunked", format!("A=1\0MARKER={token}\0"));
        assert!(environment_matches(&path, "MARKER", &token));
    }

    #[test]
    fn process_set_contains_only_marked_processes_of_the_current_user() {
        let _env = EnvGuard::new();
        let mut marked = Command::new("sleep")
            .arg("30")
            .env("ODDITY_TEST_MARKER", "value")
            .stdin(Stdio::null())
            .spawn()
            .expect("spawn marked sleep");
        let mut other = Command::new("sleep")
            .arg("30")
            .env("ODDITY_TEST_MARKER", "other")
            .stdin(Stdio::null())
            .spawn()
            .expect("spawn other sleep");
        let set = ProcessSet::new("ODDITY_TEST_MARKER", "value");
        assert!(set.contains(libc::pid_t::try_from(marked.id()).expect("pid fits i32")));
        assert!(!set.contains(libc::pid_t::try_from(other.id()).expect("pid fits i32")));
        assert!(!set.contains(0));
        assert!(!set.contains(libc::pid_t::MAX));

        marked.kill().expect("kill marked");
        other.kill().expect("kill other");
        marked.wait().expect("reap marked");
        other.wait().expect("reap other");
    }

    #[test]
    fn scoped_processes_revalidate_marker_and_kind() {
        let _env = EnvGuard::new();
        let mut marked = Command::new("sleep")
            .arg("30")
            .env("ODDITY_SCOPED_MARKER", "value")
            .stdin(Stdio::null())
            .spawn()
            .expect("spawn marked sleep");
        let mut unmarked = spawn_sleep();
        let set = ProcessSet::new("ODDITY_SCOPED_MARKER", "value");
        let classify = |process: &ProcessInfo| {
            process
                .cmdline
                .iter()
                .any(|argument| argument == "sleep")
                .then_some("sleep")
        };
        let scoped = set.scoped(classify).expect("scan scoped processes");
        let found = scoped
            .iter()
            .find(|process| process.pid() == marked.id())
            .expect("marked sleep is scoped");
        assert_eq!(*found.kind(), "sleep");
        assert!(found.matches(classify).expect("marked process matches"));
        // 未带标记的同类进程与当前进程都不在集合里
        assert!(!scoped.iter().any(|process| process.pid() == unmarked.id()));
        assert!(!scoped
            .iter()
            .any(|process| process.pid() == std::process::id()));

        found.signal(libc::SIGKILL).expect("signal scoped process");
        marked.wait().expect("reap marked");
        assert!(!found.matches(classify).expect("exited process mismatch"));
        unmarked.kill().expect("kill unmarked");
        unmarked.wait().expect("reap unmarked");
    }
}
