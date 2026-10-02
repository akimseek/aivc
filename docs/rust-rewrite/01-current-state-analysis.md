# AI Visual Console — 现状分析（Rust 重构基线）

> 分析对象：`D:\code\ai` @ v1.2.0（Electron 39 + React 19 + TypeScript 5.9）
> 分析日期：基线快照
> 目的：为 Rust 重写提供**行为契约级**的现状清单，而不是代码风格评审。

---

## 1. 规模与结构

| 区域 | 生产文件 | 生产行数 | 测试文件 | 测试行数 |
| --- | ---: | ---: | ---: | ---: |
| `src/electron`（主进程） | 56 | 12,755 | — | — |
| `src/renderer`（渲染层） | 101 | 19,319 | — | — |
| `src/shared`（共享契约） | 9 | 1,330 | — | — |
| **合计** | **166** | **33,404** | **54** | **4,555** |

关键体积分布（重构时的风险重心）：

| 文件 | 行数 | 风险 |
| --- | ---: | --- |
| `src/renderer/src/styles/app.css` | 8,214 | 中（设计系统需完整保留） |
| `src/electron/providers/codex/codex-targets.ts` | 1,234 | **极高**（官方 app-server fork 时序） |
| `src/renderer/src/app/app.tsx` | 1,193 | 中（状态编排枢纽） |
| `src/electron/providers/gemini/gemini-provider.ts` | 916 | 高 |
| `src/electron/gateway/vendor-gateway.ts` | 885 | **极高**（HTTP 语义） |
| `src/electron/vendors/vendor-manager.ts` | 848 | 高（写用户 CLI 配置） |
| `src/electron/providers/claude/claude-provider.ts` | 800 | 高 |
| `src/electron/terminal/terminal-sessions.ts` | 722 | 高（命令拼装） |
| `src/electron/providers/qoder/qoder-provider.ts` | 675 | 中高（多目录迁移） |
| `src/renderer/.../vendor-manager-dialog.tsx` | 998 | 中 |

技术栈：Electron / React / TypeScript / `node:sqlite`（WAL）/ xterm.js 6 / node-pty / Vite / Vitest。

---

## 2. 产品功能域（重构必须 1:1 覆盖）

```
┌─ 桌面外壳 ───────────────────────────────────────────────┐
│ 菜单栏 + 命令面板(Ctrl+K) + 会话提醒铃铛 + 托盘 + 关闭确认  │
├─ 侧栏 ────────────────────┬─ 工作区 ────────────────────┤
│ 工作台 / 会话              │ 终端标签(AI 会话 / 系统终端)  │
│  · 当前会话 / 收藏夹 / 回收站│  · xterm.js 渲染             │
│  · 本地搜索 + 全局会话检索  │  · 自管输入区(Composer)       │
│  · 目标(本机/PowerShell/   │  · 搜索栏 / 右键菜单          │
│    Git Bash/WSL 发行版)    │  · 会话详情弹窗(分页/分支)     │
├───────────────────────────┴─────────────────────────────┤
│ 状态栏：会话 / 目录 / 当前供应商(路由入口) / Token 用量      │
└─────────────────────────────────────────────────────────┘
浮层：供应商管理、网关设置、故障规则、用量与费用、日志清理、
      异常诊断、Skill 管理、CLI 安装、会话设置、关于、退出确认
```

**四大业务域**：① 会话资产（4 个 AI CLI）② 本地 Gateway 与供应商路由 ③ 终端运行时（PTY）④ 平台集成（WSL / CLI 环境 / Skill）。

---

## 3. 主进程架构现状

### 3.1 进程生命周期（`main.ts`）

- 启动顺序：解析运行根 → `app.setPath("userData", <root>/data/user-data)` → 设路径注入各模块 → `startVendorGatewayIfEnabled()` → 注册 8 组 IPC → `createWindow()`。
- 窗口安全：`contextIsolation: true` / `nodeIntegration: false` / `sandbox: true`；打包环境强制 CSP；`will-navigate` 白名单；`setWindowOpenHandler` 拒绝并转系统浏览器；禁用 webview。
- 退出：`close` → 渲染层确认框（最小化到托盘 / 直接退出 / 取消）→ `before-quit` 拦截 → `shutdownApplication()` 串行等待 `stopAllTerminalSessions()` → `stopVendorGateway()` → `flushGatewayLogs()`，完成后才真正 `app.quit()`。
- 性能埋点：`performance.log` 记录 `window.create.start` / `ready-to-show` / `did-finish-load` / `did-fail-load` / `render-process-gone` / `console-error` / `renderer.mounted`。

### 3.2 路径模型（`core/application-paths.ts` + `core/main-helpers.ts`）

