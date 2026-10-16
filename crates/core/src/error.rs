use thiserror::Error;

#[derive(Debug, Error)]
pub enum ODataError {
    #[error("HTTP 请求错误：{}", describe_http_error(.0))]
    Http(#[from] reqwest::Error),

    #[error("元数据解析失败：{0}")]
    MetadataParse(String),

    #[error("获取 CSRF 令牌失败：{0}")]
    CsrfFetch(String),

    #[error("认证失败：{0}")]
    AuthFailed(String),

    #[error("未找到实体：{0}")]
    EntityNotFound(String),

    #[error("未找到服务：{0}")]
    ServiceNotFound(String),

    /// Non-success HTTP status ("server returned 400 Bad Request …") or
    /// an unparseable body ("invalid JSON response: …"). The message is
    /// self-describing; no prefix, since most of these are not parse
    /// failures at all.
    #[error("{0}")]
    ResponseParse(String),

    #[error("URL 无效：{0}")]
    InvalidUrl(#[from] url::ParseError),
}

/// reqwest's own Display stops at "error sending request for url (…)",
/// hiding *why*. Append the innermost cause (connection refused, DNS
/// failure, certificate problem …) and say so plainly on timeouts.
fn describe_http_error(e: &reqwest::Error) -> String {
    let mut text = e.to_string();
    let mut root: &dyn std::error::Error = e;
    while let Some(next) = root.source() {
        root = next;
    }
    let cause = root.to_string();
    if !std::ptr::addr_eq(root, e as &dyn std::error::Error) && !text.contains(&cause) {
        text.push_str(": ");
        text.push_str(&cause);
    }
    if e.is_timeout() {
        text.push_str(&format!(
            " ——服务器未能及时响应（设置 {} 可延长读取超时时间，设为 0 可禁用超时）",
            crate::client::READ_TIMEOUT_ENV
        ));
    }
    text
}
