//! tar 归档解压.
//!
//! 只负责解包; 分发包内容校验与安装位置由调用方决定.

use std::fs::File;
use std::path::Path;

/// 解压 tar 归档到目录.
pub fn unpack_tar(archive: &Path, destination: &Path) -> std::io::Result<()> {
    tar::Archive::new(File::open(archive)?).unpack(destination)
}

/// 解压 gzip 压缩的 tar 归档到目录.
pub fn unpack_tar_gz(archive: &Path, destination: &Path) -> std::io::Result<()> {
    tar::Archive::new(flate2::read::GzDecoder::new(File::open(archive)?)).unpack(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;
    use std::io::Write;

    fn tar_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, contents) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, *name, *contents)
                .expect("append tar entry");
        }
        builder.into_inner().expect("finish tar")
    }

    #[test]
    fn unpacks_plain_and_gzipped_archives() {
        let dir = TempDir::new("archive");
        let plain = dir.write("runtime.tar", tar_with(&[("umu/umu-run", b"binary")]));
        let target = dir.join("plain");
        unpack_tar(&plain, &target).expect("unpack tar");
        assert_eq!(
            std::fs::read(target.join("umu/umu-run")).unwrap(),
            b"binary"
        );

        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder
            .write_all(&tar_with(&[("proton/proton", b"proton")]))
            .unwrap();
        let gz = dir.write("runtime.tar.gz", encoder.finish().unwrap());
        let target = dir.join("gzip");
        unpack_tar_gz(&gz, &target).expect("unpack tar.gz");
        assert_eq!(
            std::fs::read(target.join("proton/proton")).unwrap(),
            b"proton"
        );
    }

    #[test]
    fn missing_archives_are_reported() {
        let dir = TempDir::new("archive-missing");
        assert!(unpack_tar(&dir.join("missing.tar"), &dir.join("plain")).is_err());
        assert!(unpack_tar_gz(&dir.join("missing.tar.gz"), &dir.join("gzip")).is_err());
    }

    #[test]
    fn gzip_unpack_rejects_a_plain_tar_file() {
        let dir = TempDir::new("archive-plain");
        let plain = dir.write("plain.tar", tar_with(&[("file.bin", b"x")]));
        assert!(unpack_tar_gz(&plain, &dir.join("out")).is_err());
    }
}