单一运行根，三种形态派生：

| 形态 | 运行根 |
| --- | --- |
| 开发（`VITE_DEV_SERVER_URL`） | 工作目录 |
| Windows 未打包（`release/win-unpacked`） | `release` 的父目录 |
| 安装包 | 安装目录旁的运行根 |

派生：`data/`（SQLite `app.db`、`cache/`、`vendor-backups/`）、`logs/`（`gateway.log`、`performance.log`）、`data/user-data/`（Electron userData + `settings.json` + `session-metadata.json`）。

**不变量**：应用自身产物绝不写入系统用户目录；但官方 CLI 的 `~/.codex`、`~/.claude`、`~/.gemini`、`~/.qoder-cn` 是产品明确依赖的用户数据边界，必须读写。

### 3.3 持久化（`core/app-database.ts` + 各域 schema）

单文件 SQLite（`node:sqlite`，WAL + `busy_timeout=5000`），**所有写入经全局串行队列**（`BEGIN IMMEDIATE` / `COMMIT` / `ROLLBACK`）。

表清单（重构必须完整迁移，含历史兼容列）：

| 表 | 主键 | 用途 |
| --- | --- | --- |
| `session_cache` | `(cache_key, file_path)` | 会话摘要缓存；列 `mtime_ms, change_ms, size, session_json, cached_at` |
| `session_metadata` | `(target_id, session_id)` | `customTitle / branch / favorite / lastOpenedAt` |
| `session_message_index` | `(target_id, session_id, file_path, mtime_ms, size, message_offset)` | 分页锚点（`line_number`），只存位置不存正文 |
| `workspace_presets` | `id` | 工作区预设 |
| `api_vendors` | `id`，唯一索引 `(provider_id, name_norm)`、`(provider_id, sort)` | 供应商（含 `api_key` 明文、价格、`enabled`、`last_enabled_at`） |
| `api_vendor_configs` | `id`，FK→vendor | 写入 CLI 的配置模板（`target_path`, `content`） |
| `api_vendor_query_configs` | `vendor_id` | 模型/余额查询 JSON 配置 |
| `gateway_vendor_health` | `vendor_id` | 熔断健康状态 |
| `gateway_request_logs` | `request_id` | 请求日志 + `usage_json` |
| `gateway_failover_rules` | `id` | 故障识别规则（含自定义响应） |

缓存淘汰：`session_cache` 上限 4,000 条 / 64 MB，单条 1 MB，按 `cached_at` 升序 LRU。
升级策略：新增列用 `ALTER TABLE` + `try/catch`；`change_ms` / `line_number` 缺失时**强制清空重建**。

### 3.4 设置（`core/settings.ts`）

`settings.json`（原子写：临时文件 + `rename`），内存缓存单次读盘。字段与默认值：

| 字段 | 默认 | 校验范围 |
| --- | --- | --- |
| `gatewayEnabled` | `true`（`!== false`） | 布尔 |
| `gatewayPort` | `0`（自动分配） | 0–65535 整数 |
| `gatewayFailureThreshold` | `1` | 1–10 |
| `gatewayCircuitFailureThreshold` | `3` | 1–20 |
| `gatewayCircuitDurationSeconds` | `60` | 10–86400 |
| `gatewayExternalApiEnabled` | `false` | 布尔 |
| `gatewayExternalApiTokenHash` | 空 | sha256 hex |
| `wslCodexHomes` | `{}` | `{distro: path}` |
| `cachedTargets` | `[]` | 按 provider 合并覆盖 |

损坏配置降级为空配置 + 备份 `<path>.broken-<ts>`，且**缓存降级结果**（避免网关热路径反复读盘）。

---

## 4. Provider 子系统（`src/electron/providers`）

### 4.1 注册契约

`AiProvider` trait = 静态信息（`id` / `label` / `capabilities`）+ 18 个会话操作。注册表为「定义表 + 工厂」，跨平台查询走 `listAcrossProviders`（`Promise.all` 并发），按目标分发走 `dispatchToTarget`。

能力矩阵：

| 能力 | codex | gemini | claude | qoder |
| --- | :-: | :-: | :-: | :-: |
| skills | ✅ | — | — | — |
| branch | ✅ | ✅ | ✅ | — |
| usage | ✅ | ✅ | ✅ | ✅ |
| trash | ✅ | ✅ | ✅ | ✅ |
| batchActions | ✅ | ✅ | ✅ | ✅ |
| customCwd | ✅ | ✅ | ✅ | ✅ |
| export | ✅ | ✅ | ✅ | ✅ |
| sessionSettings | ✅ | — | — | — |
| duplicate | ✅ | ✅ | ✅ | — |
| vendorManagement | ✅ | ✅ | ✅ | — |

