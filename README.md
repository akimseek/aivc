# aivc-rs — AI Visual Console（Rust 重构）

> 基于 Rust 重构的 AI CLI 工作台：会话资产、本地 Gateway、终端运行时与平台集成。
> 目标是**功能一致**（行为契约级 1:1）、**高性能**、**可持续扩展**、**企业级工程标准**。

本工程与 `../`（Electron + React + TypeScript 原实现）**并存**：原实现保留为对拍
oracle，直到 parity 矩阵全绿。

---

## 架构总览

**Tauri 2 + 100% Rust 核心 + 保留 React/xterm.js 前端。**

主进程的全部业务逻辑（会话解析、Gateway、PTY、WSL、供应商）用 Rust 重写；终端渲染
沿用 xterm.js —— 它是业界最成熟的终端仿真器，重写 IME 组合态、括号粘贴、Unicode 宽度
与重绘性能是纯粹的负收益。

```
aivc-rs/
├── crates/
│   ├── aivc-core/           ★ 领域层：实体、值对象、端口 trait、错误分类（零 I/O）
│   ├── aivc-infra/            基础设施适配器
│   │   └── src/
│   │       ├── config/        运行根派生、settings.json
│   │       ├── db/            SQLite：全新 schema + 版本化迁移 + 单写者 actor + 仓储
│   │       ├── fs/            本地文件系统 SessionStore
│   │       ├── wsl/           WSL 持久 worker + 一次性回退
│   │       ├── http/          reqwest 客户端（模型 / 余额查询）
│   │       └── logging/       tracing 初始化与日志轮转
│   ├── aivc-provider/         注册表 + 能力子 trait + Codex/Claude/Gemini/Qoder
│   ├── aivc-gateway/          axum 服务：路由/转发/故障转移/熔断/规则/用量/日志
│   ├── aivc-terminal/         portable-pty + 命令构造 + 状态检测器
│   ├── aivc-tooling/          Skill 管理 + CLI 环境探测/安装
│   ├── aivc-ipc/              Tauri command 层 + specta TS 绑定导出
│   └── aivc-app/            ★ 唯一二进制：装配、窗口、托盘、生命周期
├── scripts/                   本地工具链启动器与质量门禁
├── xtask/                     依赖分层校验等构建自动化
└── docs/                      架构与 parity 文档（见 ../docs/rust-rewrite/）
```

**为什么是 8 个 crate**：crate 边界只在两种情况值得存在 ——（a）它强制了一条重要的
依赖方向，（b）它隔离了重量级或独特的外部依赖。按这条原则，原先更细的拆分里有 6 个
crate 不成立（`observability`、`infra-{db,fs,wsl,http}`、`config`、`skills`+`cli-env`），
已合并为 `aivc-infra` 与 `aivc-tooling` 的内部模块。

> Rust 里 **crate ≈ Maven module**，每个 crate 的源码根**必须**是 `<package>/src/`
> （`lib.rs` 或 `main.rs`）——这是 Cargo 的约定，不是设计失误；`src/foo/` 相当于 Java 的 package。

**依赖方向**（由 `cargo xtask check-layers` 强制，CI 拦截反向依赖）：

```
aivc-app ──► aivc-ipc ──► { provider, gateway, terminal, tooling }
                                  │
                                  ▼
                            aivc-infra ──► aivc-core
```

`aivc-infra` 内部的模块级规则（`fs` / `wsl` / `db` / `http` 之间不得互相依赖）
由代码评审把关，详见 `crates/aivc-infra/src/lib.rs` 的模块文档。

`aivc-core` 只允许依赖 `serde` / `serde_json` / `thiserror` / `uuid` / `tracing`
（白名单在 `xtask` 中，新增依赖会被 CI 拦下）。

---

## 环境准备

本机**未安装全局 Rust**时，工具链以工作区内隔离方式安装，不修改 PATH 与用户 profile：

```powershell
# 工具链位于 .toolchain/（已 gitignore），删除该目录即可完全回退
$env:CARGO_HOME  = "$PWD\.toolchain\cargo"
$env:RUSTUP_HOME = "$PWD\.toolchain\rustup"
```

所有构建都经启动器运行，它会自动设置上述变量：

