//! 宿主事件出口.
//!
//! 事件名由 `oddity-contract` 声明; 载荷是 serde 值, 由宿主序列化后交给
//! Electron 转发给渲染层. 游戏包拿到宿主能力后调 `emit`, 不自行写管道.

use oddity_contract::Event;
use serde::Serialize;
use serde_json::Value;

/// 宿主向渲染层发送事件的唯一出口.
///
/// 游戏包通过宿主能力拿到一个实现; 事件 id 必须是
/// [`oddity_contract::EVENTS`] 里声明的事件, 未声明的事件按错误返回.
pub trait EventSink: Send + Sync {
    /// 把契约声明的事件与已序列化的载荷交给宿主发送.
    ///
    /// 只由 [`EventSink::emit`] 调用; 实现方不自行核对事件 id.
    fn deliver(&self, event: &'static Event, payload: Value) -> Result<(), String>;

    /// 发送一条已声明事件的载荷.
    ///
    /// 序列化失败、事件未声明或宿主输出已关闭时返回 `Err`.
    fn emit(&self, id: &'static str, payload: impl Serialize) -> Result<(), String> {
        let event =
            oddity_contract::event(id).ok_or_else(|| format!("事件未在契约里声明: {id}"))?;
        let payload = serde_json::to_value(payload).map_err(|error| error.to_string())?;
        self.deliver(event, payload)
    }
}
