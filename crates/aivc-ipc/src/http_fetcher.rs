//! 基于 `reqwest` 的 HTTP 执行器。
//!
//! # 为什么网络失败返回状态码 0 而不是 `Err`
//!
//! 与 CLI 探测同理：探测的语义是「收集事实」而不是「传播错误」。把「连不上」
//! 表达成一个状态码，上层就能统一按状态码分类，而不必同时处理两种失败通道。
//!
//! 用 0 而不是 5xx 是因为：5xx 是**服务端**返回的，会被计入供应商熔断；
//! 连不上不该算服务端的错。
//!
//! # 为什么不需要额外的超时
//!
//! [`reqwest::Client`] 自身带连接与总超时配置。这里额外设一个**总超时**，
//! 因为中转站可能在建立连接后长时间不返回——只靠连接超时挡不住。

use std::time::Duration;

use aivc_core::ports::BoxFuture;
use aivc_tooling::vendor_query::{HttpFetcher, HttpRequest, HttpResponse, STATUS_NETWORK_ERROR};

/// 单次请求的总超时。
///
/// 比上层的查询超时（15 秒）略短：让这里的超时先触发，从而得到一个**明确的
/// 「超时」状态码**，而不是让上层在自己的计时器上看到「没有响应」。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);

/// 基于 `reqwest` 的执行器。
pub struct ReqwestFetcher {
    client: reqwest::Client,
}

impl Default for ReqwestFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl ReqwestFetcher {
    /// 构造。
    #[must_use]
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            // 不跟随重定向：中转站的重定向常指向登录页，跟过去会拿到一段 HTML，
            // 报出的错（「不是有效 JSON」）与真实原因（需要认证）完全无关。
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default();
        Self { client }
    }
}

impl HttpFetcher for ReqwestFetcher {
    fn fetch(&self, request: HttpRequest) -> BoxFuture<'_, HttpResponse> {
        Box::pin(async move {
            let mut builder = if request.method == "POST" {
                self.client.post(&request.url)
            } else {
                self.client.get(&request.url)
            };
            for (name, value) in &request.headers {
                builder = builder.header(name.as_str(), value.as_str());
            }
            if let Some(body) = request.body {
                builder = builder.body(body);
            }
            match builder.send().await {
                Ok(response) => {
                    let status = response.status().as_u16();
                    // 读体失败同样按网络失败处理：一个读不出内容的响应没有价值。
                    match response.text().await {
                        Ok(body) => HttpResponse { status, body },
                        Err(error) => {
                            tracing::debug!(%error, "读取响应体失败");
                            HttpResponse {
                                status: STATUS_NETWORK_ERROR,
                                body: String::new(),
                            }
                        }
                    }
                }
                Err(error) => {
                    // 记 debug 而不是 warn：用户填错地址时每次查询都会走到这里，
                    // 记成 warn 会淹没真正的告警。
                    tracing::debug!(%error, url = %request.url, "请求失败");
                    HttpResponse {
                        status: STATUS_NETWORK_ERROR,
                        body: String::new(),
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_executor_can_be_constructed() {
        // 构造不应失败；失败时退回默认客户端（探测能用比构造报错更实际）。
        let fetcher = ReqwestFetcher::new();
        let _ = &fetcher.client;
    }

    #[tokio::test]
    async fn an_unreachable_host_is_reported_as_the_network_error_status() {
        // 连不上不该算服务端的错，因此用 0 而不是 5xx。
        let fetcher = ReqwestFetcher::new();
        let response = fetcher
            .fetch(HttpRequest {
                method: "GET",
                // 保留地址段：RFC 2606 保证它不会被解析到真实主机。
                url: "http://nonexistent.invalid/models".to_owned(),
                headers: Vec::new(),
                body: None,
            })
            .await;
        assert_eq!(response.status, STATUS_NETWORK_ERROR);
        assert!(response.body.is_empty());
    }

    #[test]
    fn the_request_timeout_is_shorter_than_the_query_timeout() {
        // 让这里的超时先触发，从而得到一个明确的状态码，
        // 而不是让上层自己的计时器看到「没有响应」。
        assert!(REQUEST_TIMEOUT.as_secs() < 15);
    }
}
