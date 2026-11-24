//! 仅调整 Sunshine 捕获屏幕的显示模式, 结束时恢复完整输出布局.
//!
//! 切换通过 KScreen 完成, 与 Sunshine 的 KWin 捕获共享同一个输出对象; 恢复所需的
//! 布局在改动前落盘, 因此进程中途退出后仍然可以还原.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Output {
    name: String,
    connected: bool,
    enabled: bool,
    priority: u32,
    current_mode_id: String,
    modes: Vec<Mode>,
    pos: Position,
    scale: f64,
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Position {
    x: i32,
    y: i32,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Mode {
    id: String,
    size: Size,
    refresh_rate: f64,
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Size {
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
pub(crate) struct Layout {
    outputs: Vec<Output>,
}

fn layout() -> Result<Layout> {
    let output = Command::new("kscreen-doctor").arg("-j").output()?;
    if !output.status.success() {
        bail!("无法读取显示器配置");
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

/// 串流捕获的那块屏幕.
///
/// Sunshine 按 output_name 捕获, 留空时取 KWin 输出的第一个, 也就是主屏;
/// 两种情况都以 KScreen 的主屏优先级判定.
pub(crate) fn capture<'a>(outputs: &'a [Output], name: &str) -> Result<&'a Output> {
    let enabled = || {
        outputs
            .iter()
            .filter(|output| output.connected && output.enabled)
    };
    let output = if name.is_empty() {
        enabled().find(|output| output.priority == 1)
    } else {
        enabled().find(|output| output.name == name)
    };
    output.context("无法确定 Sunshine 捕获的屏幕, 请检查 output_name")
}

pub(crate) fn size(output_name: &str) -> Result<(u32, u32)> {
    let layout = layout()?;
    let output = capture(&layout.outputs, output_name)?;
    let mode = output
        .modes
        .iter()
        .find(|mode| mode.id == output.current_mode_id)
        .context("无法读取当前显示模式")?;
    Ok((mode.size.width, mode.size.height))
}

pub(crate) fn verify(output_name: &str) -> Result<()> {
    layout().and_then(|layout| capture(&layout.outputs, output_name).map(|_| ()))
}

pub(crate) fn state_path() -> PathBuf {
    crate::paths::data_dir().join("cross-screen-display.json")
}

pub(crate) fn prepare(output_name: &str, width: u32, height: u32, fps: u32) -> Result<(u32, u32)> {
    restore()?;
    let before = layout()?;
    let output = capture(&before.outputs, output_name)?.clone();
    let current = output
        .modes
        .iter()
        .find(|mode| mode.id == output.current_mode_id)
        .context("无法读取当前显示模式")?;
    let selected = output
        .modes
        .iter()
        .filter(|mode| mode.size.width == width && mode.size.height == height)
        .min_by(|a, b| {
            (a.refresh_rate - f64::from(fps))
                .abs()
                .total_cmp(&(b.refresh_rate - f64::from(fps)).abs())
        })
        .unwrap_or(current);
    let size = (selected.size.width, selected.size.height);
    if selected.id == current.id {
        return Ok(size);
    }
    let path = state_path();
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, serde_json::to_vec(&before.outputs)?)?;
    let applied = (|| -> Result<()> {
        let switched = Command::new("kscreen-doctor")
            .arg(format!("output.{}.mode.{}", output.name, selected.id))
            .status()?;
        if !switched.success() {
            bail!("切换串流显示模式失败");
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            if layout()?.outputs.iter().any(|candidate| {
                candidate.name == output.name && candidate.current_mode_id == selected.id
            }) {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                bail!("串流显示模式未生效");
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    })();
    if applied.is_err() {
        restore()?;
    }
    applied?;
    Ok(size)
}

pub(crate) fn restore() -> Result<()> {
    let path = state_path();
    if !path.exists() {
        return Ok(());
    }
    let saved: Vec<Output> = serde_json::from_slice(&std::fs::read(&path)?)?;
    let available = layout()?;
    let mut command = Command::new("kscreen-doctor");
    let mut count = 0;
    for output in saved {
        if !output.enabled
            || !available
                .outputs
                .iter()
                .any(|candidate| candidate.connected && candidate.name == output.name)
        {
            continue;
        }
        count += 1;
        command.args([
            format!("output.{}.mode.{}", output.name, output.current_mode_id),
            format!("output.{}.scale.{}", output.name, output.scale),
            format!(
                "output.{}.position.{},{}",
                output.name, output.pos.x, output.pos.y
            ),
            format!("output.{}.priority.{}", output.name, output.priority),
        ]);
    }
    if count > 0 && !command.status()?.success() {
        bail!("恢复显示设置失败, 可在跨屏游玩中重试恢复");
    }
    std::fs::remove_file(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;

    fn output(name: &str, priority: u32) -> Output {
        Output {
            name: name.into(),
            priority,
            connected: true,
            enabled: true,
            current_mode_id: "1".into(),
            modes: vec![],
            pos: Position { x: 0, y: 0 },
            scale: 1.0,
        }
    }

    fn mode(id: &str, width: u32, height: u32, refresh_rate: f64) -> Mode {
        Mode {
            id: id.into(),
            size: Size { width, height },
            refresh_rate,
        }
    }

    /// 用脚本代替 kscreen-doctor: 读取和写入同一份布局文件, 便于断言最终状态.
    fn fixture(
        tag: &str,
        outputs: serde_json::Value,
    ) -> (crate::tests::EnvGuard, crate::tests::TempDir) {
        let mut env = crate::tests::EnvGuard::new();
        let dir = crate::tests::TempDir::new(tag);
        let layout_file = dir.path().join("layout.json");
        std::fs::write(&layout_file, serde_json::to_vec(&outputs).unwrap()).unwrap();
        let script = dir.path().join("kscreen-doctor");
        std::fs::write(
            &script,
            r#"#!/usr/bin/python3
import json,os,sys
path=os.environ['ODDITY_DISPLAY_FIXTURE']
with open(path) as handle: state=json.load(handle)
if sys.argv[1:] == ['-j']:
    print(json.dumps(state))
    sys.exit(0)
if os.environ.get('ODDITY_DISPLAY_FAIL') == '1': sys.exit(1)
for argument in sys.argv[1:]:
    parts=argument.split('.')
    for screen in state['outputs']:
        if screen['name'] != parts[1]: continue
        if parts[2]=='mode': screen['currentModeId']=parts[3]
        elif parts[2]=='scale': screen['scale']=float('.'.join(parts[3:]))
        elif parts[2]=='position': screen['pos']=dict(zip(['x','y'],map(int,parts[3].split(','))))
        elif parts[2]=='priority': screen['priority']=int(parts[3])
with open(path,'w') as handle: json.dump(state,handle)
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        env.set("PATH", dir.path());
        env.set("XDG_DATA_HOME", dir.path());
        env.set("ODDITY_DISPLAY_FIXTURE", &layout_file);
        (env, dir)
    }

    fn screen() -> serde_json::Value {
        let mut screen = output("DP-1", 1);
        screen.modes = vec![
            mode("1", 2560, 1440, 144.0),
            mode("2", 1920, 1080, 60.0),
            mode("3", 1920, 1080, 120.0),
        ];
        screen.scale = 1.25;
        screen.pos = Position { x: 40, y: 20 };
        json!({ "outputs": [screen] })
    }

    #[test]
    fn capture_uses_the_named_output_or_the_primary_one() {
        let primary = output("DP-1", 1);
        let secondary = output("DP-2", 2);
        let outputs = vec![primary, secondary];
        assert_eq!(capture(&outputs, "").unwrap().name, "DP-1");
        assert_eq!(capture(&outputs, "DP-2").unwrap().name, "DP-2");
        assert!(capture(&outputs, "DP-9").is_err());
        // 没有主屏时不能猜, 否则会改错屏幕.
        let mut unset = vec![output("DP-1", 2)];
        assert!(capture(&unset, "").is_err());
        unset[0].enabled = false;
        assert!(capture(&unset, "").is_err());
    }

    #[test]
    fn changes_and_restores_display_modes_and_keeps_failed_restores() {
        let (_env, _dir) = fixture("cross-screen-display", screen());
        assert_eq!(size("DP-1").unwrap(), (2560, 1440));
        verify("DP-1").unwrap();

        // 目标分辨率有多个刷新率时取最接近客户端请求的那个.
        assert_eq!(prepare("DP-1", 1920, 1080, 90).unwrap(), (1920, 1080));
        assert_eq!(layout().unwrap().outputs[0].current_mode_id, "2");
        assert!(state_path().exists());

        std::env::set_var("ODDITY_DISPLAY_FAIL", "1");
        assert!(restore().is_err());
        assert!(state_path().exists());
        std::env::remove_var("ODDITY_DISPLAY_FAIL");

        restore().unwrap();
        let restored = layout().unwrap();
        assert_eq!(restored.outputs[0].current_mode_id, "1");
        assert_eq!(restored.outputs[0].scale, 1.25);
        assert_eq!(restored.outputs[0].pos.x, 40);
        assert!(!state_path().exists());

        // 没有匹配模式时保持当前模式, 不落到别的分辨率上.
        assert_eq!(prepare("DP-1", 1280, 800, 60).unwrap(), (2560, 1440));
        assert!(!state_path().exists());
    }

    #[test]
    fn a_failed_switch_keeps_the_record_so_the_user_can_retry() {
        let (_env, _dir) = fixture("cross-screen-display-fail", screen());
        std::env::set_var("ODDITY_DISPLAY_FAIL", "1");
        assert!(prepare("DP-1", 1920, 1080, 60).is_err());
        std::env::remove_var("ODDITY_DISPLAY_FAIL");
        // 回滚同样失败时必须留下记录, 否则界面无法提示恢复.
        assert!(state_path().exists());
        restore().unwrap();
        assert!(!state_path().exists());
    }

    #[test]
    fn restore_skips_outputs_that_are_gone_and_needs_no_state() {
        let (_env, _dir) = fixture("cross-screen-display-missing", screen());
        restore().unwrap();
        let mut gone = screen();
        gone["outputs"][0]["connected"] = json!(false);
        std::fs::create_dir_all(state_path().parent().unwrap()).unwrap();
        std::fs::write(state_path(), serde_json::to_vec(&gone["outputs"]).unwrap()).unwrap();
        restore().unwrap();
        assert!(!state_path().exists());
    }
}
