# AI Visual Console — Rust 重构架构方案

> 目标：**功能一致**（行为契约级 1:1）、**高性能**、**可持续扩展**、**企业级工程标准**。
> 本文是待确认的方案主体。确认后以全新工程落地。

---

## 1. 核心架构决策

### 决策 1：外壳选型 —— Tauri 2 + Rust 核心 + 保留 Web 前端（**推荐**）

| 方案 | Rust 占比 | 功能一致成本 | 性能 | 风险 |
| --- | --- | --- | --- | --- |
| **A. Tauri 2 + Rust 核心 + React/xterm.js 前端**（推荐） | 全部业务逻辑 100% Rust | **低**：渲染层 19k 行可增量迁移 | 高：主进程瓶颈全部消除；终端渲染仍由 xterm.js（WASM/JS 优化成熟）承担 | 低 |
| B. 纯 Rust 原生 GUI（egui / gpui / iced + `alacritty_terminal`） | 100% | **极高**：8,214 行 CSS 设计系统 + 40+ 组件 + 终端仿真 + IME/选区/搜索全部重写 | 终端渲染可能更高，但 UI 层需自建 | **极高**：功能一致几乎不可达 |
| C. Rust 后端 + 本地 HTTP/WS + 浏览器 | 100% 后端 | 中 | 中（多一跳 IPC） | 中：安全模型与窗口/托盘集成复杂 |

**推荐 A 的理由**：

1. 产品价值集中在**主进程**（会话解析、Gateway、PTY、WSL、供应商），这些 100% 用 Rust 重写即可拿到全部性能收益。
2. 终端仿真器是**最难做对**的部分（IME 组合态、括号粘贴、选区、搜索、Unicode 宽度、重绘性能）。xterm.js 是业界最成熟的实现，重写它是纯粹的负收益。
3. 8,214 行 CSS 设计系统是真实资产，且已通过视觉基线验收。
4. Tauri 2 提供托盘、单实例、通知、目录选择、外链打开等全部所需能力，二进制体积比 Electron 小一个数量级。

**关键降风险手段**：在 `ui/src/bridge/` 写一层**极薄适配器**，把现有 `window.codexConsole.*`（168 行契约）映射到 Tauri `invoke`/`listen`。这样渲染层 19,319 行**几乎零改动**即可先跑通，再按域逐步 Rust 化重构。这是整个迁移能否低风险完成的关键。

> 若最终必须 100% Rust UI，建议作为**独立后续阶段**（Phase 9），在功能对拍完成、契约冻结之后再做，届时 `aivc-ipc` 已与 UI 解耦，换壳不影响核心。

### 决策 2：其余技术选型

| 关注点 | 选型 | 理由 |
| --- | --- | --- |
| 异步运行时 | `tokio`（multi-thread） | 生态标准；`spawn_blocking` 承接阻塞 fs/PTY |
| HTTP 服务 | `axum` 0.8（hyper 1.x） | tower 中间件、`Body::from_stream` 原生流式 |
| HTTP 客户端 | `reqwest`（`stream` + `rustls`） | 流式上游代理；`rustls` 免 OpenSSL 分发问题 |
| 数据库 | `rusqlite`（`bundled`）+ `deadpool-sqlite`（读池）+ **单写者 actor** | 与现有「全局串行写队列」语义天然同构；无编译期 DB 依赖；`bundled` 免系统 SQLite |
| 迁移 | `refinery`（版本化 SQL 迁移） | 取代现有 `ALTER TABLE` + `try/catch` 的隐式升级 |
| PTY | `portable-pty`（wezterm） | Windows 走 ConPTY（与 node-pty 一致）、Unix 走 forkpty |
| 终端仿真 | **保留 xterm.js**（WebView 内） | 见决策 1 |
| IPC 类型 | `tauri-specta`（specta v2）生成 TS | **直接消除** `codex-console-api.ts` 手工漂移痛点 |
| 序列化 | `serde` + `serde_json`（typed struct，非 `Value`） | 热路径跳过 `Value` 中间层 |
| 错误 | 库 crate `thiserror`；二进制 `anyhow`；`secrecy` 包裹密钥 | 错误分类可测；`Debug` 自动脱敏 API Key |
| 日志 | `tracing` + `tracing-subscriber` + `tracing-appender`（轮转） | 结构化 span：`request_id` / `terminal_id` / `session_id` / `distro` |
| 并发原语 | `tokio-util::CancellationToken` + `Semaphore` + `futures::stream::buffered` | 取代手写 `mapLimit`；可取消 |
| 热路径快照 | `arc-swap` + `dashmap` | 供应商快照 / 规则快照 / 健康状态无锁读 |
| 正则 | `regex` + `OnceLock` 静态编译 | 终端状态检测与 ANSI 剥离 |

**不引入**：`simd-json`、`sonic-rs`（先以 typed `serde` 达成目标，仅在 profiling 证明必要时作为 feature flag 引入，避免无谓的 unsafe 与维护成本）。

---

## 2. 目标工程结构

> **落地修订（P1）**：crate 数量从 14 收敛到 **8**。原则是「crate 边界只在两种情况
> 值得存在：(a) 它强制了一条重要的依赖方向；(b) 它隔离了重量级或独特的外部依赖」。
> 按此原则，`observability`、`infra-{db,fs,wsl,http}`、`config`、`skills`+`cli-env`
> 共 6 个 crate 不成立，已合并为 `aivc-infra` 与 `aivc-tooling` 的内部模块。
>
> 澄清一个常见误解：Rust 里 **crate ≈ Maven module**，每个 crate 的源码根**必须**是
> `<package>/src/`（`lib.rs` / `main.rs`）——这是 Cargo 的硬约定，`src/foo/` 相当于
> Java 的 package。可以用 `[lib] path` 改写，但会失去 rust-analyzer、`cargo fmt`
> 与 IDE 跳转等全部工具约定。

