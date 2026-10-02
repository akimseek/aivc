# 网关统一外部接入 · 设计（待审阅）

> 2026-10-02。实机反馈第 4 项（「本地网关与外部网关是不是可以合成一个功能」）的
> 落地设计。前 3 项修复见 `docs/rust-rewrite/48-terminal-copy-and-vendor-ui.md`。
> **状态：待用户审阅**（审阅通过后出实施计划再动代码）。

## 1. 背景与现状（评估结论）

查证结果（`crates/aivc-gateway` 与 UI 现状）：

- **后端本来就是一个网关**：单个 axum 服务器（`/gateway` 前缀 + fallback 解析），
  一套供应商池 / 故障转移 / 熔断 / 规则 / 用量 / 日志。「本地」与「外部」只是两种凭据：
  - 终端路由：`/gateway/<平台>/<终端编号>`，每终端一条、本地令牌、动态候选池
    （终端编号即 routeId，由 `vendor:route-*` 注入 CLI 时注册）；
  - 外部接入：共享令牌（sha256、默认关闭、`?key=` 从上游与日志剔除），
    骨架在 `routes.rs`/`server.rs` 完整存在（`route.external` 分支、外部路由索引）。
- **外部接入从未真正落地**：没有任何生产代码注册过外部路由
  （`VendorRoute.external=true` 仅存在于测试），也没有命令把外部地址读给界面——
  因此 UI 只能拿占位符拼出 `…/gateway/codex/<routeId>`，该地址**必然 404**。
- 旧 Electron 版的外部形态是 `/external/<平台>`（`findExternalRoute` 前缀匹配，
  惰性路由 `external-<平台>`），同样是「路径=平台」的语义。

## 2. 目标与边界

**做**：

1. 外部接入路径改为 `/<平台>`：`http://127.0.0.1:<port>/codex` 直达 codex 候选池，
   `/claude`、`/gemini`、`/qoder` 同理；`/codex/v1/responses` 这类**协议后缀原样透传**
   （协议由接入方与供应商 API 地址决定，网关不参与）。
2. 「合并」落为事实：单服务器不变；外部接入补完为网关的一等接入方式；
   界面去掉「本地/外部」对立措辞。
3. 把外部端点地址在界面上摆正：每平台一条 `{网关根地址}/<平台>`，无占位符。

**不做**：

- 不改终端路由形制（`/gateway/<平台>/<编号>` 仍需绑定供应商，保持 parity 矩阵 G1 契约）。
- 不合并设置页的卡片划分（端口/外部/规则三卡保留，只统一措辞与语义）。
- 不引入「外部路径带 routeId」「外部侧选池」等新概念——外部永远是平台动态池。
- 不放开监听范围：仍仅 `127.0.0.1`（「外部」= 本机其它程序，不含局域网）。

## 3. 设计

### 3.1 路由解析（`crates/aivc-gateway/src/routes.rs` / `server.rs`）

- 判定分支（`handle_inner` 的路由解析处）：路径第一段（segment 0）
  - `gateway` → 走现有 `/gateway/<平台>/<编号>` 逻辑，一字不动；
  - 平台白名单（`parse_provider`：codex/claude/gemini/qoder）→ **外部接入分支**；
  - 其余 → 404 `ROUTE_NOT_FOUND`（现状行为，日志记因）。
- **合成外部路由**：每平台一条，`route_id = "external-<平台>"`（与旧平台一致，
  日志里一眼可辨）、`external: true`、`mode: Dynamic`、`vendor_id` 空。
  **惰性注册**（首次命中该平台时注册，双检加锁）——与旧平台一致，启动路径零新增。
- `VendorRoute::prefix()` 增加 external 分支：返回 `/<平台>`；
  既有 `strip_route_prefix`、日志 route 标识因此自动适配。
- 透传：suffix = 去掉 `/<平台>` 的余下路径，`join_upstream_url` 现有逻辑照用
  （`/v1` 去重、拒非 http(s) base、拒 query/fragment）。`/codex`（无后缀）→ 上游取 base 原地址。

### 3.2 鉴权与行为（复用现有代码路径，零改动）

