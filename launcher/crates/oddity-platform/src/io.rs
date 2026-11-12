//! 带取消检查的流复制.

use std::io::{Read, Write};

use crate::{Error, Result};

/// 分块复制并返回传输字节数; 每块之前检查取消信号.
pub fn copy_cancellable(
    reader: &mut impl Read,
    writer: &mut impl Write,
    cancelled: &dyn Fn() -> bool,
) -> Result<u64> {
    let mut buffer = vec![0; 1024 * 1024];
    let mut total = 0;
    loop {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let size = reader.read(&mut buffer)?;
        if size == 0 {
            return Ok(total);
        }
        writer.write_all(&buffer[..size])?;
        total += size as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_every_byte_and_reports_the_size() {
        let mut input = std::io::Cursor::new(b"payload".to_vec());
        let mut output = Vec::new();
        let copied = copy_cancellable(&mut input, &mut output, &|| false).expect("copy");
        assert_eq!(copied, 7);
        assert_eq!(output, b"payload");
    }

    #[test]
    fn cancellation_stops_before_reading() {
        let mut input = std::io::Cursor::new(vec![1u8; 2 * 1024 * 1024]);
        let mut output = Vec::new();
        let checks = std::cell::Cell::new(0);
        let error = copy_cancellable(&mut input, &mut output, &|| {
            checks.set(checks.get() + 1);
            true
        })
        .unwrap_err();
        assert!(matches!(error, Error::Cancelled));
        assert_eq!(checks.get(), 1);
        assert!(output.is_empty());
    }

    #[test]
    fn cancellation_between_blocks_keeps_written_bytes() {
        let mut input = std::io::Cursor::new(vec![7u8; 2 * 1024 * 1024]);
        let mut output = Vec::new();
        let checks = std::cell::Cell::new(0);
        let error = copy_cancellable(&mut input, &mut output, &|| {
            checks.set(checks.get() + 1);
            checks.get() > 1
        })
        .unwrap_err();
        assert!(matches!(error, Error::Cancelled));
        assert_eq!(output.len(), 1024 * 1024);
    }
}