```
aivc-rs/
├── Cargo.toml                    # workspace（resolver = "3"）
├── rust-toolchain.toml           # 工具链与组件
├── clippy.toml                   # lint 细粒度配置（测试豁免等）
├── deny.toml                     # cargo-deny：许可证白名单 + 禁用 openssl
├── crates/
│   ├── aivc-core/                # ★ 纯领域层：零 I/O、零平台依赖
│   │   └── src/
│   │       ├── error.rs          #   错误分类 + 用户可见消息（与现网中文文案逐字一致）
│   │       ├── id.rs             #   强类型标识（SessionId / TargetId / VendorId …）
│   │       ├── session.rs        #   会话、消息、用量、元数据、分页游标
│   │       ├── metadata.rs       #   元数据归一化与合并（parity 关键纯函数）
│   │       ├── target.rs         #   平台标识、目标、targetId 语法、能力矩阵
│   │       ├── text.rs           #   合成消息过滤、标题裁剪、搜索词归一
│   │       ├── time.rs           #   时间戳（对齐 JS toISOString）
│   │       └── ports.rs          #   端口 trait：SessionStore / Clock / ReadStart
│   ├── aivc-infra/               # 基础设施适配器（模块级依赖规则见该 crate 文档）
│   │   └── src/
│   │       ├── config/           #   运行根派生 + settings.json（逐字段容错）
│   │       ├── db/               #   SQLite：迁移 / 单写者 actor / 读池 / 仓储
│   │       ├── fs/               #   本地文件系统 SessionStore
│   │       ├── wsl/              #   WSL：持久 worker + 一次性回退 + 宿主地址探测
│   │       ├── http/             #   reqwest 客户端（模型/余额查询）
│   │       └── logging/          #   tracing 初始化 + 日志轮转
│   ├── aivc-provider/            # Provider trait + 注册表 + 能力子 trait
│   │   └── src/{codex,claude,gemini,qoder}/   # codex 含 app-server JSON-RPC 客户端
│   ├── aivc-gateway/             # axum 服务 + 路由 + 故障转移 + 熔断 + 用量 + 日志
│   ├── aivc-terminal/            # portable-pty + 命令构造 + 状态检测器
│   ├── aivc-tooling/             # Skill 管理 + CLI 环境探测/安装
│   ├── aivc-ipc/                 # Tauri command 层（DTO + 校验 + 错误映射 + specta 导出）
│   └── aivc-app/                 # ★ 唯一二进制：装配、窗口、托盘、生命周期
├── ui/                           # React 19 + TS + Vite + xterm.js（迁移自 src/renderer）
├── fixtures/                     # 对拍语料：JSONL 样本、上游 mock 响应、settings 样本
├── scripts/                      # 本地工具链启动器与质量门禁
├── xtask/                        # 构建 / 打包 / 对拍 / 分层校验自动化
└── docs/
```

**依赖方向（由 `cargo xtask check-layers` 强制）**：

```
aivc-app ──► aivc-ipc ──► { aivc-provider, aivc-gateway, aivc-terminal, aivc-tooling }
                              │
                              ▼
                        aivc-infra ──► aivc-core
```

`aivc-infra` **内部**的模块级规则（`fs` / `wsl` / `db` / `http` 之间不得互相依赖）
由代码评审把关，因为 `cargo metadata` 看不到模块级依赖。这是减少 crate 数量的代价，
已在该 crate 的模块文档中显式记录。

`aivc-core` **只依赖** `serde` / `thiserror` / `time` / `uuid`。任何反向依赖在 CI 中直接失败。

---

## 3. 领域层与端口设计（`aivc-core`）

### 3.1 端口 trait（替代现有隐式耦合）

```rust
/// 会话文件 I/O —— 对应现有 createSessionStorage({kind, distro})
pub trait SessionStore: Send + Sync {
    fn read_text<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<String>>;
    fn write_text<'a>(&'a self, path: &'a str, content: &'a str) -> BoxFuture<'a, Result<()>>;
    /// start_line + 提前停止（返回 false）—— 分页与分支扫描的公共底座
    fn read_lines<'a>(&'a self, path: &'a str, start: LineCursor,
                      opts: ReadOpts, sink: &'a mut dyn LineSink) -> BoxFuture<'a, Result<()>>;
    fn exists<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<bool>>;
    fn move_file<'a>(&'a self, from: &'a str, to: &'a str, on_conflict: ConflictPolicy) -> BoxFuture<'a, Result<()>>;
    fn remove<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<()>>;
}

/// 仓储端口（每个聚合一族，禁止跨聚合事务）
pub trait SessionCacheRepo: Send + Sync { /* read / replace / remove_entries / prune */ }
pub trait SessionMetadataRepo: Send + Sync { /* get / get_map / list / save / delete / by_parent */ }
pub trait MessageIndexRepo: Send + Sync { /* save_anchors / find_anchor_at_or_before */ }
pub trait VendorRepo: Send + Sync { /* list / get / upsert / delete / set_enabled / configs */ }
pub trait HealthRepo: Send + Sync { /* load_all / upsert / delete */ }
pub trait RequestLogRepo: Send + Sync { /* insert / query_page / aggregate_usage / diagnostics */ }
pub trait FailoverRuleRepo: Send + Sync { /* list / upsert / delete / set_enabled */ }
pub trait PresetRepo: Send + Sync { /* list / upsert / delete */ }

pub trait EventBus: Send + Sync { fn emit(&self, event: DomainEvent); }
pub trait Clock: Send + Sync { fn now(&self) -> OffsetDateTime; }
pub trait ProcessRunner: Send + Sync { /* 探测性命令执行（where/which/node -v） */ }
```