- 未启用外部接入 → `503 EXTERNAL_API_DISABLED`；无/错令牌 → `401 UNAUTHORIZED`
  （候选凭据：`Authorization: Bearer` / `x-api-key` / `x-goog-api-key` / `?key=`，
  常量时间哈希比较）；候选池为空 → `503 VENDOR_UNAVAILABLE`。
- `?key=` 从转发查询与日志路径中剔除（现有 `sanitize_external_request_path`）。
- 请求照常落库（`route_id=external-<平台>`）、照常走故障转移/熔断/规则；
  外部请求**不发**供应商切换通知（现状行为，无界面订阅）。
- 客户端协议头透传（剔除逐跳头，按平台重建上游凭据）——现有转发逻辑照用。

### 3.3 UI（设置中心）

- `gateway-external-card.tsx`：
  - 端点清单每平台显示 `{baseUrl}/<平台>`（如 `http://127.0.0.1:8787/codex`）；
  - 删除 `ROUTE_ID_PLACEHOLDER` 与引用 `routes.rs` 占位注释；说明文案改为：
    「把该地址作为 API 基地址；协议路径（如 `/v1/...`）由接入方按自己的协议追加，
    网关原样透传。」；
  - 保留：启用开关、共享令牌（轮换/只展示一次）、平台凭据头提示、「仅监听 127.0.0.1」提示。
- 命名统一：`settings-nav.ts` 的「本地网关 & 路由」→「网关」；
  卡标题「外部网关与 API 接入」→「外部接入」；面板描述同步（不再有「本地/外部」对立）。
- 端口卡不变（固定端口才让外部地址稳定，现有提示保留）。

### 3.4 兼容与偏离记录

- 相对旧版 `/external/<平台>` 的**刻意偏离**：去掉 `/external` 前缀（更短、
  与用户要求一致）。旧版形制在 Rust 版从未可用，无迁移负担；记入 parity 偏离清单，
  40/41 号文档与 G 矩阵注记同步。
- G41 令牌语义（默认关、sha256、只展示一次、`?key=` 剔除）与终端路由契约全部不变。

## 4. 测试

**Rust（aivc-gateway 端到端，沿用 server.rs 现有手法）**：

1. 未启用：`GET /codex/v1/responses` → 503 `EXTERNAL_API_DISABLED`；
2. 令牌错 → 401；三种头 + `?key=` 各一例（至少抽查两处）；
3. 启用+令牌：请求 `…/codex/v1/responses?stream=true` → 200，上游收到
   `/v1/responses`（去前缀、`/v1` 去重整一例），`key` 不在转发查询与日志里；
4. 未知平台段 → 404；`/gateway/...` 现有用例全数回归（互不影响）；
5. 外部路由的 `route_id=external-<平台>` 出现在请求记录里。

**前端**：`gateway-external-card.test.tsx` 更新（地址形如 `{base}/codex`、无 `<routeId>`、
文案断言）；`settings-nav` 相关断言更新。

**真机（可选）**：启用外部接入后用 curl 打 `/codex`（或复用 gateway 探针）验证走候选池与阻塞行为。

## 5. 影响面

| 区域 | 文件 | 改动 |
| --- | --- | --- |
| 网关 | `crates/aivc-gateway/src/routes.rs` | `prefix()` external 分支、外部路由构造 |
| 网关 | `crates/aivc-gateway/src/server.rs` | 解析分支 + 惰性注册、测试 |
| 前端 | `ui/src/features/settings/panels/gateway-external-card.tsx` | 端点构建与文案 |
| 前端 | `ui/src/features/settings/panel-header.tsx` 等 | 命名统一 |
| 文档 | `docs/rust-rewrite/{03,40,41}`、48 | 偏离记录与注记 |

无 IPC 命令面改动（端点地址由 UI 从 `baseUrl` + 平台清单拼出）。

## 6. 留给审阅的问题

1. 导航名从「本地网关 & 路由」改为「网关」，卡名改为「外部接入」——是否接受？
2. 外部端点清单保留「凭据头提示」一列（现状有，本设计保留）——是否还需要？
3. 外部接入默认关闭 + 共享令牌的现状**保持不变**（安全姿态与 parity 一致）——确认？
