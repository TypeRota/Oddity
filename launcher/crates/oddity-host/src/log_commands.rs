use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::logs::{collect_crashes, LogPreview, PreparedLogs};
use crate::service::ServiceHandle;
use serde::Serialize;

static NEXT_EXPORT: AtomicU64 = AtomicU64::new(1);

#[derive(Default)]
pub struct LogExports(Arc<Mutex<Option<(String, PreparedLogs)>>>);

#[derive(Serialize)]
pub struct Preview {
    id: String,
    #[serde(flatten)]
    summary: LogPreview,
}

pub async fn prepare_log_export(app: &ServiceHandle) -> Result<Preview, String> {
    let state = app.exports.0.clone();
    let packages = app.state.packages.clone();
    let settings = app
        .state
        .settings
        .lock()
        .map_err(|error| error.to_string())?
        .clone();
    crate::service::spawn_blocking(move || {
        // 崩溃现场属于运行环境; 解析不到环境时只导出启动器日志.
        // 转储只登记在清单里, 保持导出包体积可控.
        let crashes = collect_crashes(&packages, &settings, false);
        let logs = PreparedLogs::prepare_with_crashes(
            &crate::paths::data_dir().join("logs"),
            None,
            Some(crashes),
        )
        .map_err(|error| format!("{error:#}"))?;
        let id = NEXT_EXPORT.fetch_add(1, Ordering::Relaxed).to_string();
        let preview = Preview {
            id: id.clone(),
            summary: logs.preview.clone(),
        };
        *state.lock().map_err(|_| "无法访问待导出日志")? = Some((id, logs));
        Ok(preview)
    })
    .await
    .map_err(|_| "日志检查任务失败")?
}

pub async fn save_log_export(
    state: &LogExports,
    id: String,
    destination: String,
) -> Result<String, String> {
    let pending = state.0.clone();
    crate::service::spawn_blocking(move || {
        let mut pending = pending.lock().map_err(|_| "无法访问待导出日志")?;
        let (_, logs) = pending
            .as_mut()
            .filter(|(current, _)| *current == id)
            .ok_or("日志预览已失效, 请重新检查")?;
        let path = logs
            .save(Path::new(&destination))
            .map_err(|error| format!("{error:#}"))?;
        *pending = None;
        Ok(path.display().to_string())
    })
    .await
    .map_err(|_| "日志保存任务失败")?
}