不支持的能力必须保留**显式拒绝实现**（契约测试固化）。

### 4.2 Target ID 语法（`shared/target-ids.ts`）

- **Codex 历史裸前缀**：本机 `local`，WSL `wsl:<distro>`。
- 其余：`<provider>:wsl:<distro>`（`claude:wsl:` / `gemini:wsl:` / `qoder:wsl:`）。
- `getProviderIdFromTargetId` 对未知前缀**兜底为 `codex`**（兼容行为，契约测试刻意不固化）。
- 解析函数只认本 provider 前缀，禁止模糊匹配他方前缀。

### 4.3 磁盘布局

| Provider | 会话根 | 回收站 | 备注 |
| --- | --- | --- | --- |
| Codex | `$CODEX_HOME/sessions/**/rollout-<ts>-<uuid>.jsonl` | `<home>/.visual-console-trash/sessions/…`（保持相对路径） | 另有 `history.jsonl` 追加 |
| Claude | `~/.claude/projects/<dir>/*.jsonl` | `<home>/.visual-console-trash/projects/…` | 额外读 telemetry 推上下文窗口 |
| Gemini | `~/.gemini/tmp/<hash>/chats/*.json` + 项目映射 | 回收站镜像 | 需校验 `tmp` 与项目映射 |
| Qoder | `~/.qoder-cn/` 下 `projects` / `tasks` / `file-history` / `logs` | — | 一会话关联多目录，需整体迁移 + 回滚 |

### 4.4 解析与分页

- Codex JSONL 记录类型：`session_meta`（`id/timestamp/cwd/model/cli_version`）、`event_msg`（`user_message` / `agent_message` / `token_count` / `task_started`）、`response_item`（`payload.type === "message"`）。
- 列表用 **64 KB 头部**轻量解析（`parseSessionListContent`，最多留 8 条预览 + 计数）；详情用全量流式解析（`createSessionContentParser`）。
- 合成消息过滤：`<environment_context>` / `<skill>` / `<user_instructions>` / `<developer_instructions>` / 多代理主代理提示块。
- 连续同 `role+text` 去重。
- 标题 = 第一条合格 user 消息，`clampText(…, 88)`。
- 用量：Codex 取 `event_msg.token_count`（`total_token_usage` / `last_token_usage` / `model_context_window` / `rate_limits.primary|secondary`）；Claude / Gemini / Qoder 各有独立累加器。
- **分页算法**（`getSessionMessagesPage`）：`offset === -1` 表示首屏取末尾 `limit` 条并回填真实偏移；否则从 `session_message_index` 取 `message_offset <= offset` 的最近锚点行号起读；每 100 条可见消息写一个锚点；返回 `{offset, messages, hasMore}`。**每条消息携带 `sourceLine`（原始 JSONL 行号）**，这是分支定位的唯一依据。
- 缓存失效键：`mtimeMs + ctimeMs(changeMs) + size` 三者全等才复用（解决 Windows/WSL 复制保留 mtime/size 的场景）。
- 读取上限：本机整文件 32 MB；WSL 详情读超时按体积扩展（≥60 s，2 s/MiB，≤5 min）。

### 4.5 变更语义

- **Codex 分支（当前有效实现，风险最高）**：`findBranchTurnId` 逐行扫描至 `messageLine`，取其**前最近一条** `event_msg.task_started.payload.turn_id` 作为外层轮次 ID（消息内 `internal_chat_message_metadata_passthrough.turn_id` 是嵌套事件 ID，**不可**用于回退）→ 启动 `codex app-server` 子进程走 JSON-RPC（`initialize` → `initialized` → `thread/fork { threadId, lastTurnId }`）→ 校验返回 `thread.id` / `thread.path` 非空、非父线程、在 `sessions` 目录内 → 归一化内存对象 → 写分支元数据 → 追加 `history.jsonl`。失败时 `cleanupFailedFork`（最多重试 6 次 × 100 ms 删除残留文件）。
- **绝对禁止**：读写 `thread_history_*.sqlite`、调用 `migrate-rollouts`、创建受管 `CODEX_HOME`、手工截断 JSONL 伪造线程。
- 恢复命令统一 `codex resume --all <uuid>`（`--all` 关闭工作目录过滤，必须保留）。
- **复制**：重写 `session_meta` 的 `id` / `session_id` / `timestamp`，生成新 `rollout-<stamp>-<uuid>.jsonl`。
- **删除/恢复/清除**：一律先做**目录边界断言**（拒绝 `..` 逃逸、目录前缀混淆、根目录本身）+ **会话 ID 校验**（文件头部必须包含 `sessionId`）。恢复时若原位置已存在同名文件 → 拒绝。
- Qoder 分支/复制入口显式抛错。

