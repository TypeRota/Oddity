use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use super::protocol::Decoder;

pub(super) const MISSING_DUMPCAP: &str = "未安装 dumpcap, 将在安装后自动重试.";
const CAPTURE_ERROR: &str = "无法建立抓包输出管道, 正在重试.";

fn is_game(pid: u32, install_root: Option<&str>) -> bool {
    let directory = format!("/proc/{pid}");
    if !fs::metadata(&directory).is_ok_and(|metadata| metadata.uid() == unsafe { libc::getuid() }) {
        return false;
    }
    let Ok(raw) = fs::read(format!("{directory}/cmdline")) else {
        return false;
    };
    let command = String::from_utf8_lossy(raw.split(|byte| *byte == 0).next().unwrap_or_default())
        .trim_matches('"')
        .replace('\\', "/")
        .to_lowercase();
    command.ends_with("/htgame.exe")
        && install_root.is_none_or(|root| command.contains(&root.replace('\\', "/").to_lowercase()))
}

fn find_game(install_root: Option<&str>) -> Option<u32> {
    fs::read_dir("/proc")
        .ok()?
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .find(|pid| is_game(*pid, install_root))
}

fn ports(pid: u32) -> HashSet<u16> {
    let inodes: HashSet<String> = fs::read_dir(format!("/proc/{pid}/fd"))
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| fs::read_link(entry.path()).ok())
        .filter_map(|target| {
            target
                .to_str()?
                .strip_prefix("socket:[")?
                .strip_suffix(']')
                .map(str::to_owned)
        })
        .collect();
    let Ok(table) = fs::read_to_string("/proc/net/udp") else {
        return HashSet::new();
    };
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let values: Vec<_> = line.split_ascii_whitespace().collect();
            if !inodes.contains(*values.get(9)?) {
                return None;
            }
            u16::from_str_radix(values.get(1)?.rsplit_once(':')?.1, 16).ok()
        })
        .collect()
}

struct Producer {
    child: Child,
    output: Option<std::process::ChildStdout>,
}

