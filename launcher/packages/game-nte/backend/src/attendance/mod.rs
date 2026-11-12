//! 塔吉多登录和可选签到, 所有入口共享会话、锁和执行结果。
//!
//! 命令组声明 `attendance_*` 并由包内分发; 参数与返回沿用宿主的 JSON 载荷,
//! 账号凭据只写入桌面密钥环, 游戏状态与签到记录存放在包内配置目录.

mod client;
mod game_record;
pub(crate) mod lifecycle;
pub mod schedule;
mod service;
mod vault;

use oddity_platform::clock::unix_seconds as now;
use oddity_runtime::{
    Availability, CapabilityContext, CapabilityFailure, HostContext, TaskOutcome,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::borrow::Cow;
use std::fmt;
use std::path::PathBuf;

pub use game_record::GameRecord;
pub use service::Service;
pub use vault::{check_keyring, KeyringVault, Vault};

/// 签到域注册到宿主命令组里的命令 id。
pub(crate) const COMMANDS: &[&str] = &[
    "attendance_status",
    "attendance_refresh_game_records",
    "attendance_login",
    "attendance_send_code",
    "attendance_login_sms",
    "attendance_configure",
    "attendance_run",
    "attendance_logout",
    "attendance_keyring_check",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FailureKind {
    Network,
    Auth,
    Verification,
    Keyring,
    Storage,
    Busy,
    Protocol,
    Service,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    pub kind: FailureKind,
    /// 能力 gate 的稳定错误码; 反序列化缺少字段时按 None 读回.
    ///
    /// 静态码保持借用, 持久化状态读回时再拥有; 对外 JSON 形状不变.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<Cow<'static, str>>,
    /// 能力 `Unavailable` 的 provider 原因码.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<Cow<'static, str>>,
    pub message: String,
}
impl Failure {
    fn new(kind: FailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            code: None,
            reason: None,
            message: message.into(),
        }
    }
    fn capability(error: CapabilityFailure) -> Self {
        Self {
            kind: FailureKind::Service,
            code: Some(Cow::Borrowed(error.code)),
            reason: error.reason.map(Cow::Borrowed),
            message: error.message.into(),
        }
    }
    fn at(mut self, stage: &str) -> Self {
        self.message = format!("{stage}: {}", self.message);
        self
    }
    fn needs_login(&self) -> bool {
        matches!(self.kind, FailureKind::Auth | FailureKind::Verification)
    }
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Failure {}
pub type Result<T> = std::result::Result<T, Failure>;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub phone: String,
    pub password: Option<String>,
    pub device_id: String,
    pub uid: String,
    pub refresh_token: String,
    pub laohu_token: String,
    pub laohu_user_id: String,
}
impl Drop for Account {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.phone.zeroize();
        self.password.zeroize();
        self.refresh_token.zeroize();
        self.laohu_token.zeroize();
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Trigger {
    #[default]
    Off,
    Launch,
    Boot,
    Daily,
    Manual,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TaskPermissions {
    pub community_sign: bool,
    pub role_sign: bool,
    pub browse_posts: bool,
    pub like_posts: bool,
    pub share_posts: bool,
    pub cloud_duration: bool,
}
impl Default for TaskPermissions {
    fn default() -> Self {
        Self {
            community_sign: true,
            role_sign: true,
            browse_posts: true,
            like_posts: true,
            share_posts: true,
            cloud_duration: true,
        }
    }
}
impl TaskPermissions {
    fn coin_enabled(&self) -> bool {
        self.community_sign || self.browse_posts || self.like_posts || self.share_posts
    }
    fn any(&self) -> bool {
        self.coin_enabled() || self.role_sign || self.cloud_duration
    }
    fn allows(&self, label: &str) -> bool {
        match label {
            "社区签到" | "BBS 金币签到" => self.community_sign,
            "浏览帖子" => self.browse_posts,
            "点赞帖子" => self.like_posts,
            "分享帖子" => self.share_posts,
            "云异环时长" => self.cloud_duration,
            "今日金币" | "金币任务" => self.coin_enabled(),
            _ => self.role_sign && (label.starts_with("角色 ") || label == "获取异环角色"),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preferences {
    pub trigger: Trigger,
    pub time: String,
    pub tasks: TaskPermissions,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            trigger: Trigger::Off,
            time: "08:00".into(),
            tasks: TaskPermissions::default(),
        }
    }
}
impl Preferences {
    pub fn validate(&self) -> Result<()> {
        let b = self.time.as_bytes();
        if b.len() != 5
            || b[2] != b':'
            || !b
                .iter()
                .enumerate()
                .all(|(i, b)| i == 2 || b.is_ascii_digit())
            || self.time[..2].parse::<u8>().unwrap_or(24) > 23
            || self.time[3..].parse::<u8>().unwrap_or(60) > 59
        {
            return Err(Failure::new(
                FailureKind::Protocol,
                "签到时间必须为 HH:MM 格式",
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub label: String,
    pub ok: bool,
    pub message: String,
    pub kind: Option<FailureKind>,
    /// 附加任务失败不影响核心签到状态, 也不触发重新登录。
    #[serde(default)]
    pub optional: bool,
}
impl Step {
    fn from_result(label: impl Into<String>, result: Result<()>) -> Self {
        match result {
            Ok(()) => Self {
                label: label.into(),
                ok: true,
                message: "签到完成或今日已签到".into(),
                kind: None,
                optional: false,
            },
            Err(e) => Self {
                label: label.into(),
                ok: false,
                message: e.message.clone(),
                kind: Some(e.kind),
                optional: false,
            },
        }
    }
    fn optional(
        label: impl Into<String>,
        ok: bool,
        message: impl Into<String>,
        kind: Option<FailureKind>,
    ) -> Self {
        Self {
            label: label.into(),
            ok,
            message: message.into(),
            kind,
            optional: true,
        }
    }
    fn optional_failure(label: impl Into<String>, error: Failure) -> Self {
        Self {
            label: label.into(),
            ok: false,
            message: error.message.replace("; 自动任务已暂停", ""),
            kind: Some(error.kind),
            optional: true,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub at: u64,
    pub ok: bool,
    pub steps: Vec<Step>,
}
impl Report {
    fn coin_complete(&self, tasks: &TaskPermissions) -> bool {
        [
            "BBS 金币签到",
            "浏览帖子",
            "点赞帖子",
            "分享帖子",
            "今日金币",
        ]
        .iter()
        .filter(|label| tasks.allows(label))
        .all(|label| {
            self.steps
                .iter()
                .any(|step| step.optional && step.label == *label && step.ok)
        }) && !self.steps.iter().any(|step| {
            step.optional && tasks.allows(&step.label) && step.label != "云异环时长" && !step.ok
        })
    }

    fn core_complete(&self, tasks: &TaskPermissions) -> bool {
        (!tasks.community_sign || self.steps.iter().any(|s| s.label == "社区签到" && s.ok))
            && (!tasks.role_sign
                || self
                    .steps
                    .iter()
                    .any(|s| s.label.starts_with("角色 ") && s.ok))
            && self
                .steps
                .iter()
                .filter(|s| !s.optional && tasks.allows(&s.label))
                .all(|s| s.ok)
    }

    fn cloud_complete(&self) -> bool {
        self.steps
            .iter()
            .any(|step| step.label == "云异环时长" && step.ok)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStatus {
    pub id: String,
    pub masked_phone: String,
    pub remember_password: bool,
    pub signed_today: bool,
    #[serde(default)]
    pub failed_today: bool,
    pub last_report: Option<Report>,
    pub attention: Option<Failure>,
    #[serde(default)]
    pub game_record: Option<GameRecord>,
    #[serde(default)]
    pub game_record_error: Option<Failure>,
}
impl AccountStatus {
    fn new(account: &Account) -> Self {
        Self {
            id: account_id(&account.phone),
            masked_phone: mask_phone(&account.phone),
            remember_password: account.password.is_some(),
            signed_today: false,
            failed_today: false,
            last_report: None,
            attention: None,
            game_record: None,
            game_record_error: None,
        }
    }
    fn tasks_complete(&self, tasks: &TaskPermissions) -> bool {
        if !tasks.any() {
            return true;
        }
        self.last_report.as_ref().is_some_and(|report| {
            china_day(report.at) == china_day(now())
                && report.ok
                && report.core_complete(tasks)
                && report.coin_complete(tasks)
                && (!tasks.cloud_duration || report.cloud_complete())
        })
    }

    fn refresh_day(&mut self) {
        let today = china_day(now());
        self.signed_today = self
            .last_report
            .as_ref()
            .is_some_and(|r| r.ok && china_day(r.at) == today);
        self.failed_today = self
            .last_report
            .as_ref()
            .is_some_and(|r| !r.ok && china_day(r.at) == today);
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Status {
    pub accounts: Vec<AccountStatus>,
    pub preferences: Preferences,
}
fn account_id(phone: &str) -> String {
    use md5::{Digest, Md5};
    Md5::digest(phone.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn mask_phone(phone: &str) -> String {
    if phone.len() == 11 && phone.bytes().all(|b| b.is_ascii_digit()) {
        format!("{}****{}", &phone[..3], &phone[7..])
    } else {
        "****".into()
    }
}

pub fn default_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        })
        .join("oddity/taygedo")
}
pub fn default_service() -> Service<KeyringVault> {
    Service::new(default_dir())
}

/// 手动签到: 与宿主命令共用包内服务, 只把结果打印成终端文本.
pub fn run_manual(dir: Option<PathBuf>) -> Result<()> {
    print_status(Service::new(dir.unwrap_or_else(default_dir)).run(None)?)
}

/// 打印签到结果; 任一账号未签到或需要关注时返回错误.
fn print_status(status: Status) -> Result<()> {
    if status.accounts.is_empty() {
        return Err(Failure::new(FailureKind::Auth, "请先登录塔吉多账号"));
    }
    let mut failed = false;
    for account in status.accounts {
        if let Some(error) = &account.attention {
            eprintln!("{}: {}", account.masked_phone, error);
            failed = true;
        }
        if account.signed_today {
            println!("{}: 今日已签到", account.masked_phone);
        } else {
            failed = true;
            println!("{}: 今日未签到", account.masked_phone);
        }
        if let Some(report) = &account.last_report {
            for step in &report.steps {
                println!("{}: {}: {}", account.masked_phone, step.label, step.message);
            }
        }
    }
    if failed {
        return Err(Failure::new(FailureKind::Service, "签到未全部成功"));
    }
    Ok(())
}

/// 密钥环自检: 读写并删除独立临时条目, 不读取已保存账号.
pub fn run_keyring_check() -> Result<()> {
    check_keyring()?;
    println!("Keyring 存取与清理检查通过");
    Ok(())
}

fn china_day(at: u64) -> u64 {
    (at + 8 * 3600) / 86400
}
fn storage_error(_: impl fmt::Display) -> Failure {
    Failure::new(
        FailureKind::Storage,
        "无法读取或保存签到配置, 请检查配置目录权限",
    )
}
fn id(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// 分发签到域命令; 参数与结果都是宿主载荷。
pub(crate) async fn command(
    host: &dyn HostContext,
    command: &str,
    args: Value,
) -> std::result::Result<Value, Value> {
    dispatch(host, command, args, default_service).await
}

/// 按命令解析参数并交给业务层; 测试注入内存密钥环替身。
async fn dispatch<V, F>(
    host: &dyn HostContext,
    command: &str,
    args: Value,
    service: F,
) -> std::result::Result<Value, Value>
where
    V: Vault + Send + 'static,
    F: Fn() -> Service<V> + Send + 'static,
{
    match command {
        "attendance_status" => respond(blocking(move || service().status()).await).await,
        "attendance_refresh_game_records" => {
            require_profile(host).map_err(failure)?;
            respond(blocking(move || service().refresh_game_records()).await).await
        }
        "attendance_login" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                phone: String,
                password: String,
                remember_password: bool,
            }
            let args: Args =
                serde_json::from_value(args).map_err(|error| json!(error.to_string()))?;
            let password = zeroize::Zeroizing::new(args.password);
            require_profile(host).map_err(failure)?;
            respond(
                blocking(move || {
                    service().login(args.phone.trim(), &password, args.remember_password)
                })
                .await,
            )
            .await
        }
        "attendance_send_code" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                phone: String,
            }
            let args: Args =
                serde_json::from_value(args).map_err(|error| json!(error.to_string()))?;
            require_profile(host).map_err(failure)?;
            respond(blocking(move || service().send_code(args.phone.trim())).await).await
        }
        "attendance_login_sms" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                phone: String,
                code: String,
                device_id: String,
            }
            let args: Args =
                serde_json::from_value(args).map_err(|error| json!(error.to_string()))?;
            let code = zeroize::Zeroizing::new(args.code);
            require_profile(host).map_err(failure)?;
            respond(
                blocking(move || service().login_sms(args.phone.trim(), &code, &args.device_id))
                    .await,
            )
            .await
        }
        "attendance_configure" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                preferences: Preferences,
            }
            let args: Args =
                serde_json::from_value(args).map_err(|error| json!(error.to_string()))?;
            respond(blocking(move || service().configure(args.preferences, &executable()?)).await)
                .await
        }
        "attendance_run" => {
            require_profile(host).map_err(failure)?;
            run(host, service).await
        }
        "attendance_logout" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                account_id: String,
            }
            let args: Args =
                serde_json::from_value(args).map_err(|error| json!(error.to_string()))?;
            respond(blocking(move || service().logout(&args.account_id, &executable()?)).await)
                .await
        }
        "attendance_keyring_check" => respond(blocking(check_keyring).await).await,
        _ => Err(json!("无法识别的签到命令")),
    }
}

/// 签到 run 是长任务: 交给宿主执行器, 进度按账号上报.
///
/// 命令仍在任务结束后才返回, 成功与错误载荷与在命令组内直接执行时一致;
/// 串行语义由业务层的文件锁保持.
async fn run<V, F>(host: &dyn HostContext, service: F) -> std::result::Result<Value, Value>
where
    V: Vault + Send + 'static,
    F: Fn() -> Service<V> + Send + 'static,
{
    let task = host.submit(Box::new(move |context| {
        let result = service().run_with_progress(None, &mut |progress| {
            // 进度是尽力而为的: 宿主输出关闭不影响签到结果.
            let _ = context.progress(progress);
        });
        match result {
            Ok(status) => match serde_json::to_value(status) {
                Ok(value) => TaskOutcome::Done(value),
                Err(error) => TaskOutcome::Failed(json!(error.to_string())),
            },
            Err(error) => TaskOutcome::Failed(failure(error)),
        }
    }));
    match task.wait().await {
        TaskOutcome::Done(value) => Ok(value),
        TaskOutcome::Failed(value) => Err(value),
        TaskOutcome::Cancelled => Err(json!("签到已取消")),
    }
}

/// 塔吉多玩家档案能力的可用性: 只对《异环》国服提供.
pub(crate) fn availability(context: &CapabilityContext) -> Availability {
    if context.game == "nte" && context.distribution == "cn" {
        Availability::Available
    } else {
        Availability::Unsupported
    }
}

/// 能力门文案; 只覆盖跨网与凭据操作, 本地读取与清理不 gate.
const UNSUPPORTED: &str = "塔吉多仅支持国服账号, 国际服请使用官方账号服务";

/// 跨网与凭据操作先校验玩家档案能力; 上下文读取失败沿用域内文案.
fn require_profile(host: &dyn HostContext) -> Result<()> {
    let context = host
        .capability_context()
        .map_err(|_| Failure::new(FailureKind::Service, "无法读取当前游戏地区"))?;
    availability(&context)
        .into_gate(UNSUPPORTED)
        .map_err(Failure::capability)
}

/// 签到命令的阻塞工作交给执行器, 与宿主原有的后台线程一致。
async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|_| Failure::new(FailureKind::Service, "签到后台任务异常"))?
}

/// 定时单元用当前进程路径重启无头入口, 由宿主进程解释参数。
fn executable() -> Result<PathBuf> {
    std::env::current_exe().map_err(|_| Failure::new(FailureKind::Service, "无法确定启动器路径"))
}

/// 业务错误序列化为宿主的错误载荷, 序列化失败时退回 null。
fn failure(error: Failure) -> Value {
    serde_json::to_value(error).unwrap_or(Value::Null)
}

/// 成功结果序列化为宿主载荷; 形状与命令组接管前的宿主实现一致。
async fn respond<T: Serialize>(result: Result<T>) -> std::result::Result<Value, Value> {
    serde_json::to_value(result.map_err(failure)?).map_err(|error| json!(error.to_string()))
}

#[cfg(test)]
mod tests;