pub fn discard_log_export(state: &LogExports, id: String) -> Result<(), String> {
    let mut pending = state.0.lock().map_err(|_| "无法访问待导出日志")?;
    if pending.as_ref().is_some_and(|(current, _)| *current == id) {
        *pending = None;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{EnvGuard, TempDir};
    use std::fs;
    use std::io::Read;

    #[test]
    fn commands_bind_preview_save_retry_and_discard_to_the_same_snapshot() {
        let mut env = EnvGuard::new();
        let dir = TempDir::new("log-commands");
        env.set("XDG_DATA_HOME", dir.path());
        dir.write("oddity/logs/test.log", "token=command-secret\n");
        let mut app = crate::service::test_service();
        std::sync::Arc::get_mut(&mut app).unwrap().exports = LogExports::default();
        crate::service::block_on(async {
            let first = prepare_log_export(&app).await.unwrap();
            assert_eq!(first.summary.file_count, 1);
            assert_eq!(first.summary.redactions.credentials, 1);
            let second = prepare_log_export(&app).await.unwrap();
            assert_ne!(first.id, second.id);
            // 旧预览不能保存或释放新快照.
            discard_log_export(&app.exports, first.id.clone()).unwrap();
            assert!(
                save_log_export(&app.exports, first.id, dir.path().display().to_string())
                    .await
                    .is_err()
            );
            assert!(save_log_export(
                &app.exports,
                second.id.clone(),
                dir.join("missing").display().to_string()
            )
            .await
            .is_err());
            dir.write("oddity/logs/test.log", "unreviewed\n");
            let output = save_log_export(
                &app.exports,
                second.id.clone(),
                dir.path().display().to_string(),
            )
            .await
            .unwrap();
            let mut archive = zip::ZipArchive::new(fs::File::open(output).unwrap()).unwrap();
            let mut text = String::new();
            archive
                .by_name("test.log")
                .unwrap()
                .read_to_string(&mut text)
                .unwrap();
            assert!(text.contains("[凭据已移除]"));
            assert!(!text.contains("command-secret"));
            assert!(!text.contains("unreviewed"));
            assert!(
                save_log_export(&app.exports, second.id, dir.path().display().to_string())
                    .await
                    .is_err()
            );
            let third = prepare_log_export(&app).await.unwrap();
            discard_log_export(&app.exports, third.id.clone()).unwrap();
            assert!(
                save_log_export(&app.exports, third.id, dir.path().display().to_string())
                    .await
                    .is_err()
            );
        });
        assert!(app.exports.0.lock().unwrap().is_none());
    }

    /// 受管环境目录: 身份文件与 prefix 由环境校验读取.
    fn environment(dir: &TempDir, id: &str) -> crate::environment::RuntimeEnvironment {
        let root = dir.join(format!("oddity/environments/{id}"));
        fs::create_dir_all(root.join("prefix/drive_c")).unwrap();
        fs::write(root.join(".oddity-environment"), id).unwrap();
        crate::environment::RuntimeEnvironment {
            id: id.to_string(),
            name: "测试环境".into(),
            proton: crate::environment::ProtonPin {
                path: dir.join("proton"),
                identity: String::new(),
            },
            install_root: dir.join("install"),
            game: crate::identity::GameId::from("nte"),
            created_at: 0,
        }
    }

    #[test]
    fn prepares_crash_sites_of_the_selected_environment() {
        let mut env = EnvGuard::new();
        let dir = TempDir::new("log-commands-crash");
        env.set("XDG_DATA_HOME", dir.path());
        dir.write("oddity/logs/test.log", "启动日志\n");
        let id = "a".repeat(32);
        let site = format!(
            "oddity/environments/{id}/prefix/drive_c/users/steamuser/AppData/Local/HT/Saved/Crashes/UECC-abc_0000"
        );
        dir.write(
            format!("{site}/CrashContext.runtime-xml"),
            "<CrashContext><CrashType>Hang</CrashType></CrashContext>",
        );
        assert!(dir.join(&site).is_dir());
        let app = crate::service::test_service();
        {
            let mut settings = app.state.settings.lock().unwrap();
            settings.current_mut().distribution = Some(crate::channel::ChannelId::new("cn"));
            settings
                .current_mut()
                .set_active_environment("cn", Some(id.as_str()));
            settings.runtime_environments.push(environment(&dir, &id));
        }
        crate::service::block_on(async {
            let preview = prepare_log_export(&app).await.unwrap();
            assert_eq!(preview.summary.file_count, 1);
            assert_eq!(preview.summary.crash_sites, 1);
            let output =
                save_log_export(&app.exports, preview.id, dir.path().display().to_string())
                    .await
                    .unwrap();
            let mut archive = zip::ZipArchive::new(fs::File::open(output).unwrap()).unwrap();
            let mut text = String::new();
            archive
                .by_name("crashes/steamuser/Crashes/UECC-abc_0000/CrashContext.runtime-xml")
                .unwrap()
                .read_to_string(&mut text)
                .unwrap();
            assert!(text.contains("Hang"));
        });
    }

    #[test]
    fn absent_environment_still_exports_launcher_logs() {
        let mut env = EnvGuard::new();
        let dir = TempDir::new("log-commands-no-environment");
        env.set("XDG_DATA_HOME", dir.path());
        dir.write("oddity/logs/test.log", "启动日志\n");
        let app = crate::service::test_service();
        crate::service::block_on(async {
            let preview = prepare_log_export(&app).await.unwrap();
            assert_eq!(preview.summary.file_count, 1);
            assert_eq!(preview.summary.crash_sites, 0);
            let output =
                save_log_export(&app.exports, preview.id, dir.path().display().to_string())
                    .await
                    .unwrap();
            let archive = zip::ZipArchive::new(fs::File::open(output).unwrap()).unwrap();
            assert_eq!(archive.len(), 1);
        });
    }
}
