//! MD5 与 SHA-256 流式计算.
//!
//! 只负责摘要, 校验策略与失败修复由调用方决定.

use std::fs;
use std::io::Read;
use std::path::Path;

use md5::{Digest, Md5};
use sha2::Sha256;

use crate::{Error, Result};

/// 分块大小; 保持既有取消粒度与内存占用.
const BUFFER_SIZE: usize = 1024 * 1024;

pub fn md5_bytes(bytes: &[u8]) -> String {
    let mut hasher = Md5::new();
    hasher.update(bytes);
    hex(&hasher.finalize())
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex(&hasher.finalize())
}

pub fn md5_file(path: &Path) -> Result<String> {
    md5_file_cancellable(path, &|| false)
}

pub fn sha256_file(path: &Path) -> Result<String> {
    hash_file_cancellable::<Sha256>(path, &|| false)
}

pub fn md5_file_cancellable(path: &Path, cancelled: &dyn Fn() -> bool) -> Result<String> {
    hash_file_cancellable::<Md5>(path, cancelled)
}

fn hash_file_cancellable<D: Digest>(path: &Path, cancelled: &dyn Fn() -> bool) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = D::new();
    let mut buffer = vec![0u8; BUFFER_SIZE];
    loop {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn md5_matches_known_vectors() {
        assert_eq!(md5_bytes(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_bytes(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(
            sha256_bytes(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hashing_can_be_cancelled_between_read_blocks() {
        let dir = TempDir::new("hash-cancel");
        dir.write("large.bin", vec![7; 4 * 1024 * 1024]);
        let reads = std::cell::Cell::new(0);
        let error = md5_file_cancellable(&dir.join("large.bin"), &|| {
            reads.set(reads.get() + 1);
            reads.get() > 2
        })
        .unwrap_err();
        assert!(matches!(error, Error::Cancelled));
        assert_eq!(reads.get(), 3);
    }

    #[test]
    fn file_hashes_stream_in_chunks() {
        let dir = TempDir::new("hash-file");
        // 超过读缓冲, 确认分块读取与一次性计算一致
        let big = vec![b'x'; BUFFER_SIZE + 7];
        let path = dir.write("big.bin", &big);
        assert_eq!(md5_file(&path).expect("hash md5 file"), md5_bytes(&big));
        assert_eq!(
            sha256_file(&path).expect("hash sha256 file"),
            sha256_bytes(&big)
        );
        assert!(md5_file(&dir.join("missing.bin")).is_err());
        assert!(sha256_file(&dir.join("missing.bin")).is_err());
    }
}