**收益**：Provider / Gateway / Terminal 全部可在**内存实现**上做单元测试，无需真实文件系统、WSL 或 SQLite。

### 3.2 Provider 能力用**子 trait** 表达（而非布尔矩阵）

现有实现用 `capabilities: { branch: bool, duplicate: bool, … }` + 运行时判断，DEV-GUIDE 明确要求「能力标记与运行时行为必须一致」，靠测试约束。Rust 版改为类型系统保证：

```rust
pub trait SessionProvider: Send + Sync {
    fn id(&self) -> ProviderId;
    fn label(&self) -> &'static str;
    async fn list_targets(&self, ctx: &Ctx) -> Result<Vec<Target>>;
    async fn list_sessions(&self, t: &Target, view: View) -> Result<Vec<SessionSummary>>;
    async fn read_session(&self, t: &Target, id: &SessionId, hint: Option<&Path>) -> Result<Session>;
    async fn read_page(&self, t: &Target, id: &SessionId, cur: PageCursor) -> Result<MessagePage>;
    async fn search(&self, t: &Target, view: View, q: &str) -> Result<Vec<SessionSummary>>;
    async fn trash(&self, t: &Target, ids: &[SessionId]) -> Result<MutationOutcome>;
    async fn restore(&self, t: &Target, id: &SessionId) -> Result<RestoreOutcome>;
    async fn purge(&self, t: &Target, ids: &[SessionId]) -> Result<MutationOutcome>;
}

/// 能力子 trait —— 注册表用 downcast 暴露，能力矩阵由实现自动导出
pub trait Branchable: SessionProvider { async fn branch(&self, t: &Target, id: &SessionId, line: u64) -> Result<Session>; }
pub trait Duplicable: SessionProvider { async fn duplicate(&self, t: &Target, id: &SessionId) -> Result<Session>; }
pub trait Exportable: SessionProvider { async fn export(&self, t: &Target, id: &SessionId, fmt: ExportFormat) -> Result<ExportOutcome>; }
pub trait SkillAware: SessionProvider { /* Skill 管理 */ }
pub trait SessionSettingsAware: SessionProvider { /* cwd / codexHome 设置 */ }
```

- `ProviderRegistry` 在装配期计算能力矩阵（`fn capabilities(&self) -> ProviderCapabilities`），供 IPC 返回给 UI。
- Qoder 未实现 `Branchable` / `Duplicable`，**编译期**就不存在该路径；同时保留 `Err(Error::Unsupported { provider: Qoder, capability: Branch })` 的显式拒绝以维持对外契约与文案。

### 3.3 错误分类

```rust
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("未找到会话：{id}")]            SessionNotFound { id: String },
    #[error("拒绝将 sessions 目录之外的文件移动到回收站")] PathOutsideRoot,
    #[error("会话文件与会话编号不匹配：{id}")] SessionIdMismatch { id: String },
    #[error("原位置已存在同名会话文件，无法恢复。")] RestoreConflict,
    #[error("该历史会话正在打开，请勿重复操作。")] ResumeInFlight,
    #[error("Codex 未返回可恢复的分支线程。")] ForkInvalid,
    #[error("分支位置已失效，请重新打开会话详情后再试。")] BranchAnchorStale,
    #[error("配置文件路径不在允许范围：{path}")] ConfigPathNotAllowed { path: String },
    #[error("本地 Gateway 已关闭，请先在网关设置中启用。")] GatewayDisabled,
    #[error("{provider:?} 不支持{capability}。")] Unsupported { provider: ProviderId, capability: &'static str },
    #[error(transparent)] Io(#[from] std::io::Error),
    #[error(transparent)] Db(#[from] DbError),
    #[error(transparent)] Wsl(#[from] WslError),
    #[error(transparent)] Upstream(#[from] UpstreamError),
}
```

**文案必须与现网逐字一致**——UI 直接展示 `error.message`，这是功能一致的一部分。建议用 `insta` 快照固化全部错误文案。

---

## 4. 数据层（`aivc-infra-db`）

### 4.1 表结构与迁移

**已确认决策：全新库，不做旧库迁移。** 因此无需保留任何历史兼容列，schema 从 v1 起即为干净设计。新库文件名 `data/aivc.db`（与旧工程 `app.db` 并存互不干扰，旧工程可继续独立运行）。

语义与现有 10 张表保持一致（列名、主键、唯一索引），但结构按干净基线重写：

| 设计 | 理由 |
| --- | --- |
| `schema_version` 表 + `refinery` 版本化迁移脚本，从 `V1__initial.sql` 起 | 取代现有 `ALTER TABLE` + `try/catch` 的隐式升级；新库无历史包袱 |
| `session_message_index` **直接使用 `byte_offset INTEGER`** 作为锚点（不再有 `line_number`） | 现有实现以「行号」为锚仍需从文件头逐行读到起始行；字节偏移可 `File::seek` 真 O(1) 随机定位 |
| `session_cache` 直接含 `change_ms`、去掉冗余的 `cached_at` 默认值 hack | 无需 `ALTER TABLE` 兼容路径 |
| `gateway_request_logs` 新增 `(provider_id, created_at DESC)` 索引 | 诊断弹窗按 provider 筛选（现有仅 `created_at` / `(vendor_id, created_at)`） |
| `api_vendors.api_key` 经 `secrecy::SecretString` 读写 | 内存与日志层面默认脱敏；落库仍为明文（与现网一致，避免破坏 CLI 配置写入） |
| `PRAGMA` 保持 `journal_mode=WAL` / `busy_timeout=5000` / `foreign_keys=ON` | 行为一致 |

