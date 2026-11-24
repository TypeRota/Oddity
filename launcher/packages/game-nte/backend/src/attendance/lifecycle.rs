//! 签到域依赖宿主进程生命周期的两个触发点: 启动器打开后的自动签到,
//! 以及 systemd 单元拉起时的无头执行.
//!
//! 宿主只提供只读设置快照与 argv; 是否适用由本域判断.

use std::ffi::OsString;
use std::path::PathBuf;

use oddity_runtime::HostContext;

use super::{default_dir, Failure, FailureKind, Result, Service, Trigger};

/// 启动器打开时按偏好触发一次自动签到.
pub(crate) fn on_launch(host: &dyn HostContext) {
    let Ok(settings) = host.settings() else {
        return;
    };
    if settings.game != crate::PACKAGE_ID || settings.distribution != "cn" {
        return;
    }
    let data = host.data_dir();
    launch(Service::with_data_dir(default_dir(), data.as_deref()));
}

/// 按偏好决定是否签到; 签到是阻塞的长任务, 交给独立线程, 不占用宿主关闭路径.
pub(super) fn launch<V: super::Vault + Send + 'static>(service: Service<V>) {
    if !service
        .status()
        .is_ok_and(|status| status.preferences.trigger == Trigger::Launch)
    {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("oddity-attendance".into())
        .spawn(move || {
            let _ = service.run(Some(Trigger::Launch));
        });
}

/// systemd 单元与定时器的无头入口; 不认领参数时返回 None.
pub(crate) fn headless_entry(args: &[OsString]) -> Option<i32> {
    if !args
        .iter()
        .any(|argument| argument == "--taygedo-attendance")
    {
        return None;
    }
    let dir = args
        .windows(2)
        .find(|pair| pair[0] == "--taygedo-config")
        .map(|pair| PathBuf::from(&pair[1]))
        .unwrap_or_else(default_dir);
    // 无头入口没有宿主上下文, 日志目录按用户数据目录推导.
    Some(match run_headless(Service::with_data_dir(dir, None)) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    })
}

fn run_headless(service: Service<super::KeyringVault>) -> Result<()> {
    execute(service)
}

/// 无头执行的公共流程; 与入口共用, 测试可换用内存密钥环.
pub(super) fn execute<V: super::Vault>(service: Service<V>) -> Result<()> {
    let current = service.status()?;
    if current.accounts.is_empty() {
        return Err(Failure::new(FailureKind::Auth, "请先登录塔吉多账号"));
    }
    let trigger = current.preferences.trigger;
    if !matches!(trigger, Trigger::Daily | Trigger::Boot) {
        return Ok(());
    }
    let status = service.run(Some(trigger))?;
    if status.accounts.is_empty() {
        return Err(Failure::new(FailureKind::Auth, "请先登录塔吉多账号"));
    }
    let mut failed = false;
    for account in status.accounts {
        if let Some(error) = &account.attention {
            eprintln!("{}: {}", account.masked_phone, error);
            failed = true;
        }
        if let Some(report) = &account.last_report {
            if !report.ok {
                failed = true;
            }
            for step in &report.steps {
                println!("{}: {}: {}", account.masked_phone, step.label, step.message);
            }
        }
        if account.signed_today {
            println!("{}: 今日已签到", account.masked_phone);
        } else {
            failed = true;
        }
    }
    if failed {
        return Err(Failure::new(FailureKind::Service, "签到未全部成功"));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{EnvGuard, TempDir, TestHost};

    #[test]
    fn headless_entry_ignores_other_invocations() {
        assert_eq!(headless_entry(&[]), None);
        assert_eq!(headless_entry(&[OsString::from("--ipc")]), None);
    }

    #[test]
    fn headless_entry_reports_missing_accounts() {
        let config = TempDir::new("headless-empty");
        let args = [
            OsString::from("--taygedo-attendance"),
            OsString::from("--taygedo-config"),
            config.path().as_os_str().to_os_string(),
        ];
        assert_eq!(headless_entry(&args), Some(1));
    }

    #[test]
    fn on_launch_only_runs_for_the_cn_region() {
        on_launch(&TestHost::with_settings("fixture", "cn"));
        on_launch(&TestHost::with_settings("nte", "global"));
        on_launch(&TestHost::failing_settings());
    }

    #[test]
    fn on_launch_skips_when_the_trigger_is_off() {
        let config = TempDir::new("launch-off");
        let mut env = EnvGuard::new();
        env.set("XDG_CONFIG_HOME", config.path());
        on_launch(&TestHost::with_settings("nte", "cn"));
    }
}
