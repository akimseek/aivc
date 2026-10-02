# aivc · AI Visual Console

> **AI CLI 工作台**：把 Codex、Claude Code、Gemini、Qoder 四个 AI 命令行工具收进一个
> 带界面的控制台——会话资产、内置终端、本地网关、WSL 多目标，统一管理。

**Tauri 2 + 100% Rust 核心 + React / xterm.js 前端。** 二进制约 21 MB，安装包约 6–8 MB。

---

## 功能

- **会话资产**：四平台会话的浏览 / 全文搜索 / 收藏 / 重命名 / 分支 / 复制 / 导出
  （Markdown / JSON / HTML）/ 回收站。分支直接落到 CLI 的原生 `thread/fork`，历史完整。
- **内置终端**：一键进入会话终端（`--resume` 继续对话），xterm.js 渲染、ConPTY 运行，
  支持查找、右键复制/粘贴、自管输入框（含图片粘贴）。
- **本地网关**：axum 反向代理接管 CLI 的上游流量——路由分组、故障转移、熔断、
  用量统计、请求日志、异常诊断；GUI 里改规则即时生效。
- **多目标**：本机与 WSL 发行版并存，目标级会话隔离；WSL 走持久 worker，冷启动开销
  从每次调用 5–7 秒摊到常驻进程。
- **Skills 与 CLI 环境**：技能导入 / 启用 / 回收站；Node / Git / CLI 版本探测与安装引导。
- **双主题**：明亮 / 暗黑，终端配色随主题联动。

## 架构

```
aivc/
├── crates/
│   ├── aivc-core/           ★ 领域层：实体、值对象、端口 trait、错误分类（零 I/O）
│   ├── aivc-infra/            基础设施适配器
│   │   └── src/
│   │       ├── config/        运行根派生、settings.json
│   │       ├── db/            SQLite：schema + 版本化迁移 + 单写者 actor + 仓储
│   │       ├── fs/            本地文件系统 SessionStore
│   │       ├── wsl/           WSL 持久 worker + 一次性回退
│   │       ├── http/          reqwest 客户端（模型 / 余额查询）
│   │       ├── logging/       tracing 初始化与日志轮转
│   │       └── process.rs     子进程的 Windows 细节（如无窗口启动）
│   ├── aivc-provider/         注册表 + 能力子 trait + Codex/Claude/Gemini/Qoder
│   ├── aivc-gateway/          axum 服务：路由/转发/故障转移/熔断/规则/用量/日志
│   ├── aivc-terminal/         portable-pty + 命令构造 + 状态检测器
│   ├── aivc-tooling/          Skill 管理 + CLI 环境探测/安装
│   ├── aivc-ipc/              Tauri command 层 + 命令注册表与路由
│   └── aivc-app/            ★ 唯一二进制 **AI**（Windows 下 `AI.exe`）：装配、窗口、托盘、生命周期
├── ui/                        React 前端（Vite + vitest）
├── scripts/                   本地工具链启动器与质量门禁
├── xtask/                     依赖分层校验 / parity 报告 / golden 重建
└── docs/                      设计与验收文档（见下）
```

**依赖方向**（由 `cargo xtask check-layers` 强制，CI 拦截反向依赖）：

```
aivc-app ──► aivc-ipc ──► { provider, gateway, terminal, tooling }
                                  │
                                  ▼
                            aivc-infra ──► aivc-core
```

`aivc-core` 只允许依赖 `serde` / `serde_json` / `thiserror` / `uuid` / `tracing`
（白名单在 `xtask` 中，新增依赖会被门禁拦下）。

## 安装与运行

- **NSIS 安装器**（推荐，per-user、免管理员）：装到 `%LOCALAPPDATA%\AI 可视化控制台`，
  双击即用。
- **MSI 安装器**（per-machine，需管理员）：装到 `Program Files`；数据目录不可写时，
  应用会自动回退到 `%LOCALAPPDATA%\aivc`（日志里有一条 warn 记录两处路径）。
- **裸二进制**：构建产物 `target/release/AI.exe`（macOS / Linux 同名 `AI`），双击可跑，
  数据写在自己旁边（可写时）。
- 缺 WebView2 的机器：安装阶段需要联网引导（默认下载式引导）。

## 从源码构建

完整步骤（含隔离工具链、镜像、只打某一种安装器、实测坑位）见
**[docs/rust-rewrite/47-build-and-package.md](docs/rust-rewrite/47-build-and-package.md)**。
最短路径：

```bash
cd crates/aivc-app
npx --yes @tauri-apps/cli@^2 build     # 前端构建 + release 编译 + nsis/msi 一趟出
```

前端单独开发：`cd ui && npm install && npm run dev`（dev server 端口 5273）。

## 质量门禁