### 4.6 存储抽象

`createSessionStorage({kind, distro})` 统一 `readText / writeText / readLines / exists / move / remove`；本机走 `fs/promises`，WSL 走 `wsl.exe`。`readLines` 支持 `startLine` 与提前停止（返回 `false`），是流式分页与分支扫描的公共底座。

---

## 5. 本地 Gateway（`src/electron/gateway`）

### 5.1 HTTP 表面

- 路由前缀 `/gateway/<providerId>/<routeId>`（`routeId` 为第三段，O(1) 命中）；额外 `/external/<providerId>` 对外模式。
- 监听固定 `127.0.0.1:<configuredPort>`；端口被占用时回退随机端口并记录 `fallbackFromPort`（配置为 `0` 时冲突属真异常）。
- 客户端（CLI）经环境变量指向网关：
  - Codex：`-c model_provider=akim_gateway`、`wire_api="responses"`、`base_url=<route>`，`OPENAI_API_KEY=<localToken>`。
  - Claude：`ANTHROPIC_BASE_URL` + `ANTHROPIC_AUTH_TOKEN`。
  - Gemini：`GOOGLE_GEMINI_BASE_URL` + `GEMINI_API_KEY`。
  - Qoder：不接网关（`supportsVendorGateway: false`）。
- WSL 目标需先探测宿主可达地址（`/dev/tcp` 探测 127.0.0.1 → `ip route` 默认网关 → `resolv.conf` nameserver → 回退 127.0.0.1），结果按 `(distro, port)` 缓存。

### 5.2 认证

- **本地路由令牌**：每 route `crypto.randomBytes(32).toString("hex")`；接受 `Authorization: Bearer` / `x-api-key` / `x-goog-api-key`，用 **`timingSafeEqual`** 比较。
- **对外 API 令牌**：`base64url(32B)` 只展示一次，库内存 **sha256 hex**；额外接受 `?key=`，比较前对候选值做 sha256 再 `timingSafeEqual`；`key` 参数从日志与上游请求中剔除。
- **上游认证**：`claude → x-api-key`；`gemini → x-goog-api-key`；其余 → `Authorization: Bearer`。转发时剔除 `host / authorization / x-api-key / x-goog-api-key / content-length / transfer-encoding / connection`。
- 响应头回传时剔除 `content-length / transfer-encoding / connection`。

### 5.3 路由与候选池

- Route 绑定 `{routeId, providerId, vendorId, mode, localToken, baseUrl, window, terminalId}`；`bindVendorRouteTerminal` 绑定终端；未绑定期间的切换原因暂存 `pendingSwitchReason`，绑定后补发事件。
- `mode = "dynamic"` 允许故障转移；`"locked"` 只重试指定供应商且**绕过熔断**。
- 候选池按 `providerId` 隔离；资格 = `providerId` 匹配 ∧ `enabled` ∧ `apiKey` 非空 ∧ `apiBaseUrl` 非空 ∧ 未熔断。
- 选择：`chooseVendor` 首选当前路由供应商，否则按 `sort` 升序（`createdAt` 次键）。
- 故障转移：`chooseNextVendor` 在**完整**（含已熔断）排序中从当前位置环形前进，跳过 `attemptedVendorIds` 与熔断项。**每次重新取快照**，确保已关闭供应商不会进入本轮。
- 最大尝试次数 = `bufferedBody && isRetryableMethod ? failureThreshold × candidateCount : 1`。**只有请求体已缓冲（≤2 MB 且声明 content-length）才可安全重试**，否则零缓冲透传、不重试。

### 5.4 故障识别与熔断

- 内置默认：7 条文本模式（`selected model is at capacity`、`exceeded retry limit`、`429 too many requests`、`too many requests`、`currently overloaded`、`rate limit`、`temporarily unavailable`）+ 10 个状态码（401/403/404/408/425/429/500/502/503/504）。
- 自定义规则：作用域 `global | provider | vendor`，**不区分大小写的安全包含匹配**（不开放正则），模式 ≤200 字符，≤100 条，自定义响应体 ≤32 KB 且必须合法 JSON，状态码 400–599。
- 规则走**内存快照 + 版本号**，CRUD 后失效，热路径不读 SQLite。
- 熔断状态机：内存 `Map` + SQLite 持久化。失败窗口 5 min；`failures.length >= circuitFailureThreshold` → `open` + `circuitUntil = now + duration`；读时若 `circuitUntil <= now` → 转 `half-open`；成功 → `healthy` 并清空。成功写入节流 5 s（状态变化立即写）。
- DB 不可用时降级为纯内存，且**把失败视为初始化完成**（避免每请求重试建表）。