**接受的数据后果**（用户已确认）：首次运行会重建会话摘要缓存（可重建，无损失）；但 `session_metadata` 中的**自定义标题、收藏、最近打开、分支关系**不会从旧 `app.db` 带入。如需保留，可选提供 `xtask import-metadata`（一次性只读旧库并写入新库，约 50 行，非必需项）。

### 4.2 并发模型（替代全局写队列）

```
        ┌──────────── 读池：deadpool-sqlite（N 个只读连接，WAL 并发读）────────────┐
IPC ────┤                                                                        │
Gateway ┘                                                                        │
        └──────────── 写：单一 writer task（mpsc<WriteCmd> + oneshot ack）─────────┘
```

- 所有写操作经 `mpsc` 投递到**唯一** writer task，串行执行事务 —— 与现有 `updateAppDatabase` 的 `BEGIN IMMEDIATE` 语义完全同构，但**不阻塞 tokio 工作线程**（现有 `node:sqlite` 是同步 API，会阻塞事件循环）。
- 读操作走连接池，与写不互斥（WAL）。
- 批量写（如 `replaceSessionCache` 的整表替换 + 淘汰）在单个事务内完成。

### 4.3 缓存淘汰

保持 4,000 条 / 64 MB / 单条 1 MB 上限，按 `cached_at ASC, rowid ASC` 淘汰。淘汰逻辑在 writer task 内与插入同事务执行。

---

## 5. Provider 层（`aivc-provider`）

### 5.1 分层

```
aivc-provider
├── registry.rs        # 定义表 + 能力矩阵导出 + 目标分发
├── common/            # 目标探测、排序、搜索匹配、缓存增量加载编排
├── jsonl/             # 流式行读取 + typed 反序列化 + 锚点索引写入
├── codex/  { targets.rs, store.rs, appserver.rs, branch.rs }
├── claude/ { provider.rs, parser.rs, telemetry.rs }
├── gemini/ { provider.rs, parser.rs, projects.rs }
└── qoder/  { provider.rs, parser.rs, migrate.rs }
```

### 5.2 Codex（最高风险模块）

- **`appserver.rs`**：`codex app-server` 子进程 + JSON-RPC 客户端。`initialize` → `initialized` → `thread/fork { threadId, lastTurnId }`。用 `tokio::process::Command` + `BufReader::lines()` + `oneshot` 请求表；`CancellationToken` 承载 60 s 超时；`Drop` 保证 SIGTERM 与子进程回收。
- **`branch.rs`**：
  - `find_branch_turn_id(path, message_line)`：流式扫描至目标行，维护 `active_turn_id`（仅在 `event_msg.task_started.payload.turn_id` 更新），命中行时快照。**不用**消息内嵌 turn_id。
  - `canonicalize_fork(...)`：校验 `thread_id` 非空、≠ 父、`path` 在 `sessions` 根内、≠ 父文件。
  - `cleanup_failed_fork(...)`：6 次 × 100 ms 重试删除。
  - 硬约束（CI 静态检查 + 代码评审清单）：**不触碰** `thread_history_*.sqlite`、**不调用** `migrate-rollouts`、**不创建**受管 `CODEX_HOME`、**不手工截断 JSONL**。
- **`targets.rs`**：目标探测 30 s TTL + 进行中 Promise 合流 → Rust 用 `tokio::sync::OnceCell` + `arc-swap` 快照 + `Mutex<Option<JoinHandle>>` 合流。
- **分页**：`PageCursor::Latest | Offset(n)`。首屏 `Latest` 取末尾 `limit` 条（环形缓冲 `VecDeque`）并回填真实偏移；`Offset(n)` 先查锚点（**优先 `byte_offset` seek**，回退行号线性读）。每 100 条可见消息落锚点。

### 5.3 Claude / Gemini / Qoder

- **Claude**：`~/.claude/projects/<dir>/*.jsonl` + telemetry 目录推上下文窗口（15 s TTL 缓存）；`1M` beta 模型识别；usage 累加器（input / output / cache_creation / cache_read）。
- **Gemini**：`~/.gemini/tmp/<hash>/chats/*.json` + 项目映射校验；message tokens 用量。
- **Qoder**：`~/.qoder-cn/{projects,tasks,file-history,logs}` 多目录整体迁移 + 回滚；`projects`/`tasks` 关联校验；不实现 `Branchable` / `Duplicable`。

### 5.4 性能改进点

| 现状 | Rust 版 |
| --- | --- |
| 列表解析并发 16（手写 `mapLimit`） | `futures::stream::iter(files).map(parse).buffered(16)` |
| `serde_json::Value` 全量解析每行 | 直接反序列化到 `#[derive(Deserialize)]` 结构体（只取需要的字段） |
| 详情分页从行号线性读 | 新增字节偏移锚点，`File::seek` 真 O(1) |
| 每次文件操作同步阻塞事件循环 | `tokio::fs` + `spawn_blocking`（本地）/ 持久 worker（WSL） |
| 缓存写入整表 `DELETE` + 逐条 `INSERT` | 单事务 + `INSERT ... ON CONFLICT` 批量；仅写入变更项 |

---

## 6. Gateway 层（`aivc-gateway`）

