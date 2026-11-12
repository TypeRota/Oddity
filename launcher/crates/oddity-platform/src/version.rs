//! 点分数字版本比较.
//!
//! 官方清单里的版本形如 `1.0.7.0924`, 按数字段比较, 非数字段按 0 处理,
//! 与官方启动器的宽松解析保持一致.

use std::cmp::Ordering;

/// 比较两个点分数字版本; 段数不同时缺失段按 0 处理.
pub fn compare(a: &str, b: &str) -> Ordering {
    let pa: Vec<u64> = a.split('.').map(|x| x.parse().unwrap_or(0)).collect();
    let pb: Vec<u64> = b.split('.').map(|x| x.parse().unwrap_or(0)).collect();
    for i in 0..pa.len().max(pb.len()) {
        let x = *pa.get(i).unwrap_or(&0);
        let y = *pb.get(i).unwrap_or(&0);
        match x.cmp(&y) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    Ordering::Equal
}

/// 远端版本比本地新时返回 true.
pub fn is_newer(local: &str, remote: &str) -> bool {
    compare(local, remote) == Ordering::Less
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_dotted_versions_numerically() {
        assert_eq!(compare("1.10.0", "1.9.9"), Ordering::Greater);
        assert_eq!(compare("1.0", "1.0.0"), Ordering::Equal);
        assert_eq!(compare("1.0.0.1", "1.0"), Ordering::Greater);
        assert_eq!(compare("0.9", "1"), Ordering::Less);
        // 非数字段按 0 处理，和官方启动器的宽松解析保持一致
        assert_eq!(compare("1.x", "1.0"), Ordering::Equal);
        assert!(is_newer("1.0.0", "1.0.1"));
        assert!(!is_newer("1.0.1", "1.0.0"));
        assert!(!is_newer("1.0", "1.0"));
    }
}
