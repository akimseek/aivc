# 49 · 网关统一外部接入（外部路径 /<平台>）

> 2026-10-02。实机反馈第 4 项的落地记录。设计与用户裁定见
> `docs/superpowers/specs/2026-10-02-gateway-unified-external-design.md`（§6 为裁定）；
> 实施计划见 `docs/superpowers/plans/2026-10-02-gateway-unified-external.md`。

## 一、做了什么

- **外部接入落地**：`/<平台>`（如 `http://127.0.0.1:8787/codex`）直达该平台候选池。
  每平台一条**惰性注册**的合成路由（`external-<平台>`、`external: true`、动态池），
  首段即平台（`routes.rs` 的 `extract_external_provider`）；`/gateway/<平台>/<编号>`
  终端分支一字未动。
- **前缀剥离复用**：`VendorRoute::prefix()` 对外部路由返回 `/<平台>`，
  `strip_route_prefix` / 日志 route 标识 / `?key=` 脱敏全部沿用。
- **鉴权零改动**：默认关闭 + 共享令牌（sha256）；未启用 503 `EXTERNAL_API_DISABLED`、
  令牌错 401；`?key=` 不进上游与日志。
- **UI 收口**：端点清单从占位符 `/gateway/<平台>/<routeId>` 改为 `{网关根地址}/<平台>`
  （编号占位符与凭据头列移除——后者为用户 2026-10-02 裁定）；导航「本地网关 & 路由」→
  「网关」，卡「外部网关与 API 接入」→「外部接入」，面板/开关去「本地」对立措辞。

## 二、刻意偏离

| 项 | 旧 Electron 版 | 本实现 | 理由 |
| --- | --- | --- | --- |
| 外部路径 | `/external/<平台>` | `/<平台>` | 更短；用户 2026-10-02 指定形制 |

（G41 令牌语义、终端路由契约不变；03 号 G41 段与 40/41 号已加注记。）

## 三、实现备注（两处对计划的修正，落代码时发现）

1. **请求记录只存路径、不含查询串**：计划里断言 `record.path` 含 `stream=true`，
   实际 `RequestTracker::start` 收的是 `uri().path()`（`server.rs:510`）——修正为
   「路径落库 + 令牌字符串绝不出现」两条断言。`?key=` 在日志里的外一层防护仍由
   `sanitize_external_request_path` 的单测覆盖。
2. **合成外部路由的 `base_url` / `local_token` / `vendor_id` 留空**：`route.base_url`
   仅进 `Debug` 输出（`routes.rs:84`），转发用的上游地址取自候选供应商
   （`attempt_vendor.api_base_url`），故此三项对外部路由无意义。

## 四、证据

- Rust：`routes.rs` 4 条新单测（首段判平台 / 裸前缀 / 惰性注册幂等 / 编号命名）+
  `server.rs` 4 条端到端（真 HTTP：未启用 503、令牌 401 且 x-api-key 对照通过、
  成功 200 且上游收到去前缀 `/v1/responses`、记录 `route_id=external-codex`、
  未知一段 404）；全 workspace 测试通过。
- 前端：`gateway-external-card.test.tsx` 按新契约更新（端点形如 `{base}/codex`、
  无 `.gateway-endpoint-auth`、无 `<routeId>`）；设置中心相关断言同步；tsc / vitest /
  build 全绿。
- 门禁：`verify.ps1 -Quick` 八段通过。

## 五、提交

- `5a7b02b` routes.rs；`6f3d3e7` server.rs；`6586e9d` UI 端点；`5deaead` UI 命名；
  本文件与注记随收尾提交。