### 6.1 模块拆分（替代 885 行单文件）

```
aivc-gateway
├── server.rs        # axum Router + 监听 + 固定端口回退 + 常驻启停 + 状态快照
├── route.rs         # RouteTable（arc-swap）+ 令牌校验 + 路由解析
├── upstream.rs      # reqwest 转发 + URL 拼接 + 头部改写
├── probe.rs         # HTTP 200 错误体探测（≤64 KB / 首个 SSE 事件）+ 字节回放
├── failover.rs      # 候选池选择 + 环形故障转移 + attempted 集合
├── health.rs        # 熔断状态机 + 内存 Map + 节流持久化
├── rules.rs         # 故障规则快照 + 作用域匹配 + 自定义响应
├── usage.rs         # SSE/JSON 用量解析 + 合并 + 费用计算 + 聚合报表
├── log.rs           # 文件日志 + 轮转 + 环形缓冲 + 请求落库
└── external.rs      # 对外 API 模式（sha256 令牌 + key 参数剔除）
```

### 6.2 关键实现要点

**路由与「请求中不换路由」**

```rust
// 请求开始时快照 Arc<Route>，后续只改 RouteTable，不影响在途请求
let route: Arc<Route> = self.routes.get(&route_id).ok_or(Error::RouteNotFound)?;
```

`RouteTable = ArcSwap<HashMap<RouteId, Arc<Route>>>`；`Arc<Route>` 内的 `vendor_id` / `mode` 用 `Mutex` 保护（仅切换时写，读在请求开始时一次）。

**上游转发与探测**（对应 `probeUpstreamBody` + `replayBody`）

```rust
// 读至多 64 KiB 或首个 \n\n 边界，判定是否为 HTTP 200 携带错误
let (peeked, rest) = peek_stream(upstream.bytes_stream(), PROBE_MAX, |buf| classify(buf)).await?;
// 已读字节零拷贝接回：Bytes 是引用计数，不复制
let body = Body::from_stream(stream::iter(peeked).chain(rest));
```

`Bytes` 的引用计数语义让「探测 + 回放」比 Node 版的手写 `ReadableStream` 更简单且零拷贝。

**客户端断开取消**：`hyper` 在客户端断开时会 drop 响应体 future。用一个 `CancelOnDrop(CancellationToken)` 守卫包住上游请求，drop 即取消 —— 精确对应现有 `response.on("close")` 语义（且更可靠，不依赖 `writableEnded` 判断）。

**超时**：`tokio::time::timeout(10min, ...)`，超时错误映射为 `outcome = "timeout"` + 504，与现网一致。

**用量捕获**：`Stream` 包装器 tee 出前 512 KiB 到 `BytesMut`；结束后在 `spawn_blocking` 中做 `data:` 行解析与字段提取（避免阻塞 reactor）。

**健康状态**：`DashMap<VendorId, Mutex<HealthEntry>>`；持久化走 writer task 的批量 upsert，成功路径 5 s 节流、状态变化立即写。

**故障规则**：`ArcSwap<RuleSnapshot>` + `AtomicU64` 版本号；CRUD 后版本自增并重建快照。内置默认规则编译期常量，DB 不可用仍工作。

**日志**：`tracing` 写入 `logs/gateway.log`（保持文件名与轮转策略），同时结构化字段进 `logs/aivc.jsonl`。**绝不记录** API Key、请求/响应正文、终端输出；错误摘要统一 `[\r\n\t]+ → " "` + `truncate(500)`。

### 6.3 性能收益（相对现状）

| 项 | 现状 | Rust 版 |
| --- | --- | --- |
| 请求体/响应体 | Node `Buffer` 拼接 + `Transform` 计数 | `Bytes` 零拷贝 + `http_body_util` |
| 用量捕获 | 每 chunk `Buffer.concat` 后 `toString` | `BytesMut` 累积，结束一次性解码 |
| 健康/规则/供应商快照 | `Promise` 缓存 + 每次请求 `await` | `ArcSwap` 无锁读，零 await |
| 日志落库 | 同步 `node:sqlite` 阻塞事件循环 | writer task 异步，不阻塞 |
| 上游客户端 | undici `fetch` | hyper/reqwest 连接池复用 |

---

## 7. 终端层（`aivc-terminal`）

### 7.1 PTY 会话

```rust
pub struct TerminalRuntime { sessions: DashMap<TerminalId, Arc<PtySession>>, resume_keys: ResumeKeyGuard }

pub struct PtySession {
    writer: Mutex<Box<dyn Write + Send>>,   // portable-pty master writer
    child: Mutex<Box<dyn Child + Send>>,
    status: Mutex<TerminalStatusDetector>,  // 纯结构，可单测
    output_tx: mpsc::Sender<Bytes>,
}
```

- 每 PTY 一个 `spawn_blocking` 阻塞读线程（portable-pty reader 是 `std::io::Read`），输出送入 `mpsc`。
- **16 ms 合并**：独立 task 用 `tokio::time::interval(16ms)` 或「首块到达即启动 16 ms 窗口」策略，把 `BytesMut` 合并后经 IPC 事件推送。改进：推送**二进制**（`tauri::ipc::Response` 的 raw bytes）而非 UTF-8 字符串，减少序列化开销与多字节切分风险。
- `resume_key = "{target_id}\0{session_id}"` 互斥：`DashSet` + RAII guard，等价于 `pendingResumeKeys` / `activeResumeKeys`。
- 退出：清理检测器 → flush → 移除 session → 释放 guard → 销毁 route → 发 `terminal:exit`。
- 应用退出：`join_all` + `tokio::time::timeout(3s)`。
- 环境注入：构造子进程 env 时**若设置 `OPENAI_API_KEY` 则移除 `CODEX_API_KEY` / `CODEX_ACCESS_TOKEN`**，并强制 `TERM=xterm-256color`。