### 5.5 HTTP 200 携带错误体的识别（关键）

`probeUpstreamBody` 读取上游响应**首个完整事件或最多 64 KB**：
1. 逐块累积解码 → `extractGatewayResponseError`：优先解析 JSON 的 `message` / `error` / `error.message` / `detail`；纯文本仅在命中内置模式时采信。
2. 若已解析出**结构化载荷**（任意候选可 JSON 解析），则**不**因正文偶然包含限流短语而触发切换。
3. 达到 `\n\n`（SSE 事件边界）或 64 KB 立即放行。
4. 已读字节通过 `replayBody` **原样接回**流，不改变正常 SSE。
5. 已向客户端写出内容后**绝不重放**，无其他候选时保留原始上游响应并记 `UPSTREAM_RESPONSE_ERROR`。
6. 命中带自定义响应的规则时，在未写出内容前返回配置的状态码与 JSON 体。

### 5.6 流式与用量

- 请求体：声明长度 ≤2 MB 才缓冲（同时提取 `model` 字段）；否则 `sizeGuardStream` 零缓冲透传并在超 32 MB 时 abort。
- 响应：`pipeline(upstream → byteCountingTransform → response)`；用量只捕获**前 512 KB**，按 `Buffer` 收集、结束一次性解码（避免跨 chunk 多字节截断与字符串反复扩容）。
- 用量解析支持 OpenAI / Anthropic / Gemini 字段族：`input_tokens|prompt_tokens|inputTokenCount`、`output_tokens|completion_tokens|outputTokenCount`、`cached_tokens` 族、`reasoning_tokens|thoughtsTokenCount`、`total_tokens|totalTokenCount`；SSE 逐 `data:` 行解析取最后一条。
- 合并语义：token 取**逐项 max**（同请求多次上报时防重复累加）。
- 费用：`tokens / 1e6 × 单价`（供应商 `pricing`），未配置单价不估算。
- 超时：上游 10 min；客户端断开（`response` 未 `writableEnded` 时 `close`）→ `client-aborted` 并 abort 上游；`TimeoutError` → `timeout` + 504。

### 5.7 日志与可观测性

- `logs/gateway.log` + 轮转 `gateway.log.1`，单文件 2 MB；内存环形缓冲（请求 200 条 / 事件 100 条），flush 间隔 200 ms。
- `gateway_request_logs` 记录 `request_id, route_id, provider_id, vendor_id, method, path(≤500), model, upstream_status, outcome, duration_ms, bytes_in/out, retry_count, switched, error_code, error_message(≤500), usage_json, created_at`。
- 事件推送：`gateway:request-recorded`（仅元数据，落库成功后发）、`gateway:vendor-switched`。
- **绝不记录**：API Key、请求/响应正文、终端输出。错误摘要统一 `replace(/[\r\n\t]+/g," ")` + `slice(0,500)`。

### 5.8 供应商管理

- CRUD + 唯一索引 `(provider_id, name_norm)`、`(provider_id, sort)`。
- **启用 = 把配置模板写入用户 CLI 配置文件**（`~/.codex/config.toml`、`~/.claude/settings.json`、`~/.gemini/…`），写前 `assertAllowedConfigPath` 白名单校验（只允许 `~/` 或 `$HOME/` 下 `.codex/`、`.gemini/`、`.claude/`，`path.posix.normalize` 防 `..` 穿越，反斜杠统一为正斜杠），并备份到 `data/vendor-backups/<vendor>/<ts>/`。
- 模型查询：15 s 超时 / 2 MB / 最多 500 条；余额查询支持 `auto | generic | new-api | custom` 模板 + 点号 JSON Path 提取。
- DeepSeek 预设走 Anthropic 兼容入口 `https://api.deepseek.com/anthropic` 并强制切到 `claude` 平台（协议透传）。

---

## 6. 终端运行时（`terminal-sessions.ts` + `terminal-status-detector.ts`）

### 6.1 PTY

- `node-pty` 动态加载（3 个候选路径），`name: "xterm-256color"`，默认 100×30，resize 下限 10×4。
- 输出按块累积 + **16 ms 定时合并**后发 `terminal:data`（避免大段历史恢复反复复制字符串）。
- 终端 ID = `crypto.randomUUID()`；`resumeKey = targetId \0 sessionId` 做**单写入者互斥**（`pendingResumeKeys` / `activeResumeKeys`），重复打开同一历史会话直接抛错。
- 退出：清理状态检测器 → resolve → flush → 删 session → 释放 resumeKey → 销毁 route → 发 `terminal:exit`。
- 应用退出：kill 全部 PTY，`Promise.race([all exitPromise, 3 s])`。
- 环境注入：覆盖 `TERM`；**注入 `OPENAI_API_KEY` 时必须删除继承的 `CODEX_API_KEY` / `CODEX_ACCESS_TOKEN`**（否则 Codex 会用错 bearer）。

