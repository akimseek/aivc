//! 日志初始化与轮转。
//!
//! 统一日志出口：`tracing` 订阅器 + 按大小轮转的文件写入。**绝不记录** API Key、
//! 请求/响应正文、终端输出与会话正文——这是功能一致与安全契约的一部分。
//!
//! 对应现网 `core/performance.ts`（启动阶段埋点）与 `gateway/gateway-log.ts` 的文件部分
//! （现网为 `gateway.log` + `gateway.log.1`，单文件 2 MB）。