### 7.2 命令构造（保持逐字一致）

- `CommandBuilder` 按 `(provider, target_kind, platform)` 三元组分派，输出 `{ program, args, cwd, env }`。
- 引号：`posix_shell_quote` / `cmd_quote` 用 `proptest` 覆盖（含 `'`、`"`、`^&|<>%`、换行、非 UTF-8 边界）。
- Windows Codex 解析顺序：`where.exe codex.cmd|codex.exe` → `%PATH%` → `%APPDATA%\npm` → `%LOCALAPPDATA%\npm` → `%ProgramFiles%\nodejs` → `%ProgramFiles(x86)%\nodejs` → `%NVM_HOME%\*`。
- Codex 路由参数：`-c model_provider=… -c model_providers.…wire_api="responses" -c …base_url=…`（`-c` 值用 `JSON.stringify` 等价的双引号包裹）。
- 恢复命令恒为 `codex resume --all <id>`。

### 7.3 状态检测器（纯函数化）

```rust
pub struct TerminalStatusDetector { /* 与 TS 版字段一一对应 */ }
impl TerminalStatusDetector {
    pub fn feed_input(&mut self, data: &str) -> Option<TerminalStatusEvent>;
    pub fn feed_output(&mut self, data: &str) -> Option<TerminalStatusEvent>;
    pub fn on_exit(&mut self, code: i32) -> Option<TerminalStatusEvent>;
}
```

- 保留 32 KB 环形缓冲、提示符可见性状态机（`saw_output_without_prompt` / `prompt_was_visible` / `prompt_disappeared`）、`turn_id`、同状态去重。
- ANSI 剥离与提示符/确认/错误正则**逐字移植**，用 `OnceLock<Regex>` 静态编译。
- `on_exit(0)` 不产生「本轮已完成」。
- 纯结构 + 无 I/O ⇒ 可用 `insta` 对拍 TS 版同一批 PTY 字节流样本。

---

## 8. WSL 层（`aivc-infra-wsl`）—— 最大的单项性能收益

### 8.1 现状瓶颈

每次文件操作 spawn 一次 `wsl.exe`（60 s 兜底超时 + base64 脚本包装），单次固定开销 **50–150 ms**。会话列表遍历数百文件、批量删除/恢复时开销线性放大。

### 8.2 持久 worker 设计

```
每发行版一个常驻：wsl.exe -d <distro> -- bash -lc '<worker 脚本>'
协议：stdin/stdout 长度前缀帧（u32 LE 长度 + 载荷）
  请求  { id, op: Read|Write|Stat|List|Move|Remove|Exec, args… }  (payload 用 base64)
  响应  { id, exit_code, stdout_b64, stderr_b64 }
worker 内每个请求在**子 shell** 中执行，不污染 worker 状态
```

- **超时**：单请求超时 → kill worker + 该请求返回超时错误 → 自动降级为一次性 `wsl.exe` 执行并重建 worker。
- **保活**：worker 退出（发行版关闭、WSL 重启）时自动重连，指数退避。
- **降级开关**：`AIVC_WSL_WORKER=0` 强制一次性模式（排障用）。
- **保留**：base64 脚本透传语义、`cat > file` 写文件语义、16/32/1 MB 输出上限、Docker Desktop 发行版排除、UTF-16 `-l -q` 解码。

**预期收益**：WSL 单文件操作从 ~100 ms 降至 ~1–3 ms（约 30–50×），会话列表加载提升一个数量级。

### 8.3 宿主网关地址探测

保留现有顺序：`/dev/tcp/127.0.0.1` 探测 → `ip route` 默认网关 → `resolv.conf` nameserver → 回退 `127.0.0.1`；结果按 `(distro, port)` 缓存。

---

## 9. IPC 层与前端（`aivc-ipc` + `ui`）

### 9.1 类型安全契约（消除手工漂移）

- 全部 DTO 在 `aivc-ipc` 用 `serde` + `specta::Type` 定义。
- `tauri-specta` 构建期生成 `ui/src/bridge/bindings.ts` —— 取代手写 168 行 `codex-console-api.ts`。
- 现有 81 个 `ipcMain.handle` channel 映射为同名的 `#[tauri::command]`（或 `specta` 命名空间），**首期保持方法名与参数顺序不变**，以最小化前端改动。

### 9.2 迁移桥（关键降风险手段）

```ts
// ui/src/bridge/codex-console.ts —— 保持 window.codexConsole 形状不变
export const codexConsole: CodexConsoleApi = {
  listCachedSessions: (targetId, view) => invoke("codex_list_cached_sessions", { targetId, view }),
  onTerminalData: (handler) => {
    const un = listen<[string, Uint8Array]>("terminal:data", (e) => handler(e.payload[0], decode(e.payload[1])));
    return un;
  },
  // …168 个成员逐一映射
};
```

渲染层 19,319 行**先零改动跑通**，再按域逐步重构（终端 → 会话 → 供应商 → 工作台）。

### 9.3 前端后续演进（非首期）

- 保留设计系统 token 与组件契约（`ui-button` / `IconButton` / `Dialog` / `Pagination`）。
- `app.tsx`（1,193 行）按域拆分为 feature 入口 + 状态机（可用 XState 或纯 reducer 替代 20+ hooks 的手工编排）。
- `app.css`（8,214 行）按组件拆分为 CSS Modules 或 CSS Layers，token 层保持单文件。