impl Producer {
    fn start(dumpcap: &std::path::Path) -> Result<Self, String> {
        // 管道输出关闭缓冲, 让稀疏的角色更新及时交给解析器.
        let mut command = if let Some(stdbuf) = oddity_platform::process::find_executable("stdbuf")
        {
            let mut command = Command::new(stdbuf);
            command.arg("-o0").arg(dumpcap);
            command
        } else {
            Command::new(dumpcap)
        };
        let mut child = command
            .args([
                "-i",
                "any",
                "-p",
                "-s",
                "0",
                "-q",
                "-F",
                "pcap",
                "-f",
                "ip and udp",
                "-w",
                "-",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("无法启动抓包组件: {error}"))?;
        let output = child.stdout.take().expect("抓包输出管道已创建");
        for descriptor in [
            output.as_raw_fd(),
            child
                .stderr
                .as_ref()
                .expect("抓包错误管道已创建")
                .as_raw_fd(),
        ] {
            let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
            if flags < 0
                || unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
            {
                let _ = child.kill();
                let _ = child.wait();
                return Err(CAPTURE_ERROR.into());
            }
        }
        Ok(Self {
            child,
            output: Some(output),
        })
    }

    fn failure(&mut self) -> String {
        let mut bytes = [0; 4096];
        let count = self
            .child
            .stderr
            .as_mut()
            .and_then(|stderr| stderr.read(&mut bytes).ok())
            .unwrap_or(0);
        let message = String::from_utf8_lossy(&bytes[..count]).to_lowercase();
        if message.contains("permission denied") || message.contains("operation not permitted") {
            return "当前用户没有抓包权限, 配置权限后将自动重试.".into();
        }
        match self
            .child
            .try_wait()
            .ok()
            .flatten()
            .and_then(|status| status.code())
        {
            Some(code) => format!("抓包进程已退出 (退出码 {code}), 正在重试."),
            None => "抓包进程输出已断开, 正在重试.".into(),
        }
    }
}

impl Drop for Producer {
    fn drop(&mut self) {
        self.output.take();
        unsafe { libc::kill(self.child.id() as i32, libc::SIGINT) };
        for _ in 0..10 {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(super) fn run(
    stop: &AtomicBool,
    install_root: Option<&str>,
    executable: Option<&std::path::Path>,
) {
    while !stop.load(Ordering::Acquire) {
        let dumpcap = executable
            .map(std::path::Path::to_path_buf)
            .or_else(|| oddity_platform::process::find_executable("dumpcap"));
        let result = match dumpcap {
            Some(dumpcap) => collect(stop, install_root, &dumpcap),
            None => Err(MISSING_DUMPCAP.into()),
        };
        let Err(message) = result else { return };
        if stop.load(Ordering::Acquire) {
            return;
        }
        {
            let mut manager = super::manager().lock().expect("角色同步状态锁中毒");
            manager.status.state = "error";
            manager.status.message = Some(message.clone());
            manager.status.capture.last_error = Some(message);
            manager.status.capture.retry_at = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64
                    + 5000,
            );
        }
        let retry = Instant::now();
        while !stop.load(Ordering::Acquire) && retry.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(100));
        }
    }
}

fn collect(
    stop: &AtomicBool,
    install_root: Option<&str>,
    dumpcap: &std::path::Path,
) -> Result<(), String> {
    while !stop.load(Ordering::Acquire) {
        let Some(pid) = find_game(install_root) else {
            super::change("waiting", Some("等待游戏启动."));
            thread::sleep(Duration::from_millis(250));
            continue;
        };
        super::change("starting", Some("正在开启同步."));
        let mut producer = Producer::start(dumpcap)?;
        let opened = Instant::now();
        let mut stream = Pcap::default();
        let mut owned = ports(pid);
        let mut refreshed = Instant::now();
        let mut decoders = HashMap::<(u32, u16, u16), Decoder>::new();
        let boot = fs::read_to_string("/proc/stat").ok().and_then(|table| {
            table
                .lines()
                .find_map(|line| line.strip_prefix("btime ")?.parse::<u64>().ok())
        });
        let started = fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|stat| {
                stat.rsplit_once(')')?
                    .1
                    .split_whitespace()
                    .nth(19)?
                    .parse::<u64>()
                    .ok()
            })
            .zip(boot)
            .map(|(ticks, boot)| boot + ticks / unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as u64);
        let mut current = {
            let mut manager = super::manager().lock().expect("角色同步状态锁中毒");
            let status = &mut manager.status;
            let last_error = status.capture.last_error.clone();
            status.capture = super::CaptureStatus {
                pid: Some(pid),
                ports: owned.len(),
                started_at: Some(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64,
                ),
                last_error,
                ..Default::default()
            };
            status
                .observation
                .clone()
                .filter(|record| started.is_some_and(|started| record.updated_at >= started))
                .or_else(|| {
                    status.record.clone().filter(|record| {
                        started.is_some_and(|started| record.updated_at >= started)
                    })
                })
        };
        let mut chunk = [0u8; 65536];
        while !stop.load(Ordering::Acquire) {
            if stream.link.is_none() && opened.elapsed() >= Duration::from_secs(5) {
                return Err("抓包组件未返回数据头, 正在重试.".into());
            }
            if refreshed.elapsed() >= Duration::from_millis(500) {
                if !is_game(pid, install_root) {
                    break;
                }
                owned = ports(pid);
                super::manager()
                    .lock()
                    .expect("角色同步状态锁中毒")
                    .status
                    .capture
                    .ports = owned.len();
                refreshed = Instant::now();
            }
            let output = producer.output.as_mut().expect("抓包输出管道存在");
            let mut poll = libc::pollfd {
                fd: output.as_raw_fd(),
                events: libc::POLLIN | libc::POLLHUP,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut poll, 1, 100) };
            if ready < 0 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(format!("抓包管道读取失败: {error}"));
                }
                continue;
            }
            if ready == 0 {
                continue;
            }
            match output.read(&mut chunk) {
                Ok(0) => return Err(producer.failure()),
                Ok(count) => {
                    let had_header = stream.link.is_some();
                    stream.push(&chunk[..count], &mut |link, frame| {
                        let Some(packet) = udp(link, frame) else {
                            return;
                        };
                        if !owned.contains(&packet.destination_port) {
                            owned = ports(pid);
                            if !owned.contains(&packet.destination_port) {
                                return;
                            }
                        }
                        {
                            let mut manager = super::manager().lock().expect("角色同步状态锁中毒");
                            manager.status.capture.packets += 1;
                            manager.status.capture.last_packet_at = Some(
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_millis() as u64,
                            );
                        }
                        let decoder = decoders
                            .entry((packet.source, packet.source_port, packet.destination_port))
                            .or_insert_with(|| Decoder::with_record(current.clone()));
                        for record in decoder.ingest(packet.payload) {
                            current = Some(record.clone());
                            super::install_record(record, &super::cache_file());
                        }
                    })?;
                    if !had_header && stream.link.is_some() && super::status().state == "starting" {
                        super::change("reading", Some("正在监听游戏内状态变化."));
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => return Err("读取游戏同步数据失败.".into()),
            }
        }
        drop(producer);
    }
    Ok(())
}

