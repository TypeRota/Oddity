//! 游戏包装载测试.

use super::{NtePackage, NTE_PACKAGE};
use crate::testing::{block_on, EnvGuard, TempDir, TestHost};
use oddity_contract::{Audience, COMMANDS};
use oddity_runtime::{CapabilityId, CapabilityRevision, GamePackage, Necessity, Registry, Scope};
use serde_json::json;

fn loaded() -> Registry {
    let mut registry = Registry::new();
    registry.install(&NTE_PACKAGE);
    registry
}

/// 契约里由后端处理的命令.
fn backend_commands() -> Vec<&'static str> {
    COMMANDS
        .iter()
        .filter(|command| command.audience != Audience::Window)
        .map(|command| command.id)
        .collect()
}

#[test]
fn package_loads_with_its_descriptor_and_domain_groups() {
    let registry = loaded();
    let descriptor = NTE_PACKAGE.descriptor();
    assert_eq!(registry.packages(), [descriptor]);

    let groups: Vec<(Option<&str>, &str, Vec<&str>)> = registry
        .groups()
        .iter()
        .map(|group| (group.game(), group.domain(), group.commands().to_vec()))
        .collect();
    assert_eq!(
        groups,
        [
            (Some("nte"), "bulletin", vec!["fetch_bulletins"]),
            (Some("nte"), "background", vec!["launcher_background"]),
            (
                Some("nte"),
                "attendance",
                vec![
                    "attendance_status",
                    "attendance_refresh_game_records",
                    "attendance_login",
                    "attendance_send_code",
                    "attendance_login_sms",
                    "attendance_configure",
                    "attendance_run",
                    "attendance_logout",
                    "attendance_keyring_check",
                ]
            ),
            (
                Some("nte"),
                "auth",
                vec![
                    "game_accounts",
                    "game_send_code",
                    "game_login",
                    "game_account_remove",
                ]
            ),
            (Some("nte"), "update", vec!["check_update"]),
            (
                Some("nte"),
                "download",
                vec![
                    "game_resource_catalog",
                    "game_resource_remove",
                    "game_resource_download",
                    "game_preload_delete",
                    "game_download_status",
                    "game_download_start",
                    "game_download_cancel",
                ]
            ),
        ]
    );
}

#[test]
fn package_owns_its_domains_and_leaves_other_commands_to_the_host() {
    let registry = loaded();
    let claimed: Vec<&str> = registry
        .groups()
        .iter()
        .flat_map(|group| group.commands().iter().copied())
        .collect();
    assert_eq!(
        claimed,
        [
            "fetch_bulletins",
            "launcher_background",
            "attendance_status",
            "attendance_refresh_game_records",
            "attendance_login",
            "attendance_send_code",
            "attendance_login_sms",
            "attendance_configure",
            "attendance_run",
            "attendance_logout",
            "attendance_keyring_check",
            "game_accounts",
            "game_send_code",
            "game_login",
            "game_account_remove",
            "check_update",
            "game_resource_catalog",
            "game_resource_remove",
            "game_resource_download",
            "game_preload_delete",
            "game_download_status",
            "game_download_start",
            "game_download_cancel",
        ]
    );

    let host: Vec<&str> = backend_commands()
        .into_iter()
        .filter(|command| !claimed.contains(command))
        .collect();
    registry
        .validate(&host)
        .expect("组与宿主覆盖全部后端命令时装配校验应通过");

    let error = registry
        .validate(&backend_commands())
        .expect_err("宿主重复声明组命令时装配校验应失败");
    let message = error
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ");
    assert!(message.contains("fetch_bulletins"), "{message}");
    assert!(
        message.contains("命令 fetch_bulletins 同时由宿主与命令组 bulletin 声明"),
        "{message}"
    );
    assert!(message.contains("launcher_background"), "{message}");
    assert!(
        message.contains("命令 launcher_background 同时由宿主与命令组 background 声明"),
        "{message}"
    );
    assert!(message.contains("attendance_status"), "{message}");
    assert!(
        message.contains("命令 attendance_status 同时由宿主与命令组 attendance 声明"),
        "{message}"
    );
    assert!(message.contains("game_accounts"), "{message}");
    assert!(
        message.contains("命令 game_accounts 同时由宿主与命令组 auth 声明"),
        "{message}"
    );
}

#[test]
fn registered_group_dispatches_bulletin_commands() {
    let registry = loaded();
    let group = registry
        .group_for("nte", "fetch_bulletins")
        .expect("公告组应接管抓取命令");
    assert!(group.handles("fetch_bulletins"));
    assert!(!group.handles("launcher_background"));

    let error = block_on(group.dispatch(&TestHost::new(None), "fetch_bulletins", json!(false)))
        .unwrap_err();
    assert!(
        error.as_str().unwrap().contains("expected struct Args"),
        "{error}"
    );

    let error = block_on(group.dispatch(&TestHost::new(None), "unknown", json!({}))).unwrap_err();
    assert_eq!(error, json!("无法识别的公告命令"));
}