---

## 10. 企业级工程标准（落地清单）

### 10.1 代码质量门禁

| 项 | 配置 |
| --- | --- |
| 格式 | `cargo fmt --all --check` |
| Lint | `cargo clippy --all-targets -- -D warnings`；`correctness`/`suspicious` = deny；`unwrap_used`/`expect_used` = deny（测试豁免） |
| unsafe | 除 `aivc-infra-wsl` 外全部 `#![forbid(unsafe_code)]` |
| 文档 | `#![warn(missing_docs)]`，公开 API 必须有文档注释 |
| 依赖 | `cargo deny check`（许可证白名单 + 禁止反向依赖 + 重复版本告警） |
| 漏洞 | `cargo audit` |
| 分层 | `xtask check-layers` 校验 crate 依赖图符合既定方向 |

### 10.2 测试策略

| 层级 | 手段 |
| --- | --- |
| 领域纯函数 | 单元测试 + `proptest`（引号转义、CLI 参数解析、路径白名单、边界断言、分页偏移数学） |
| 解析器 | `insta` 快照（Codex/Claude/Gemini/Qoder JSONL → 结构化输出） |
| 仓储 | `tempfile` + 真实 SQLite（含既有 `app.db` 升级路径测试） |
| Gateway | `wiremock`/本地 axum mock 上游；覆盖 HTTP 200 错误体、环形故障转移、熔断、头部改写、令牌鉴权、用量聚合、自定义响应、客户端断开 |
| 终端 | 状态检测器表驱动 + `insta`；命令构造逐平台断言 argv |
| Provider 契约 | 能力矩阵与子 trait 实现一致性测试（等价于现有 `ai-providers.test.ts`） |
| **对拍** | `xtask diff-test`：同一 `fixtures/` 语料分别喂给 TS 版与 Rust 版，比对 JSON 输出 |

**测试迁移**：现有 54 个测试文件、4,555 行作为验收语料逐条移植；DEV-GUIDE 已记录 2 项既存失败（`terminal-status-detector.test.ts` 确认提示识别），重构时一并修正。

### 10.3 可观测性

- `tracing` span 贯穿：`gateway.request{request_id, provider, route_id}`、`provider.session{target_id, session_id}`、`terminal{terminal_id}`、`wsl.exec{distro}`。
- 日志分级 + 轮转（`logs/aivc.log`，保留 `logs/gateway.log` 兼容）+ 可选 JSON 层。
- 保留 `performance.log` 等价物（启动阶段关键节点），并新增端到端 span 耗时。
- **绝不记录**：API Key、请求/响应正文、终端输出、会话正文。

### 10.4 秘密与安全

- API Key 用 `secrecy::SecretString` 包裹，`Debug`/`Display` 自动脱敏；仅在写上游 Header 时 `expose_secret()`。
- 令牌比较统一 `subtle::ConstantTimeEq`（替代 `crypto.timingSafeEqual`）。
- 对外 API 令牌只存 sha256；`?key=` 从日志与上游剔除。
- 配置文件路径白名单 + `..` 穿越防御（`proptest` 覆盖）。
- 会话文件目录边界断言 + 会话 ID 头部校验。
- Tauri 侧：CSP、`withGlobalTauri(false)`、最小化 capability 白名单、`dangerousRemoteDomainIpcAccess` 禁用。

### 10.5 CI/CD

```
matrix: [windows-latest, macos-latest, ubuntu-latest]
steps:  fmt → clippy → check-layers → deny → audit → test → build
release: tauri build（nsis/msi | dmg | AppImage/deb）+ 校验和 + 可选签名
```

---

## 11. 性能目标（可度量验收）

| 指标 | 现状（待补基线） | 目标 |
| --- | --- | --- |
| 冷启动 → 窗口可见 | ~1.5–3 s | **≤ 800 ms** |
| 空闲内存 RSS | ~200–400 MB | **≤ 120 MB** |
| 安装包体积 | ~80–150 MB | **≤ 15 MB** |
| 会话列表（5,000 × 64 KB 头部，冷缓存） | 待测 | **≤ 1.5 s** |
| 会话详情分页（50 MB JSONL 尾部 50 条） | 线性扫描 | **≤ 100 ms**（字节偏移 seek） |
| WSL 单文件 stat/exists | ~50–150 ms | **≤ 5 ms**（持久 worker） |
| Gateway 代理附加延迟 P99（小响应） | 待测 | **≤ 3 ms** |
| Gateway 并发 200 请求 | 待测 | 无错误、内存平稳 |

---

## 12. 分阶段落地计划