### 6.2 命令拼装（各平台差异）

| Provider | 本机 Windows | 本机 POSIX | WSL |
| --- | --- | --- | --- |
| Codex | `codex.cmd`（`where.exe` → PATH → `%APPDATA%\npm` → `%LOCALAPPDATA%\npm` → `ProgramFiles\nodejs` → `NVM_HOME\*` 逐级解析）+ `-c` 覆盖 | `$SHELL -ic` + `exec codex` | `wsl.exe -d <d> -- bash -ic "unset CODEX_API_KEY CODEX_ACCESS_TOKEN; export …; cd …; exec codex"` |
| Claude | `cmd.exe /d /s /c claude …` + env | `$SHELL -ic` + `env ANTHROPIC_… claude` | 同上（`export ANTHROPIC_BASE_URL/AUTH_TOKEN`） |
| Gemini | `cmd.exe /d /s /c gemini …` + env | 同上 | 同上 |
| Qoder | `cmd.exe /d /s /c qodercn …` | `$SHELL -ic` | `wsl.exe -d <d> -- bash -ic` |

- cwd 缺省 `~/.akim` 并 `mkdir -p`；Codex 支持 `-C <cwd>`（`useCodexCwdFlag`）。
- 恢复参数：Codex `resume --all <id>`；Claude/Gemini `--resume <id>`；Qoder `--resume <id>` + 可选 `--cwd`。
- 系统终端：`auto` / `powershell` / `cmd` / `git-bash`（4 个固定路径探测）/ `wsl` / `wsl-default` / `bash` / `zsh` / `fish`。
- 引号规则：POSIX `'\''` 转义；cmd `^` 转义元字符。

### 6.3 终端状态检测器（逐字重实现）

- 仅保留最近 **32 KB** 去 ANSI 文本。
- `markInput`：只有含**可打印**字符（去 ANSI 与控制符后非空）才计为新轮次；同一轮内后续按键**不重置**缓冲（除非当前是 `awaiting-confirmation`）；递增 `turnId`。
- `inspectOutput`：提示符可见性状态机 —— 必须出现「非空闲提示输出」且提示符**消失后再次出现**才判 `completed`；启动界面静态提示不误报。
- 确认提示识别：`Press enter to confirm or esc to cancel`、Codex 选项式 `Yes, proceed` / `Allow … [..]`、`y/n`、`allow/approve/confirm` 结尾、中英文授权语句。
- 错误识别：`stream disconnected before completion`、`transport error`、`conversation interrupted`、`fatal error/unhandled exception`、`<cli> 已退出/退出`。
- `markExit(0)` **不产生**「本轮已完成」（仅表示进程结束）；非 0 且用户提交过输入 → `error`。
- 去重：同 `status + attention key` 不重复发；事件携带 `turnId`。

### 6.4 WSL 执行层（`core/wsl.ts`）

- `wsl.exe` 探测 5 个候选路径；统一 60 s 兜底超时（`spawn` 走 `attachSpawnTimeout` → SIGKILL）。
- 脚本经 **base64 透传**（`printf %s <b64> | base64 -d | bash`）绕开 `wsl.exe` 对引号/反斜杠的再解析 —— 含 `$(...)` 的脚本必须走这条路。
- 写文件走 `cat > file` + stdin（内容不经任何 shell）。
- 输出上限：shell 16 MB、读文件 32 MB、stderr 1 MB。
- 排除 Docker Desktop 内部发行版（`docker-desktop` / `docker-desktop-data`）。
- `-l -q` 输出按 UTF-16 解码 + 去 `\0` + 去 `*` 前缀。

---

## 7. Skill 与 CLI 环境

- Skill 根：`$CODEX_HOME/skills`；回收站 `<home>/.visual-console-trash/skills/<name>-<ts>`。
- 停用 = 目录重命名追加 `.disabled`；恢复时剥掉后缀。
- 导入：目录需含 `SKILL.md`（≤2 MB），必须 YAML frontmatter 且含 `name` / `description`；先写临时目录再原子换入。
- CLI 环境探测：node / npm / nvm / CLI 版本，推荐 Node 24；Gemini、Qoder 最低 Node 20；安装命令 `npm install -g …`，超时 15 min；WSL 侧用 `bash -lc` 探测（nvm/fnm 只在登录 shell 注入 PATH）。

---

## 8. 渲染层架构

### 8.1 状态所有权

`App` 是编排枢纽，自身只持有 UI 布尔态（浮层开关、右键菜单位置、侧栏折叠）与 `error`。业务状态全部下沉到 hooks：