#[derive(Default)]
struct Pcap {
    buffer: Vec<u8>,
    link: Option<u32>,
    big_endian: bool,
}

impl Pcap {
    fn push(&mut self, bytes: &[u8], receive: &mut impl FnMut(u32, &[u8])) -> Result<(), String> {
        self.buffer.extend_from_slice(bytes);
        let mut position = 0;
        if self.link.is_none() {
            if self.buffer.len() < 24 {
                return Ok(());
            }
            self.big_endian = match &self.buffer[..4] {
                [0xd4, 0xc3, 0xb2, 0xa1] | [0x4d, 0x3c, 0xb2, 0xa1] => false,
                [0xa1, 0xb2, 0xc3, 0xd4] | [0xa1, 0xb2, 0x3c, 0x4d] => true,
                _ => return Err("抓包组件返回了无法识别的数据格式.".into()),
            };
            self.link = Some(self.word(20));
            if !matches!(self.link, Some(1 | 101 | 113 | 276)) {
                return Err("当前网络接口的抓包格式尚不支持.".into());
            }
            position = 24;
        }
        while self.buffer.len() >= position + 16 {
            let captured = self.word(position + 8) as usize;
            let original = self.word(position + 12) as usize;
            if captured > 131072 || captured > original {
                return Err("抓包组件返回了无效的数据长度.".into());
            }
            let end = position + 16 + captured;
            if end > self.buffer.len() {
                break;
            }
            if captured == original {
                receive(
                    self.link.expect("PCAP 头已读取"),
                    &self.buffer[position + 16..end],
                );
            }
            position = end;
        }
        self.buffer.drain(..position);
        Ok(())
    }

    fn word(&self, position: usize) -> u32 {
        let bytes = self.buffer[position..position + 4]
            .try_into()
            .expect("PCAP 字段边界已验证");
        if self.big_endian {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        }
    }
}

struct Udp<'a> {
    source: u32,
    source_port: u16,
    destination_port: u16,
    payload: &'a [u8],
}