```powershell
.\scripts\cargo.ps1 build --workspace
.\scripts\cargo.ps1 test --workspace
```

> 已自行安装全局 `rustup` 的协作者可直接使用 `cargo`，启动器会自动回退。
> `rust-toolchain.toml` 固定了组件集合，CI 另行固定具体版本以保证可复现构建。

### 前置依赖

- **Windows**：需要 MSVC 生成工具（`link.exe`）与 Windows SDK（`rusqlite` 的
  `bundled` 特性会编译 SQLite 的 C 源码，因此还需要 `cl.exe`）。
- **macOS / Linux**：需要 C 编译器（`clang` / `gcc`）与 `pkg-config`。

---

## 质量门禁

```powershell
.\scripts\verify.ps1            # 格式 + 分层 + 编译 + clippy + 测试
.\scripts\verify.ps1 -Quick     # 跳过测试
```

等价的单独命令：

```powershell
.\scripts\cargo.ps1 fmt --all --check
.\scripts\cargo.ps1 run --package xtask -- check-layers
.\scripts\cargo.ps1 check --workspace --all-targets
.\scripts\cargo.ps1 clippy --workspace --all-targets -- -D warnings
.\scripts\cargo.ps1 test --workspace
```

> **PowerShell 注意**：`clippy` 需要 `--` 分隔符，而 Windows PowerShell 会在字面量
> 命令行中吞掉 `--`。请使用 `scripts/verify.ps1`，或在调用时用数组 splatting 传参。
> 启动器 `scripts/cargo.ps1` 同样受此限制。

### 工程标准

| 项 | 约定 |
| --- | --- |
| unsafe | 全部 crate `#![forbid(unsafe_code)]` |
| Lint | `clippy::correctness` / `suspicious` = deny；`unwrap_used` / `expect_used` / `panic` = deny（测试由 `clippy.toml` 放行） |
| 文档 | 公开 API 必须有文档注释；`missing_docs` = warn |
| 依赖 | `cargo-deny` 许可证白名单；禁止 `openssl-sys`（统一 rustls） |
| 分层 | `cargo xtask check-layers` |
| 错误 | 库 crate 一律 `thiserror`，禁止 `unwrap`；只有 `aivc-app` 可用 `anyhow` |

---

## 功能一致的约束

**错误消息、序列化字段名与 targetId 语法不是实现细节，而是对外契约**：
错误消息经 IPC 直接展示给用户；会话与目标结构写入 SQLite 并送往渲染层；
targetId 被持久化在设置文件里。

因此 `aivc-core` 中的字符串与 serde 形状改动等同于产品行为改动。完整验收清单见
[`../docs/rust-rewrite/03-parity-matrix-and-migration.md`](../docs/rust-rewrite/03-parity-matrix-and-migration.md)（A–L 共 12 组 150+ 项）。

几处刻意**不能**统一的历史遗留：

- Codex 的 WSL targetId 是裸 `wsl:<distro>`，其余平台是 `<provider>:wsl:<distro>`；
- 未知 target 前缀兜底归 Codex；
- 消息分页用 `offset === -1` 表示「取末尾一页」；
- 时间戳恒为 `toISOString()` 格式（UTC + 固定 3 位毫秒），不能改用会裁剪尾随零的
  RFC 3339 实现。

---

## 刻意偏离现网的实现

重构原则：**现网实现不合理的地方，一律用正确的方式实现**，并在 parity 矩阵中逐条标注。
已落地的偏离：