| Hook | 负责 |
| --- | --- |
| `useProviderTargets` | 平台列表、目标列表、选中平台/目标、`syncTerminalTabTarget` |
| `useSessionWorkspaceState` | 视图（工作台/会话）、会话缓存、`loadedViews`、选中会话 |
| `useSessionLoader` / `useSessionListState` | 加载、本地搜索、批量选择 |
| `useTerminalTabs` | 标签、`terminalIdsByTabKey`、`terminalAttentionByTabKey`、`terminalInputStatesByTabKey` |
| `useSessionDetails` | 详情弹窗、分页、分支面板 |
| `useVendors` / `useTabVendors` | 供应商 CRUD、**按标签的路由映射** |
| `useSessionQuickAccess` | 收藏 / 最近打开 |
| `useGlobalSessionSearch` | 全局检索（250 ms 防抖、受限并发、过期失效） |

- **只读供应商快照**走 `VendorDataProvider` 窄 Context；可变编辑态仍用显式 props（避免放大终端重构风险）。
- 每标签状态：终端 ID、输入模式、路由供应商与模式、提醒状态 —— 全部**仅内存**，关标签即释放。
- 渲染层不做轮询：工作台靠 `gateway:request-recorded` 事件（250 ms 窗口合并）刷新。

### 8.2 终端 UI

- `useXtermHost` 为 Composer / 嵌入式 AI 终端 / 系统终端共用挂载逻辑。
- 两种输入模式：**终端原生输入** vs **自管输入区（Composer）**。自管输入支持模型选择、文件/目录附件、`Alt+Enter` 换行。
- Windows IME：兼容组合态 Enter（含 Chromium `keyCode=229`），等 `compositionend` 后再发送。
- 括号粘贴结束标记与回车**合并为一次 PTY 写入**（Qoder 除外，保留协议所需间隔）以规避 Windows 分次写入的回显/丢提交。
- 焦点只由活动标签切换、显式请求、用户切换输入模式触发。
- **活动标签门禁**：非活动标签不向自己的 PTY 转发字符、粘贴、Ctrl+C。

### 8.3 设计系统

- 单一青绿色强调色 `--ui-accent: #168c78`；中性浅色工作区 + 深色高对比终端表面。
- Token：`--ui-bg/surface/surface-subtle/border/border-strong/text/muted/accent/accent-strong/accent-soft/focus/shadow`。
- 组件契约：`ui-button` + `ui-button-primary|secondary|danger`；`IconButton` 为唯一纯图标操作（32px 安静型）；统一 `Dialog` 骨架（关闭按钮固定在标题栏右上）；共用 `Pagination`（10/20/50，默认 10，上限 50）。
- 图标统一 `lucide-react`；图标按钮必须带 `title` + `aria-label`。

### 8.4 交互契约

- `Ctrl/Cmd + K` 命令面板（递归复用菜单 action；原生终端输入模式下不拦截）。
- `Alt + Enter` 自管输入换行；`Escape` 关闭浮层 / 菜单。
- `TerminalTextContextMenu` 是全部终端类文本区域的唯一右键菜单（仅复制 / 粘贴；捕获阶段滚动、点击外部、Escape 关闭）。
- 提醒中心：铃铛按钮展开；点击条目切换标签并标已读；`X` 真正移除；全部移除后入口整体卸载。
- 窗口最小化/失焦时通过系统通知提示；按事件签名去重，聚焦时不打扰。
- 关闭确认三选一；托盘单击恢复，右键「显示窗口 / 退出」。

---

## 9. 现状中的结构性痛点（Rust 重构的动因）

1. **IPC 契约手工维护**：`shared/codex-console-api.ts` 是 168 行手写类型，与 81 个 `ipcMain.handle` channel 靠测试约束，漂移风险长期存在。
2. **WSL 每次文件操作都 spawn `wsl.exe`**（60 s 超时 + base64 包装），单次 50–150 ms 固定开销；会话列表要遍历大量文件时是主要瓶颈。
3. **分页索引以「行号」为锚**：`readLines(startLine)` 仍要从文件头逐行读到起始行，大 JSONL 的随机定位不是真正的 O(1)。
4. **主进程单线程**：JSONL 解析、SQLite 写入、HTTP 代理共用事件循环；`node:sqlite` 为同步 API，长事务会阻塞网关热路径。
5. **`vendor-gateway.ts` 885 行单文件**混合了路由、认证、探测、故障转移、用量、日志六种关注点。
6. **Provider 抽象泄漏**：Codex 因 `app-server` 分支、32 MB 读取限制、独立缓存体系而无法与其余三家共享基类（DEV-GUIDE 已明确记录该结论）。
7. **渲染层 `app.tsx` 1,193 行 + `app.css` 8,214 行**，无组件级样式隔离。
8. **无结构化日志/追踪**：仅文本 `gateway.log` 与 `performance.log`，跨模块排障依赖人工拼接。
9. **测试与实现同语言同进程**：无法用差分测试直接对拍两版实现。

