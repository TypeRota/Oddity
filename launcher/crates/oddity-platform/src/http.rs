use std::io::Read;
use std::sync::OnceLock;
use std::time::Duration;

use crate::{Error, Result, USER_AGENT};

/// 复用一个连接池：清单、补丁和整包下载共享 keep-alive 连接。
pub struct HttpClient {
    agent: ureq::Agent,
}

impl HttpClient {
    /// 启动器文件下载共用同一个连接池，避免每个小文件重新握手。
    pub fn shared() -> &'static HttpClient {
        static CLIENT: OnceLock<HttpClient> = OnceLock::new();
        CLIENT.get_or_init(HttpClient::new)
    }

    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .max_idle_connections(16)
            .max_idle_connections_per_host(8)
            .timeout_global(Some(Duration::from_secs(120)))
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }

    pub fn request(
        &self,
        url: &str,
        timeout: Duration,
    ) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        self.request_as(url, timeout, USER_AGENT)
    }

    /// 构造 GET 请求, 用调用方给出的 User-Agent 覆盖启动器标识.
    ///
    /// 官方页面与社交平台按浏览器身份响应, 游戏域需要在不脱离共享连接池的
    /// 前提下伪装成普通浏览器.
    pub fn request_as(
        &self,
        url: &str,
        timeout: Duration,
        user_agent: &str,
    ) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        self.agent
            .get(url)
            .header("User-Agent", user_agent)
            .config()
            .timeout_global(Some(timeout))
            .build()
    }

    /// 构造 POST 请求, 用调用方给出的 User-Agent 覆盖启动器标识.
    ///
    /// 游戏协议以表单提交并自行判定状态码; 重定向与状态码语义由调用方在
    /// 返回的构建器上配置, 连接池与 GET 共用.
    pub fn post_as(
        &self,
        url: &str,
        timeout: Duration,
        user_agent: &str,
    ) -> ureq::RequestBuilder<ureq::typestate::WithBody> {
        self.agent
            .post(url)
            .header("User-Agent", user_agent)
            .config()
            .timeout_global(Some(timeout))
            .build()
    }

    pub fn fetch_bytes(&self, url: &str, limit: u64, timeout: Duration) -> Result<Vec<u8>> {
        let mut response = self
            .request(url, timeout)
            .call()
            .map_err(|error| Error::Http(format!("GET {url}: {error}")))?;
        let mut bytes = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(limit)
            .read_to_end(&mut bytes)
            .map_err(|error| Error::Http(format!("read {url}: {error}")))?;
        Ok(bytes)
    }
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new()
    }
}

/// 提取 HTTP 状态码; 连接、超时与其他传输错误没有状态码.
///
/// 需要按状态码给出不同提示的调用方用它区分, 不必依赖底层 HTTP 库的错误类型.
pub fn status_code(error: &ureq::Error) -> Option<u16> {
    match error {
        ureq::Error::StatusCode(code) => Some(*code),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_code_reports_only_http_status_errors() {
        assert_eq!(status_code(&ureq::Error::StatusCode(429)), Some(429));
        let io = ureq::Error::Io(std::io::Error::other("offline"));
        assert_eq!(status_code(&io), None);
    }
}