| 项 | 现网 | 本工程 | 理由 |
| --- | --- | --- | --- |
| 分页锚点 | 记录 JSONL **行号**，随机定位仍需从文件头线性读取 | 记录**字节偏移**，`seek` 直接定位 | 大 JSONL 分页从 O(n) 降到 O(1) |
| 缓存总量统计 | `LENGTH(session_json)` 得到**字符数** | `LENGTH(CAST(... AS BLOB))` 得到**字节数** | 中文正文 3 字节/字符，现网 64 MB 上限实际允许约 192 MB |
| 网关诊断索引 | 只有 `created_at` 与 `(vendor_id, created_at)` | 补齐 `(provider_id, created_at)` 与 `(outcome, created_at)` | 诊断弹窗按 Provider / outcome 筛选会全表扫描 |
| 分支子会话反查 | `json_extract` 全表扫描 | 表达式索引 | 同上；副作用是数据库会拒绝非法 JSON，等于多一道完整性护栏 |
| 数据库写入 | `node:sqlite` 同步 API，阻塞主进程事件循环，与网关热路径争用 | 专用写线程 + 读连接池 | 保留「写入全串行」语义，但不再阻塞异步运行时 |
| 会话元数据更新 | 「读 → 改 → 写」两次独立 `await` | 在写连接内一次完成 | 现网并发更新同一会话的不同字段会互相覆盖 |
| 迁移 | `ALTER TABLE` + `try/catch` 隐式升级，失败即清空重建 | `refinery` 式的版本化迁移（自建执行器） | 升级路径显式、可审计；全新库无历史包袱 |

---

## 交付阶段

| 阶段 | 内容 | 状态 |
| --- | --- | --- |
| **P0** | workspace 骨架、8 个 crate、CI 门禁、`aivc-core` 领域层与端口、分层守卫 | ✅ 完成 |
| **P1** | 运行根三形态派生、`settings.json` 逐字段容错读写、全新 SQLite schema（12 表）+ 版本化迁移（V1/V2）、单写者 actor + 读连接池、**8 个仓储**（会话缓存、会话元数据、消息锚点、工作区预设、供应商、网关健康、故障规则、请求日志与用量） | ✅ 完成 |
| **P2** | 本地 fs `SessionStore` + 流式行读取 + POSIX 路径边界 + shell 引号/参数解析 + WSL 适配器（持久 worker + 一次性回退 + 宿主地址探测 + `WslSessionStore`） | ✅ 完成 |
| **P3-Codex** | 解析 / 分页 / 路径围栏 / 列表 / 编排与变更 / 分支 / 复制 / 导出 / app-server 客户端 + 注册与能力自检 | ✅ 完成 |
| **P3-Claude** | 解析 / 用量累加 / 存储布局 / 分支 / 复制 / 导出 / 编排（17 个方法）+ 注册与能力自检 | ✅ 完成 |
| **P3-Gemini** | 解析 / 存储布局 / 分支 / 复制 / 导出 / 编排（17 个方法）+ 注册与能力自检 | ✅ 完成 |
| **P3-Qoder** | 解析 / 多产物存储 / 导出 / 编排（多产物原子移动 + 回滚）+ 注册与能力自检 | ✅ 完成 |
| **P3 全部** | Codex / Claude / Gemini / Qoder 四平台 + 注册表 + 能力自检 | ✅ 完成 |
| **P4-Gateway** | 路由/鉴权/上游头/故障转移/尝试规划/响应探测 + **axum 代理服务器**（8 项端到端测试） | ✅ 完成 |
| P4 剩余 | **请求记录已打通到数据库**（代理 → 累加器 → 仓储）/ 余额查询、供应商管理编排 | 🔄 进行中 |
| P7 | 参数校验层 + 会话/设置/预设/网关/供应商命令域 + 81 条命令注册表 + 命令路由器（**49/81 已接线**，缺口可追踪） | 🔄 进行中 |
| P7 剩余 | Tauri command 注册（约十行胶水）、specta 绑定、其余 32 条命令 | 待开始 |
| P5 | 终端状态检测器 + **四平台命令构造与网关路由注入**（47 项测试） | ✅ 已验证 |
| P5 剩余 | PTY 会话（编译与 Lint 已验证，**运行未验证**）、输出流送渲染层 | 🔄 进行中 |
| P6 | Skill / CLI 环境 | 待开始 |
| **P7-组装根** | ivc-app 一次启动建好全部组件（目录→库→上下文→网关来源→路由器），命令面端到端可用 | ✅ 完成 |
| **P7-Tauri 胶水** | Tauri 2.11.6 接入，单一 IPC 命令转发到命令路由器（错误映射为消息文本） | ✅ 完成 |
| **P7-应用入口** | main.rs + uild.rs + 	auri.conf.json；**可编译并启动出窗口**，加载现网已构建前端 | ✅ 完成 |
| **P7-退出清理** | A6 退出顺序（PTY → Gateway → flush），幂等 + 非关键失败不阻止退出（9 项测试） | ✅ 完成 |
| **P7-托盘** | A5 托盘：幂等创建、提示文字、右键「显示窗口/退出」、单击恢复窗口（实测可启动） | ✅ 完成 |
| **P7-窗口加固** | A1 窗口规格移入代码（可测）+ A3 导航白名单/外链转交/禁 <webview>（12 项测试） | ✅ 完成 |
| **P7-前端适配层** | window.codexConsole 桥（45 个方法映射 + 未映射显式报错）；命令表补全 **85 条**；实测前端启动即驱动 14 条命令 | ✅ 完成 |
| P7 剩余 | 关闭三选一（A4）、系统通知（A7）、事件订阅推送、其余命令接线 | 待开始 |
| **P8-验收报告器** | cargo xtask parity：把矩阵的 36 条映射到测试并校验覆盖未丢失 | ✅ 完成 |
| **P8-端到端对拍** | 终端状态检测器（24 场景）+ Codex 会话解析（15 用例）+ **shell 引号/命令构造/参数解析/目标编号/路径围栏（180 对 + 111 项输入）**，Node 作 oracle、golden 离线比对 | ✅ 完成 |
| **P8-对拍闭环** | cargo xtask regen-golden 重建 golden；四条对拍登记进验收报告（纳入项 36 → 40） | ✅ 完成 |
| **P8-端到端冒烟** | 真实文件 + 真实命令走通全流程（含真实网关注册表代理请求并落库） | ✅ 完成 |
| **P4-事件推送** | gateway:request-recorded 经记录钩子推给界面（工作台不轮询） | ✅ 完成 |
| P8 剩余 | Claude/Gemini/Qoder 解析对拍、分页对拍、性能基准、三平台打包 | 待开始 |