---

## 10. 必须在 Rust 版中逐字保留的高风险行为（按风险降序）

| # | 行为 | 依据 |
| --- | --- | --- |
| 1 | Codex 分支走官方 `app-server` JSON-RPC + `lastTurnId`；禁止 `migrate-rollouts` / 私有 SQLite | `codex-targets.ts:673-895` |
| 2 | `findBranchTurnId` 用**前最近** `task_started.turn_id`，不用嵌套消息 turn_id | `codex-targets.ts:679-696` |
| 3 | 恢复命令恒为 `codex resume --all <id>` | `terminal-sessions.ts:451,469` |
| 4 | 分页 `offset === -1` 语义 + `sourceLine` 原始行号 | `codex-targets.ts:221-275` |
| 5 | HTTP 200 错误体探测 ≤64 KB / 首个 SSE 事件，且**结构化载荷不误判** | `vendor-gateway.ts:629-678` |
| 6 | 已写出内容后绝不重放；无候选时回放真实上游响应 | `vendor-gateway.ts:452-474` |
| 7 | 仅请求体已缓冲才允许重试；`maxAttempts = threshold × candidateCount` | `vendor-gateway.ts:362-379` |
| 8 | 锁定模式绕过熔断；动态模式才环形故障转移 | `vendor-gateway.ts:337-339,389-405` |
| 9 | 熔断窗口 5 min / 阈值 3(1-20) / 时长 60 s(10-86400) / 读时转 half-open | `gateway-resilience.ts:5,185-196,266-268` |
| 10 | 转发时剔除的 7 个请求头 / 3 个响应头；`joinUpstreamUrl` 的 `/v1` 去重 | `vendor-gateway.ts:487-490,855-879` |
| 11 | 令牌 `timingSafeEqual`；对外令牌只存 sha256；`?key=` 从日志与上游剔除 | `vendor-gateway.ts:811-849` |
| 12 | 客户端断开 vs 超时的区分（`response.close` 且未 `writableEnded`） | `vendor-gateway.ts:287-291,697-701` |
| 13 | 终端状态检测的提示符状态转移 + 32 KB 缓冲 + 确认后抑制 | `terminal-status-detector.ts:44-64` |
| 14 | `markExit(0)` 不产生「本轮已完成」 | `terminal-status-detector.ts:66-83` |
| 15 | 注入 `OPENAI_API_KEY` 时删除 `CODEX_API_KEY` / `CODEX_ACCESS_TOKEN` | `terminal-sessions.ts:207-216` |
| 16 | WSL 脚本 base64 透传；写文件走 stdin | `core/wsl.ts:102-152,155-191` |
| 17 | 会话文件目录边界断言 + 会话 ID 头部校验 + 恢复冲突拒绝 | `codex-store.ts:243-258,206-221` |
| 18 | 缓存失效键 `mtime + ctime + size` 三者全等 | `codex-store.ts:107` |
| 19 | 配置路径白名单 + `..` 穿越防御 + 反斜杠归一 | `shell-args.ts:69-87` |
| 20 | 合成消息块过滤 + 连续同 role/text 去重 | `session-parser.ts:305-321` |
| 21 | `wslCodexHomes` 覆盖优先于探测；Codex 裸 `wsl:` 前缀兼容 | `codex-targets.ts:143-155`、`target-ids.ts` |
| 22 | 唯一索引 `(provider_id, name_norm)` / `(provider_id, sort)` | `vendor-manager.ts:575-582` |
| 23 | 固定端口被占用回退随机 + `fallbackFromPort`；有活动路由时拒绝停常驻网关 | `vendor-gateway.ts:70-80`、DEV-GUIDE |
| 24 | Skill `.disabled` 后缀方案 + 回收站 `<name>-<ts>` | `skills.ts:119-168` |
| 25 | 运行根三形态派生 + userData 落到 `data/user-data` | `application-paths.ts`、`main.ts:37-47` |
| 26 | Qoder 无分支/复制/网关；多目录整体迁移 | `ai-providers.ts:121-136`、DEV-GUIDE |
| 27 | 15 s/2 MB/500 条模型查询上限；配置写入前备份 | `vendor-manager.ts:680-683,235` |
| 28 | `session_cache` 4,000 条 / 64 MB / 单条 1 MB LRU | `app-database.ts:59-61,454-475` |
