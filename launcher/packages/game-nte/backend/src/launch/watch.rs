use oddity_platform::process;
use std::collections::HashMap;

/// 进程名是否指向游戏主进程。Windows 路径和 Wine 的 comm 都可能保留
/// HTGame 或 GameThread，其他参数里的同名文本不算。
fn is_game_process_name(text: &str) -> bool {
    let name = text
        .trim()
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(text)
        .to_ascii_lowercase();
    matches!(
        name.as_str(),
        "htgame" | "htgame.exe" | "gamethread" | "gamethread.exe"
    )
}

fn is_game_process(process: &process::ProcessInfo) -> bool {
    if process.is_zombie() {
        return false;
    }
    if is_game_process_name(&process.comm) {
        return true;
    }
    process
        .cmdline
        .iter()
        .any(|argument| is_game_process_name(argument))
}

/// 记录进程身份, 父进程退出或进程组变化后仍跟踪已发现的游戏。
pub struct GameProcessWatch {
    root: u32,
    pub known: HashMap<u32, u64>,
}

impl GameProcessWatch {
    pub fn new(root: u32) -> Self {
        Self {
            root,
            known: HashMap::new(),
        }
    }

    pub fn alive(&mut self) -> bool {
        if self.root == 0 {
            return false;
        }
        let Ok(processes) = process::snapshot() else {
            return true;
        };
        let by_pid: HashMap<_, _> = processes
            .iter()
            .map(|process| (process.pid, process))
            .collect();
        self.known.retain(|pid, started| {
            by_pid
                .get(pid)
                .is_some_and(|process| !process.is_zombie() && process.start_time == *started)
        });
        for process in process::process_tree(&processes, self.root) {
            if is_game_process(process) {
                self.known.insert(process.pid, process.start_time);
            }
        }
        !self.known.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::CommandExt;
    use std::process::Command;
    use std::thread;
    use std::time::{Duration, Instant};

    /// 轮询直到条件成立; CI 负载高时 /proc 首次可见会晚一些, 留足宽限.
    fn wait_until(mut condition: impl FnMut() -> bool, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if condition() {
                return true;
            }
            thread::sleep(Duration::from_millis(25));
        }
        condition()
    }

    fn game_main_process_alive(root_pid: u32) -> bool {
        GameProcessWatch::new(root_pid).alive()
    }

    #[test]
    fn matches_only_the_game_executable() {
        assert!(is_game_process_name("HTGame.exe"));
        assert!(is_game_process_name("htgame"));
        assert!(is_game_process_name(r"Z:\games\HTGame.exe"));
        assert!(is_game_process_name("GameThread"));
        assert!(!is_game_process_name("NTEGame.exe"));
        assert!(!is_game_process_name("exec -a HTGame.exe sleep 30"));
    }

    #[test]
    fn detects_the_game_main_process() {
        // 不经 shell: 直接执行拷贝成 HTGame.exe 的二进制, comm 在 dash/bash
        // 环境下都一致.
        let dir = TempDir::new("game-process");
        let sleep = oddity_platform::process::find_executable("sleep")
            .or_else(|| {
                let fallback = std::path::PathBuf::from("/usr/bin/sleep");
                fallback.is_file().then_some(fallback)
            })
            .expect("find sleep");
        let game = dir.join("HTGame.exe");
        std::fs::copy(&sleep, &game).expect("copy sleep");
        std::fs::set_permissions(&game, std::fs::Permissions::from_mode(0o755))
            .expect("make game executable");
        let mut child = Command::new(&game)
            .arg("30")
            .process_group(0)
            .spawn()
            .expect("spawn game process");
        let root = child.id();
        assert!(
            wait_until(|| game_main_process_alive(root), Duration::from_secs(10)),
            "游戏主进程未被识别: pid={root}, snapshot={:?}",
            oddity_platform::process::snapshot().map(|processes| processes
                .iter()
                .filter(|process| process.pid == root || process.ppid == root)
                .map(|process| (
                    process.pid,
                    process.ppid,
                    process.pgid,
                    process.state,
                    process.comm.clone()
                ))
                .collect::<Vec<_>>())
        );
        let waiter = thread::spawn(move || child.wait());
        oddity_platform::process::terminate_group(root as i32).expect("terminate game process");
        let _ = waiter.join();
        assert!(wait_until(
            || !game_main_process_alive(root),
            Duration::from_secs(10)
        ));
    }
}
