//! 网络计费状态查询: 通过 NetworkManager 的 Metered 属性判断当前连接.

use std::time::Duration;

/// NetworkManager 报告的计费状态.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metered {
    Yes,
    No,
    Unknown,
}

impl Metered {
    /// 只有明确 (或猜测) 为计费网络时才视为计费, 未知状态按不计费处理.
    pub fn is_metered(self) -> bool {
        matches!(self, Metered::Yes)
    }
}

/// 查询当前默认连接的计费状态; 查询失败返回未知.
pub fn metered() -> Metered {
    query_metered().unwrap_or(Metered::Unknown)
}

fn query_metered() -> Option<Metered> {
    let connection = zbus::blocking::connection::Builder::system()
        .ok()?
        .method_timeout(Duration::from_secs(2))
        .build()
        .ok()?;
    let proxy = zbus::blocking::Proxy::new(
        &connection,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager",
        "org.freedesktop.NetworkManager",
    )
    .ok()?;
    let value: u32 = proxy.get_property("Metered").ok()?;
    Some(classify(value))
}

/// 按 NetworkManager 的 NMMetered 定义归类.
fn classify(value: u32) -> Metered {
    match value {
        // YES 与 GUESS_YES 都按计费处理, 这是 NetworkManager 文档的建议.
        1 | 3 => Metered::Yes,
        2 | 4 => Metered::No,
        _ => Metered::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metered_values_follow_networkmanager_semantics() {
        assert_eq!(classify(0), Metered::Unknown);
        assert_eq!(classify(1), Metered::Yes);
        assert_eq!(classify(2), Metered::No);
        assert_eq!(classify(3), Metered::Yes);
        assert_eq!(classify(4), Metered::No);
        assert_eq!(classify(9), Metered::Unknown);
        assert!(classify(3).is_metered());
        assert!(!classify(4).is_metered());
    }
}
