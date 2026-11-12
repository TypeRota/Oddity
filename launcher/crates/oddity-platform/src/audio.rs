//! 通过 PulseAudio 协议管理本轮游戏的播放流, 兼容 pipewire-pulse.
use libpulse_binding::{
    callbacks::ListResult,
    context::{Context, FlagSet, State},
    mainloop::standard::{IterateResult, Mainloop},
    operation::{Operation, State as OperationState},
    volume::{ChannelVolumes, Volume},
};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Options {
    pub device: Option<String>,
    pub volume: u16,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            device: None,
            volume: 100,
        }
    }
}
impl Options {
    /// 设备偏好保持不变, 不可用时临时采用系统默认输出.
    fn target<'a>(&'a self, status: &'a Status) -> Option<&'a String> {
        self.device
            .as_ref()
            .filter(|name| status.devices.iter().any(|d| &d.name == *name))
            .or(status.default_device.as_ref())
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.volume > 150 {
            return Err("游戏音量必须在 0% 至 150% 之间.".into());
        }
        if self
            .device
            .as_ref()
            .is_some_and(|name| name.is_empty() || name.contains('\0'))
        {
            return Err("音频设备标识无效.".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub devices: Vec<Device>,
    pub default_device: Option<String>,
    pub streams: Vec<Stream>,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Device {
    pub index: u32,
    pub name: String,
    pub label: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stream {
    pub index: u32,
    pub pid: Option<u32>,
    pub device: u32,
    pub volume: u16,
    pub muted: bool,
    #[serde(skip)]
    channels: ChannelVolumes,
}

/// 在创建播放流前声明独立目标, 让音效软件保留本轮游戏的路由控制权.
pub fn configure(command: &mut std::process::Command) -> Result<(), String> {
    let inherited = std::env::var("PULSE_PROP").unwrap_or_default();
    let mut properties = libpulse_binding::proplist::Proplist::new_from_string(&inherited)
        .ok_or("PULSE_PROP 音频属性格式无效.")?;
    // 未绑定物理设备的目标会先使用系统默认输出, 随后由会话控制器选择设备.
    properties
        .set_str("target.object", "oddity-managed-game")
        .expect("固定音频属性有效");
    command.env(
        "PULSE_PROP",
        properties.to_string().ok_or("无法生成游戏音频属性.")?,
    );
    Ok(())
}

/// libpulse 的上下文和事件循环始终留在创建它们的线程.
pub struct Client {
    context: Context,
    mainloop: Mainloop,
    applied: HashMap<u32, (String, u16)>,
}
fn wait<T: ?Sized>(mainloop: &mut Mainloop, mut operation: Operation<T>) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(2);
    while operation.get_state() == OperationState::Running {
        if Instant::now() >= deadline {
            operation.cancel();
            return Err("等待音频服务超时.".into());
        }
        if !matches!(mainloop.iterate(false), IterateResult::Success(_)) {
            return Err("音频服务连接中断.".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    if operation.get_state() == OperationState::Cancelled {
        return Err("音频操作已取消.".into());
    }
    Ok(())
}
impl Client {
    pub fn connect() -> Result<Self, String> {
        let mut mainloop = Mainloop::new().ok_or("无法创建音频事件循环.")?;
        let mut context = Context::new(&mainloop, "Oddity").ok_or("无法创建音频连接.")?;
        context
            .connect(None, FlagSet::NOAUTOSPAWN, None)
            .map_err(|e| format!("无法连接音频服务: {e}"))?;
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match context.get_state() {
                State::Ready => break,
                State::Failed | State::Terminated => return Err("系统音频服务不可用.".into()),
                _ if Instant::now() >= deadline => return Err("连接音频服务超时.".into()),
                _ => {
                    mainloop.iterate(false);
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        }
        Ok(Self {
            mainloop,
            context,
            applied: HashMap::new(),
        })
    }
    pub fn status(&mut self) -> Result<Status, String> {
        if self.context.get_state() != State::Ready {
            return Err("系统音频服务连接中断.".into());
        }
        let status = Rc::new(RefCell::new(Status::default()));
        let output = status.clone();
        wait(
            &mut self.mainloop,
            self.context.introspect().get_server_info(move |info| {
                output.borrow_mut().default_device =
                    info.default_sink_name.as_deref().map(str::to_owned);
            }),
        )?;
        let output = status.clone();
        let failed = Rc::new(RefCell::new(false));
        let error = failed.clone();
        wait(
            &mut self.mainloop,
            self.context
                .introspect()
                .get_sink_info_list(move |item| match item {
                    ListResult::Item(info) => {
                        if let Some(name) = &info.name {
                            output.borrow_mut().devices.push(Device {
                                index: info.index,
                                name: name.to_string(),
                                label: info.description.as_deref().unwrap_or(name).to_owned(),
                            });
                        }
                    }
                    ListResult::Error => *error.borrow_mut() = true,
                    _ => {}
                }),
        )?;
        let clients = Rc::new(RefCell::new(HashMap::new()));
        let output = clients.clone();
        let error = failed.clone();
        wait(
            &mut self.mainloop,
            self.context
                .introspect()
                .get_client_info_list(move |item| match item {
                    ListResult::Item(info) => {
                        output.borrow_mut().insert(
                            info.index,
                            info.proplist
                                .get_str("application.process.id")
                                .and_then(|pid| pid.parse::<u32>().ok()),
                        );
                    }
                    ListResult::Error => *error.borrow_mut() = true,
                    _ => {}
                }),
        )?;
        let output = status.clone();
        let error = failed.clone();
        wait(
            &mut self.mainloop,
            self.context
                .introspect()
                .get_sink_input_info_list(move |item| match item {
                    ListResult::Item(info) => output.borrow_mut().streams.push(Stream {
                        index: info.index,
                        pid: info
                            .proplist
                            .get_str("application.process.id")
                            .and_then(|pid| pid.parse().ok())
                            .or_else(|| {
                                info.client
                                    .and_then(|id| clients.borrow().get(&id).copied().flatten())
                            }),
                        device: info.sink,
                        volume: ((u64::from(info.volume.max().0) * 100)
                            / u64::from(Volume::NORMAL.0)) as u16,
                        muted: info.mute,
                        channels: info.volume,
                    }),
                    ListResult::Error => *error.borrow_mut() = true,
                    _ => {}
                }),
        )?;
        if *failed.borrow() {
            return Err("读取系统音频状态失败.".into());
        }
        let result = status.borrow().clone();
        Ok(result)
    }
    pub fn apply(&mut self, options: &Options, pids: &[u32]) -> Result<Status, String> {
        let status = self.status()?;
        let target = options.target(&status);
        let Some(target) = target else {
            let mut status = status;
            status
                .streams
                .retain(|s| s.pid.is_some_and(|pid| pids.contains(&pid)));
            self.applied.clear();
            return Ok(status);
        };
        self.applied.retain(|index, _| {
            status
                .streams
                .iter()
                .any(|s| s.index == *index && s.pid.is_some_and(|pid| pids.contains(&pid)))
        });
        for stream in status
            .streams
            .iter()
            .filter(|s| s.pid.is_some_and(|pid| pids.contains(&pid)))
        {
            let desired = (target.clone(), options.volume);
            if self.applied.get(&stream.index) == Some(&desired) {
                continue;
            }
            let success = Rc::new(RefCell::new(true));
            let result = success.clone();
            wait(
                &mut self.mainloop,
                self.context.introspect().move_sink_input_by_name(
                    stream.index,
                    target,
                    Some(Box::new(move |ok| *result.borrow_mut() &= ok)),
                ),
            )?;
            let mut volume = stream.channels;
            volume.scale(Volume((u32::from(options.volume) * Volume::NORMAL.0) / 100));
            let result = success.clone();
            wait(
                &mut self.mainloop,
                self.context.introspect().set_sink_input_volume(
                    stream.index,
                    &volume,
                    Some(Box::new(move |ok| *result.borrow_mut() &= ok)),
                ),
            )?;
            let result = success.clone();
            wait(
                &mut self.mainloop,
                self.context.introspect().set_sink_input_mute(
                    stream.index,
                    options.volume == 0,
                    Some(Box::new(move |ok| *result.borrow_mut() &= ok)),
                ),
            )?;
            if !*success.borrow() {
                return Err("无法应用游戏音频设置.".into());
            }
            // 只在偏好或设备可用性变化时重新应用, 避免与系统混音器争抢路由.
            self.applied.insert(stream.index, desired);
        }
        let mut actual = self.status()?;
        actual
            .streams
            .retain(|s| s.pid.is_some_and(|pid| pids.contains(&pid)));
        Ok(actual)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disconnected_device_falls_back_and_reconnected_device_restores_preference() {
        let options = Options {
            device: Some("headphones".into()),
            volume: 120,
        };
        let mut status = Status {
            default_device: Some("speakers".into()),
            ..Default::default()
        };
        assert_eq!(
            options.target(&status).map(String::as_str),
            Some("speakers")
        );
        status.devices.push(Device {
            index: 1,
            name: "headphones".into(),
            label: "耳机".into(),
        });
        assert_eq!(
            options.target(&status).map(String::as_str),
            Some("headphones")
        );
        status.devices.clear();
        status.default_device = Some("new-default".into());
        assert_eq!(
            options.target(&status).map(String::as_str),
            Some("new-default")
        );
        assert_eq!(options.device.as_deref(), Some("headphones"));
        status.default_device = None;
        assert!(options.target(&status).is_none());
    }
    #[test]
    fn preferences_reject_invalid_volume_and_device() {
        assert!(Options::default().validate().is_ok());
        assert!(Options {
            volume: 150,
            device: None
        }
        .validate()
        .is_ok());
        assert!(Options {
            volume: 151,
            device: None
        }
        .validate()
        .is_err());
        assert!(Options {
            device: Some("bad\0sink".into()),
            ..Default::default()
        }
        .validate()
        .is_err());
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    struct Probe(std::process::Child);
    impl Drop for Probe {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    #[test]
    #[ignore = "需要本机音频服务和 paplay, 仅播放静音并调整自建测试流"]
    fn own_stream_volume_mute_restore_and_unrelated_stream_isolation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("silence.wav");
        let samples = vec![0u8; 48000 * 4 * 30];
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36u32 + samples.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt \x10\0\0\0\x01\0\x02\0");
        wav.extend_from_slice(&48000u32.to_le_bytes());
        wav.extend_from_slice(&192000u32.to_le_bytes());
        wav.extend_from_slice(b"\x04\0\x10\0data");
        wav.extend_from_slice(&(samples.len() as u32).to_le_bytes());
        wav.extend_from_slice(&samples);
        std::fs::write(&path, wav).unwrap();
        let mut probes = Vec::new();
        for name in ["Oddity Audio Test", "Oddity Unrelated Test"] {
            let mut command = std::process::Command::new("paplay");
            command.arg(format!("--client-name={name}")).arg(&path);
            if probes.is_empty() {
                configure(&mut command).unwrap();
            }
            probes.push(Probe(command.spawn().unwrap()));
        }
        let owned = probes[0].0.id();
        let unrelated = probes[1].0.id();
        let mut client = Client::connect().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let before = loop {
            let status = client.status().unwrap();
            if [owned, unrelated]
                .iter()
                .all(|pid| status.streams.iter().any(|s| s.pid == Some(*pid)))
            {
                break status;
            }
            assert!(Instant::now() < deadline, "测试流未建立");
            std::thread::sleep(Duration::from_millis(25));
        };
        let device = before
            .devices
            .iter()
            .find(|d| d.name.starts_with("alsa_output."))
            .unwrap()
            .name
            .clone();
        let baseline = before
            .streams
            .iter()
            .find(|s| s.pid == Some(unrelated))
            .unwrap();
        for value in [150, 0, 100] {
            let options = Options {
                device: Some(device.clone()),
                volume: value,
            };
            client.apply(&options, &[owned]).unwrap();
            std::thread::sleep(Duration::from_millis(150));
            let status = client.apply(&options, &[owned]).unwrap();
            assert_eq!(status.streams.len(), 1);
            assert_eq!(
                status
                    .devices
                    .iter()
                    .find(|d| d.name == device)
                    .unwrap()
                    .index,
                status.streams[0].device
            );
            assert_eq!(status.streams[0].volume, value);
            assert_eq!(status.streams[0].muted, value == 0);
            let all = client.status().unwrap();
            let other = all
                .streams
                .iter()
                .find(|s| s.pid == Some(unrelated))
                .unwrap();
            assert_eq!(
                (other.volume, other.muted, other.device),
                (baseline.volume, baseline.muted, baseline.device)
            );
            assert_eq!(all.default_device, before.default_device);
        }
        for device in before
            .devices
            .iter()
            .filter(|d| d.name.starts_with("alsa_output."))
        {
            let options = Options {
                device: Some(device.name.clone()),
                volume: 100,
            };
            client.apply(&options, &[owned]).unwrap();
            std::thread::sleep(Duration::from_millis(150));
            let actual = client.apply(&options, &[owned]).unwrap();
            assert_eq!(actual.streams[0].device, device.index);
        }
        assert!(client
            .apply(&Options::default(), &[])
            .unwrap()
            .streams
            .is_empty());
    }
}