```powershell
.\scripts\verify.ps1            # 八段：BOM / 探针选择器 / 命令名 / fmt / 分层 / check / clippy×2
.\scripts\verify.ps1 -Quick     # 跳过 cargo test（Rust 全量测试另跑）
.\scripts\cargo.ps1 test --workspace
```

| 项 | 约定 |
| --- | --- |
| unsafe | 全部 crate `#![forbid(unsafe_code)]` |
| Lint | `clippy::correctness` / `suspicious` = deny；`unwrap_used` / `expect_used` / `panic` = deny（测试由 `clippy.toml` 放行） |
| 文档 | 公开 API 必须有文档注释 |
| 依赖 | `cargo-deny` 许可证白名单；禁止 `openssl-sys`（统一 rustls） |
| 分层 | `cargo xtask check-layers` |
| 错误 | 库 crate 一律 `thiserror`；只有 `aivc-app` 可用 `anyhow` |

另有两条**机械守卫**（跑在 `verify.ps1` 里）：探针脚本引用的界面类名必须真实存在；
前端 `invoke` 的命令名必须登记在后端命令表里。

## 关键设计决策（偏离旧版实现之处）

重构原则：旧版实现不合理的地方，一律用正确的方式实现。已落地的代表：

| 项 | 旧版 | 本工程 | 理由 |
| --- | --- | --- | --- |
| 分页锚点 | 记录 JSONL **行号**，随机定位从文件头线性读取 | 记录**字节偏移**，`seek` 直接定位 | 大 JSONL 分页 O(n) → O(1) |
| 缓存总量统计 | 字符数 | `LENGTH(CAST(... AS BLOB))` 字节数 | 中文 3 字节/字符，旧版 64 MB 上限实际放行约 192 MB |
| 数据库写入 | 同步 API 阻塞主进程事件循环 | 专用写线程 + 读连接池 | 保留「写入全串行」语义，不阻塞异步运行时 |
| 会话元数据更新 | 「读 → 改 → 写」两次独立 await | 在写连接内一次完成 | 并发更新不同字段会互相覆盖 |
| 迁移 | `ALTER TABLE` + `try/catch`，失败即清空重建 | 版本化迁移（自建执行器） | 升级路径显式、可审计 |

**行为契约**：错误消息、序列化字段名与 `targetId` 语法是对外契约（经 IPC 展示、
写入数据库、持久化进设置文件），改动等同于产品行为改动。几处刻意保留的历史形状：
Codex 的 WSL 编号是裸 `wsl:<distro>`（其余平台带前缀）、消息分页用 `offset === -1`
表示「取末尾一页」、时间戳恒为 `toISOString()` 形状。

## 数据与运行根

应用自有产物（`data/`、`logs/`、SQLite、日志）从**运行根**派生，与启动目录无关：

| 形态 | 运行根 |
| --- | --- |
| 开发构建 | 编译期已知的工作区根 |
| 未打包构建目录（`win-unpacked` 等） | 该目录的父目录 |
| 已安装 | 可执行文件所在目录；**不可写时回退** `%LOCALAPPDATA%\aivc`（macOS `~/Library/Application Support/aivc`、Linux `$XDG_DATA_HOME` 或 `~/.local/share`） |

环境变量 `AIVC_RUNTIME_ROOT` 可显式覆盖（测试/探针用它把数据挪到临时目录）。

## 文档

- **[docs/rust-rewrite/](docs/rust-rewrite/)** —— 设计与验收全套（48 篇，按编号阅读）：
  - [02 架构](docs/rust-rewrite/02-rust-architecture.md)、[03 parity 矩阵](docs/rust-rewrite/03-parity-matrix-and-migration.md)；
  - [43 终态对照复核](docs/rust-rewrite/43-parity-final-check.md)、
    [44 打包与装机实测](docs/rust-rewrite/44-p8-packaging.md)、
    [45](docs/rust-rewrite/45-session-click-enter.md) / [46](docs/rust-rewrite/46-terminal-strip-and-fit.md) 行为修正、
    [47 构建与打包手册](docs/rust-rewrite/47-build-and-package.md)。
- **[docs/superpowers/](docs/superpowers/)** —— 界面重做的设计规格与分批实施计划。
- **[docs/ui-redesign-demo.html](docs/ui-redesign-demo.html)** —— 界面设计稿（浏览器直接打开）。

## 测试环境注意

- **WSL 测试**：需要能运行的 bash；受限环境会自动跳过（探测后报「不可用」而不是误报
  失败）。完整跑法：`.\scripts\cargo.ps1 test -p aivc-infra wsl::`。
- **PTY 测试**：在默认关闭的 `pty` 特性后面（受限环境无法创建控制台宿主）。
  门禁始终以 `--all-features` 做编译与 Lint 检查，避免腐坏。

## License

[MIT](LICENSE)
