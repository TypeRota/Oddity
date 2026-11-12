//! 更新域使用的路径原语.
//!
//! 清单与预下载缓存里的单段路径校验属于更新协议; 文件安全原语来自平台.

pub(crate) use oddity_platform::paths::{ensure_no_symlink, TemporaryFile};

/// 预下载分支/版本等单段路径只允许字母数字和 ._-, 且不能是 . 或 ...
pub(crate) fn valid_segment(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}
