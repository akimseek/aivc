//! `reqwest` 客户端适配器：模型列表与余额查询。
//!
//! 统一使用 `rustls`，避免跨平台 OpenSSL 分发问题（`deny.toml` 同时禁止 `openssl-sys`）。
//! 所有请求都必须带超时与体积上限：现网模型查询为 15 s / 2 MB / 最多 500 条。
//!
//! 计划子模块：`client`（超时与上限统一的请求封装）、`model_query`、`balance_query`。