#[test]
fn registered_group_dispatches_launcher_background() {
    let dir = TempDir::new("background-group");
    dir.write("launcher/Config/Config.ini", "[General]\nGameID=1289\n");
    let video = dir.write("launcher/ResFilesM/1289/bgimgs/bg.mp4", b"video");

    let registry = loaded();
    let group = registry
        .group_for("nte", "launcher_background")
        .expect("背景组应接管背景命令");
    assert!(group.handles("launcher_background"));
    assert!(!group.handles("fetch_bulletins"));

    let host = TestHost::new(Some(dir.join("launcher")));
    let media = block_on(group.dispatch(&host, "launcher_background", json!({})))
        .expect("背景命令应返回素材");
    assert_eq!(
        media["video"],
        json!(std::fs::canonicalize(video)
            .expect("规范化路径")
            .display()
            .to_string())
    );
    assert!(media["image"].is_null());

    let error = block_on(group.dispatch(&host, "unknown", json!({}))).unwrap_err();
    assert_eq!(error, json!("无法识别的背景命令"));
}

#[test]
fn registered_group_dispatches_attendance_commands() {
    // 状态查询只读配置目录, 不访问密钥环.
    let config = TempDir::new("attendance-group");
    let mut env = EnvGuard::new();
    env.set("XDG_CONFIG_HOME", config.path());

    let registry = loaded();
    let group = registry
        .group_for("nte", "attendance_status")
        .expect("签到组应接管签到命令");
    assert!(group.handles("attendance_run"));
    assert!(!group.handles("launcher_background"));

    let status = block_on(group.dispatch(&TestHost::new(None), "attendance_status", json!({})))
        .expect("状态查询应经命令组返回");
    assert_eq!(status["accounts"], json!([]));
    assert_eq!(status["preferences"]["trigger"], json!("off"));

    let error = block_on(group.dispatch(&TestHost::new(None), "unknown", json!({}))).unwrap_err();
    assert_eq!(error, json!("无法识别的签到命令"));
}

#[test]
fn registered_group_dispatches_game_account_commands() {
    let registry = loaded();
    let group = registry
        .group_for("nte", "game_accounts")
        .expect("账号组应接管账号命令");
    assert!(group.handles("game_account_remove"));
    assert!(!group.handles("attendance_run"));

    let error = block_on(group.dispatch(
        &TestHost::with_settings("nte", "global"),
        "game_accounts",
        json!({}),
    ))
    .unwrap_err();
    assert_eq!(error["kind"], json!("unsupported"));

    let error = block_on(group.dispatch(
        &TestHost::with_settings("nte", "cn"),
        "game_send_code",
        json!({}),
    ))
    .unwrap_err();
    assert_eq!(error, json!("游戏账号请求参数无效"));

    let error = block_on(group.dispatch(&TestHost::new(None), "unknown", json!({}))).unwrap_err();
    assert_eq!(error, json!("无法识别的游戏账号命令"));
}

#[test]
fn package_declares_account_and_profile_capabilities() {
    let registry = loaded();
    let sheet = registry
        .capabilities("nte")
        .expect("异环包应声明能力 sheet");
    let declared: Vec<(CapabilityId, CapabilityRevision, Scope)> = sheet
        .iter()
        .map(|capability| {
            (
                capability.descriptor.id,
                capability.descriptor.revision,
                capability.descriptor.scope,
            )
        })
        .collect();
    let scope = Scope::GAME.union(Scope::DISTRIBUTION).union(Scope::ACCOUNT);
    assert_eq!(
        declared,
        [
            (
                CapabilityId("oddity.account.game"),
                CapabilityRevision(1),
                scope
            ),
            (
                CapabilityId("oddity.profile.player"),
                CapabilityRevision(1),
                scope
            ),
        ]
    );
    assert_eq!(NTE_PACKAGE.descriptor().id, crate::PACKAGE_ID);
}

#[test]
fn package_requirements_resolve_against_its_own_sheet() {
    let registry = loaded();
    assert_eq!(registry.validate_capabilities(), Ok(()));
    let requirements = registry.requires("nte").expect("异环包应声明能力依赖");
    assert_eq!(
        requirements
            .iter()
            .map(|requirement| requirement.id)
            .collect::<Vec<_>>(),
        [
            CapabilityId("oddity.account.game"),
            CapabilityId("oddity.profile.player")
        ]
    );
    assert!(
        requirements
            .iter()
            .all(|requirement| requirement.necessity == Necessity::Required),
        "能力依赖默认为必需"
    );
}

#[test]
fn package_descriptor_matches_the_nte_game_id() {
    let descriptor = <NtePackage as GamePackage>::descriptor(&NTE_PACKAGE);
    assert_eq!(descriptor.id, "nte");
    assert_eq!(descriptor.name, "异环");
}
