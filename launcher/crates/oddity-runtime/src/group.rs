use crate::HostContext;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;

/// 命令组的分发结果; 失败载荷同样是 JSON.
pub type DispatchOutcome = Result<Value, Value>;

/// 命令组的异步分发.
pub type DispatchFuture<'a> = Pin<Box<dyn Future<Output = DispatchOutcome> + Send + 'a>>;

/// 组内分发函数.
pub type CommandDispatch = for<'a> fn(&'a dyn HostContext, &'a str, Value) -> DispatchFuture<'a>;

/// 游戏包声明的一组命令.
///
/// 组自带所有者, 命令 id 集合与分发入口; 宿主按 (游戏, 命令) 找组,
/// 不关心组内结构.
pub struct CommandGroup {
    /// None = 通用组, 对所有游戏可用; Some(game) = 只属于该游戏.
    game: Option<&'static str>,
    domain: &'static str,
    commands: &'static [&'static str],
    dispatch: CommandDispatch,
}

impl CommandGroup {
    /// 游戏包的命令组; `game` 取包 id 常量, 不在调用点写字符串字面量.
    pub fn for_game(
        game: &'static str,
        domain: &'static str,
        commands: &'static [&'static str],
        dispatch: CommandDispatch,
    ) -> Self {
        Self {
            game: Some(game),
            domain,
            commands,
            dispatch,
        }
    }

    /// 通用命令组; 同一命令的游戏专属组优先.
    pub fn shared(
        domain: &'static str,
        commands: &'static [&'static str],
        dispatch: CommandDispatch,
    ) -> Self {
        Self {
            game: None,
            domain,
            commands,
            dispatch,
        }
    }

    /// 组的所有者; None 表示通用组.
    pub fn game(&self) -> Option<&'static str> {
        self.game
    }

    /// 组所属的领域 id.
    pub fn domain(&self) -> &'static str {
        self.domain
    }

    /// 组声明的命令 id.
    pub fn commands(&self) -> &'static [&'static str] {
        self.commands
    }

    /// 命令是否属于本组.
    pub fn handles(&self, command: &str) -> bool {
        self.commands.contains(&command)
    }

    /// 组内分发; 调用方负责提供宿主能力与命令载荷.
    pub async fn dispatch(
        &self,
        host: &dyn HostContext,
        command: &str,
        payload: Value,
    ) -> DispatchOutcome {
        (self.dispatch)(host, command, payload).await
    }
}