fn udp(link: u32, frame: &[u8]) -> Option<Udp<'_>> {
    let skip = match link {
        1 if frame.get(12..14)? == [8, 0] => 14,
        101 => 0,
        113 if frame.get(14..16)? == [8, 0] => 16,
        276 if frame.get(..2)? == [8, 0] => 20,
        _ => return None,
    };
    let ip = frame.get(skip..)?;
    if ip.len() < 20 || ip[0] >> 4 != 4 || ip[9] != 17 {
        return None;
    }
    let header = (ip[0] as usize & 15) * 4;
    let total = u16::from_be_bytes([ip[2], ip[3]]) as usize;
    let fragmented = u16::from_be_bytes([ip[6], ip[7]]) & 0x3fff != 0;
    if header < 20 || total < header + 8 || total > ip.len() || fragmented {
        return None;
    }
    let udp = &ip[header..total];
    let length = u16::from_be_bytes([udp[4], udp[5]]) as usize;
    if length < 8 || length > udp.len() {
        return None;
    }
    Some(Udp {
        source: u32::from_be_bytes(ip[12..16].try_into().ok()?),
        source_port: u16::from_be_bytes([udp[0], udp[1]]),
        destination_port: u16::from_be_bytes([udp[2], udp[3]]),
        payload: &udp[8..length],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Vec<u8> {
        let mut raw = vec![0; 20 + 8];
        raw[0] = 0x45;
        raw[2..4].copy_from_slice(&31u16.to_be_bytes());
        raw[9] = 17;
        raw[12..16].copy_from_slice(&[1, 2, 3, 4]);
        raw[20..22].copy_from_slice(&30100u16.to_be_bytes());
        raw[22..24].copy_from_slice(&50000u16.to_be_bytes());
        raw[24..26].copy_from_slice(&11u16.to_be_bytes());
        raw.extend_from_slice(&[1, 2, 3]);
        raw
    }

    #[test]
    fn udp_accepts_supported_links_and_rejects_fragments_and_lengths() {
        let raw = frame();
        for link in [1, 101, 113, 276] {
            let mut wrapped = vec![
                0;
                match link {
                    1 => 14,
                    113 => 16,
                    276 => 20,
                    _ => 0,
                }
            ];
            match link {
                1 => wrapped[12..14].copy_from_slice(&[8, 0]),
                113 => wrapped[14..16].copy_from_slice(&[8, 0]),
                276 => wrapped[..2].copy_from_slice(&[8, 0]),
                _ => {}
            }
            wrapped.extend_from_slice(&raw);
            let packet = udp(link, &wrapped).unwrap();
            assert_eq!(packet.source_port, 30100);
            assert_eq!(packet.destination_port, 50000);
            assert_eq!(packet.payload, [1, 2, 3]);
        }
        let mut bad = raw.clone();
        bad[6] = 0x20;
        assert!(udp(101, &bad).is_none());
        let mut bad = raw.clone();
        bad[3] = 200;
        assert!(udp(101, &bad).is_none());
        assert!(udp(101, &raw[..25]).is_none());
    }

    #[test]
    fn pcap_reads_split_records_and_validates_bounds() {
        let payload = frame();
        let mut bytes = vec![0xd4, 0xc3, 0xb2, 0xa1, 2, 0, 4, 0];
        bytes.extend_from_slice(&[0; 12]);
        bytes.extend_from_slice(&101u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 8]);
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&payload);
        let mut parser = Pcap::default();
        let mut received = Vec::new();
        for chunk in bytes.chunks(3) {
            parser
                .push(chunk, &mut |link, data| {
                    received.push((link, data.to_vec()))
                })
                .unwrap();
        }
        assert_eq!(received, [(101, payload)]);
        let mut broken = bytes;
        broken[32..36].copy_from_slice(&200000u32.to_le_bytes());
        assert!(Pcap::default().push(&broken, &mut |_, _| {}).is_err());
        assert!(Pcap::default().push(&[0; 24], &mut |_, _| {}).is_err());
    }

    #[test]
    fn dropping_producer_stops_and_reaps_the_capture_child() {
        let mut child = Command::new("/usr/bin/sleep")
            .arg("10")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id();
        let output = child.stdout.take();
        let started = Instant::now();
        drop(Producer { child, output });
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(
            unsafe { libc::waitpid(pid as i32, std::ptr::null_mut(), libc::WNOHANG) },
            -1
        );
    }

    #[test]
    fn capture_exit_retries_and_stopping_interrupts_the_retry_wait() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::Arc;
        let _test = super::super::tests::TEST_LOCK.lock().unwrap();
        let previous = super::super::manager().lock().unwrap().status.clone();
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("dumpcap");
        fs::write(&script, "#!/bin/sh\nprintf x >> \"$0.count\"\nprintf '\\324\\303\\262\\241\\002\\000\\004\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\145\\000\\000\\000'\nexit 42\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let mut game = Command::new("/bin/bash")
            .args(["-c", "exec -a \"$1/HTGame.exe\" /usr/bin/sleep 20", "_"])
            .arg(directory.path())
            .spawn()
            .unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let root = directory.path().to_string_lossy().into_owned();
        let worker = thread::spawn(move || run(&worker_stop, Some(&root), Some(&script)));
        let limit = Instant::now();
        let count = directory.path().join("dumpcap.count");
        while fs::read(&count).map_or(0, |value| value.len()) < 2
            && limit.elapsed() < Duration::from_secs(8)
        {
            thread::sleep(Duration::from_millis(20));
        }
        let retries = fs::read(&count).map_or(0, |value| value.len());
        let last_error = super::super::status().capture.last_error;
        let stopping = Instant::now();
        stop.store(true, Ordering::Release);
        worker.join().unwrap();
        let _ = game.kill();
        let _ = game.wait();
        super::super::manager().lock().unwrap().status = previous;
        assert!(retries >= 2);
        assert!(last_error.is_some());
        assert!(stopping.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn socket_ownership_refreshes_after_a_udp_socket_is_closed() {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        assert!(ports(std::process::id()).contains(&port));
        drop(socket);
        assert!(!ports(std::process::id()).contains(&port));
        assert!(ports(u32::MAX).is_empty());
        assert!(!is_game(std::process::id(), None));
    }

    #[test]
    fn pcaps_accept_both_byte_orders_and_reject_unsupported_links() {
        let payload = frame();
        for header in [
            [0xa1, 0xb2, 0xc3, 0xd4, 2, 0, 4, 0],
            [0xa1, 0xb2, 0x3c, 0x4d, 2, 0, 4, 0],
        ] {
            let mut bytes = header.to_vec();
            bytes.extend_from_slice(&[0; 12]);
            bytes.extend_from_slice(&276u32.to_be_bytes());
            bytes.extend_from_slice(&[0; 8]);
            bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            bytes.extend_from_slice(&payload);
            let mut parser = Pcap::default();
            let mut received = Vec::new();
            parser
                .push(&bytes, &mut |link, data| {
                    received.push((link, data.to_vec()))
                })
                .unwrap();
            assert_eq!(received, [(276, payload.clone())]);
        }
        let mut unsupported = vec![0xd4, 0xc3, 0xb2, 0xa1, 2, 0, 4, 0];
        unsupported.extend_from_slice(&[0; 12]);
        unsupported.extend_from_slice(&999u32.to_le_bytes());
        assert!(Pcap::default().push(&unsupported, &mut |_, _| {}).is_err());
        let mut long = vec![0xd4, 0xc3, 0xb2, 0xa1, 2, 0, 4, 0];
        long.extend_from_slice(&[0; 12]);
        long.extend_from_slice(&101u32.to_le_bytes());
        long.extend_from_slice(&[0; 8]);
        long.extend_from_slice(&500u32.to_le_bytes());
        long.extend_from_slice(&100u32.to_le_bytes());
        assert!(Pcap::default().push(&long, &mut |_, _| {}).is_err());
    }

    #[test]
    fn game_lookup_rejects_processes_outside_the_install_root() {
        assert_eq!(find_game(Some("/nonexistent-install-root")), None);
        assert!(!is_game(
            std::process::id(),
            Some("/definitely-not-this-process")
        ));
        assert!(ports(u32::MAX).is_empty());
    }
}