| 阶段 | 内容 | 产出/验收 |
| --- | --- | --- |
| **P0 骨架** | Cargo workspace、10 个 crate 骨架、CI 门禁、`rust-toolchain`、`deny.toml`、`aivc-core` 领域类型 + 端口 + 错误（含文案快照）、Tauri 窗口 + specta 导出管线 | `cargo test` 绿；空壳窗口可启动；TS bindings 生成 |
| **P1 配置与存储** | 运行根三形态派生、`settings.json` 读写与校验、**全新 SQLite schema（`refinery` V1 迁移，10 表）**、writer task + 读池、全部仓储 | 新库创建成功；仓储测试覆盖；`byte_offset` 分页定位基准达标 |
| **P2 平台层** | 本地 fs `SessionStore`、WSL 持久 worker + 一次性回退、shell 引号/参数解析、路径白名单与边界断言 | proptest 通过；WSL 基准达标 |
| **P3 Provider** | 注册表 + 子 trait；Codex（列表/详情分页/分支 app-server/复制/回收站）；Claude/Gemini/Qoder；会话元数据/收藏/最近/导出 | 解析器 `insta` 快照与 TS 版一致；分支端到端通过 |
| **P4 Gateway** | axum 服务、路由表、令牌鉴权、上游转发、探测回放、故障转移、熔断、规则、用量、日志、对外 API | 定向并发/故障/熔断/用量测试通过；延迟达标 |
| **P5 终端** | portable-pty 会话、命令构造、状态检测器、路由绑定、退出清理 | argv 逐平台断言；检测器对拍通过 |
| **P6 Skill / CLI 环境** | Skill CRUD/导入/回收站、环境探测、安装器 | 契约测试通过 |
| **P7 IPC + 前端桥接** | 81 个 command + specta bindings + `window.codexConsole` 适配层 | **现有渲染层零改动跑通全功能** |
| **P8 对拍与打包** | `xtask diff-test` 全量对拍、性能基准、三平台打包 | 功能一致报告；性能目标达标 |
| **P9（可选）前端重构 / 原生 UI** | 前端按域拆分、CSS 分层；（可选）纯 Rust UI 探索 | 独立决策 |

**P0–P8 为功能一致交付路径；P9 为可选增强。**

---

## 13. 已确认的决策

| # | 决策 | 结论 |
| --- | --- | --- |
| 1 | 外壳 | **Tauri 2 + 100% Rust 核心 + 保留 React/xterm.js 前端**（P9 可选再探索原生 UI） |
| 2 | 前端迁移强度 | **首期适配层 + 渲染层零改动**，跑通后再按域重构 |
| 3 | 新工程位置 | **`D:\code\ai\aivc-rs\`**（与旧工程并存，旧工程作为对拍 oracle） |
| 4 | 数据库 | **全新库，不做旧库迁移**；`data/aivc.db`，schema 从 v1 起干净设计 |
| 5 | 交付节奏 | **按 P0–P8 分阶段交付**，每阶段附验收证据 |

### 环境说明

本机未预装 Rust。工具链以**工作区内隔离**方式安装：`aivc-rs/.toolchain/{cargo,rustup}`，不修改 PATH 与用户 profile，删除该目录即可完全回退。日常构建：

```powershell
$env:CARGO_HOME  = "D:\code\ai\aivc-rs\.toolchain\cargo"
$env:RUSTUP_HOME = "D:\code\ai\aivc-rs\.toolchain\rustup"
& "$env:CARGO_HOME\bin\cargo.exe" test --workspace
```

CI 与协作者使用标准 `rustup` 安装（`rust-toolchain.toml` 固定版本），两者行为一致。

---

## 14. 重构原则：现网不合理处一律用正确方式实现

**功能一致 ≠ 照搬实现。** 凡是现网的实现方式本身有缺陷的，本工程按正确方式实现，
并在 `03-parity-matrix-and-migration.md` 的「刻意偏离」表中逐条登记。判定标准是：

1. 用户可见行为（消息文案、字段名、targetId 语法、持久化格式）**必须**一致；
2. 内部实现若存在性能缺陷、竞态、缺索引、错误抽象，**必须**修正；
3. 修正必须附带证据（基准数据、失败用例或索引计划），不接受「感觉更好」。

已落地的偏离：

| 项 | 现网做法 | 本工程做法 | 依据 |
| --- | --- | --- | --- |
| 分页锚点 | 记录 JSONL **行号** | 记录**字节偏移** | 行号锚仍需从文件头线性读到起始行，大 JSONL 分页是 O(n) |
| 缓存总量统计 | `LENGTH(session_json)`（**字符数**） | `LENGTH(CAST(… AS BLOB))`（**字节数**） | 中文正文 3 字节/字符，现网 64 MB 上限实际允许约 192 MB |
| 网关诊断索引 | 仅 `created_at`、`(vendor_id, created_at)` | 补齐 Provider 与 outcome 索引 | 诊断弹窗的筛选条件会全表扫描（已用 `EXPLAIN QUERY PLAN` 验证） |
| 分支子会话反查 | `json_extract` 全表扫描 | 表达式索引 | 同上；副作用是数据库拒绝非法 JSON，等于多一道完整性护栏 |
| 数据库写入 | `node:sqlite` 同步 API 阻塞事件循环 | 专用写线程 + 读连接池 | 保留「写入全串行」语义，但不再与网关热路径争用 |
| 会话元数据更新 | 「读 → 改 → 写」两次独立 `await` | 写连接内一次完成 | 现网并发更新同一会话的不同字段会互相覆盖（有回归测试） |
| 迁移 | `ALTER TABLE` + `try/catch`，失败即清空重建 | 版本化迁移执行器 | 升级路径显式可审计；全新库无历史包袱 |
| 时间戳 | 依赖 JS `toISOString()` | 自建公历转换 | RFC 3339 实现会裁剪尾随零，无法逐字对齐（65 组 Node 对拍固化） |
| 设置读取 | `JSON.parse` 后逐字段判类型 | 保留原始 `Value` + 归一化快照 | 现网语义是「单字段回退默认、其余照常生效」，强类型反序列化做不到 |
| 元数据读取 | 同上 | 同上（`RawSessionMetadata`） | 单字段损坏不应让用户丢掉自定义标题 |

**反面清单**（刻意**不**改的现网行为）：Codex 裸 `wsl:` 前缀、未知 target 兜底归 Codex、
`offset === -1` 哨兵、错误消息文案、`codex resume --all`、不触碰 Codex 私有 SQLite 存储。
这些是**对外契约或安全边界**，改动等于破坏功能一致。
