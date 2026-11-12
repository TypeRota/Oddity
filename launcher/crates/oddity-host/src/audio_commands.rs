//! 音频连接独占后台线程, 界面和游戏监控只传递已确定的偏好与 PID.
use oddity_platform::audio::{Client, Options, Status};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

enum Update {
    Preferences(Options),
    Processes(Vec<u32>),
}
pub struct Audio {
    input: mpsc::Sender<Update>,
    status: Arc<Mutex<Status>>,
}
impl Default for Audio {
    fn default() -> Self {
        let (input, receiver) = mpsc::channel();
        let status = Arc::new(Mutex::new(Status::default()));
        let output = status.clone();
        std::thread::spawn(move || {
            let mut client: Option<Client> = None;
            let mut options = Options::default();
            let mut pids = Vec::new();
            let mut retry = Instant::now();
            loop {
                match receiver.recv_timeout(Duration::from_millis(500)) {
                    Ok(Update::Preferences(next)) => options = next,
                    Ok(Update::Processes(next)) => pids = next,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                if client.is_none() && Instant::now() < retry {
                    continue;
                }
                let result = match client.as_mut() {
                    Some(client) => client.apply(&options, &pids),
                    None => Client::connect().and_then(|mut connected| {
                        let result = connected.apply(&options, &pids);
                        client = Some(connected);
                        result
                    }),
                };
                let snapshot = match result {
                    Ok(status) => status,
                    Err(error) => {
                        client = None;
                        retry = Instant::now() + Duration::from_secs(3);
                        Status {
                            error: Some(error),
                            ..Default::default()
                        }
                    }
                };
                if let Ok(mut status) = output.lock() {
                    *status = snapshot;
                }
            }
        });
        Self { input, status }
    }
}
impl Audio {
    pub fn preferences(&self, options: Options) {
        let _ = self.input.send(Update::Preferences(options));
    }
    pub fn processes(&self, pids: Vec<u32>) {
        let _ = self.input.send(Update::Processes(pids));
    }
    pub fn status(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }
}
