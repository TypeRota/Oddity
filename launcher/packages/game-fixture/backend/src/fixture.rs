//! fixture 包的命令实现: 固定数据、参数回显与失败注入.

use serde::Deserialize;
use serde_json::{json, Value};

/// 探测组: 回显参数与失败注入.
pub(crate) const PROBE_COMMANDS: &[&str] = &["fixture_echo", "fixture_fail"];

/// 信息组: 返回固定的包事实.
pub(crate) const INFO_COMMANDS: &[&str] = &["fixture_info"];

/// 参数只接受一个字符串字段, 形状与契约保持一致.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MessageArgs {
    message: String,
}

/// 分发 fixture 命令; 参数与结果都是宿主载荷.
pub(crate) async fn command(command: &str, args: Value) -> std::result::Result<Value, Value> {
    match command {
        "fixture_echo" => echo(args),
        "fixture_info" => Ok(info()),
        "fixture_fail" => fail(args),
        _ => Err(json!("无法识别的 fixture 命令")),
    }
}

fn message_args(args: Value) -> std::result::Result<String, Value> {
    let MessageArgs { message } =
        serde_json::from_value(args).map_err(|error| json!(error.to_string()))?;
    Ok(message)
}

fn echo(args: Value) -> std::result::Result<Value, Value> {
    let message = message_args(args)?;
    let length = message.chars().count();
    Ok(json!({
        "message": message,
        "length": length,
    }))
}

fn info() -> Value {
    json!({
        "game": "fixture",
        "name": "测试游戏",
        "capabilities": ["info", "echo", "failure-injection"],
    })
}

fn fail(args: Value) -> std::result::Result<Value, Value> {
    let message = message_args(args)?;
    Err(json!({
        "code": "fixture_failure",
        "message": message,
    }))
}