当前规模：111 个 Rust 文件、43,300 行；**1075 项测试全绿**（含 65 项时间戳与 Node
`toISOString()` 的逐值对拍、路径遍历防护与常量时间比较的 `proptest` 属性测试）。

门禁共七项：格式、依赖分层、编译、编译（全部特性）、Lint、Lint（全部特性）、测试。
另有验收报告器：cargo xtask parity；重建对拍 golden：cargo xtask regen-golden（需要 Node）。

### 关于 WSL 测试

WSL 的 48 项测试（worker 协议 + adapter 端到端）需要一个能运行的 bash。本机装了
Git Bash，但**受限沙箱不允许 MSYS2 bash 创建内部 signal pipe**
（`couldn't create signal pipe, Win32 error 5`），因此这些测试在沙箱内会自动跳过
（`LocalBashLauncher::is_usable` 探测），而不是把环境限制误报成代码缺陷。
以更宽权限运行时可全部通过：

```powershell
.\scripts\cargo.ps1 test -p aivc-infra wsl::
```

CI（Linux 原生 bash）无需任何额外处理。

### 关于 PTY 测试

`aivc-terminal` 的 `session` 模块（PTY 会话）放在**默认关闭的 `pty` 特性**后面。
原因与 WSL 同类：受限沙箱不允许创建控制台宿主进程，链接了 `portable-pty` 的测试二进制
直接以 `STATUS_DLL_INIT_FAILED` 退出——这是环境限制，不是代码缺陷。

关闭不等于放任腐坏：门禁额外跑一遍 `cargo check --all-features` 与
`cargo clippy --all-features`，因此该模块**始终被编译与 Lint 检查**，只是不在本沙箱内
运行。

**注意**：以更宽权限运行这批测试曾把整个会话卡死——PTY 会拉起控制台宿主进程
（`conhost.exe` / `OpenConsole.exe`），它是脱离的子进程，会活得比被测试的进程更久，
从而永久占住沙箱的作业对象，此后**任何命令都无法启动**（`0xC0000142`）。因此除非确实
需要验证 PTY 运行时行为，不要轻易运行它；运行后请检查并清理残留的 `conhost` 进程。

真实 PTY 行为需在沙箱外验证：

```powershell
.\scripts\cargo.ps1 test -p aivc-terminal --features pty
```

架构决策与理由见 [`../docs/rust-rewrite/02-rust-architecture.md`](../docs/rust-rewrite/02-rust-architecture.md)。

---

## License

MIT
