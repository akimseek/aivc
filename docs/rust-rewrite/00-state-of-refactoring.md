# 重构现状摘要（单页入口）

> 这份文件是**接手入口**：读完它就能继续，不依赖任何会话记忆。
> 最后更新：L 分区接口层审计完成、命令面 70/85 时。

## 一、目标（不变）

把 `D:\code\ai`（Electron + TypeScript）重构为 **Tauri 2 + 100% Rust 内核**，
新工程在 `D:\code\ai\aivc-rs`。要求：

1. **功能一致**，但实现方式取其最合理者
2. **高性能、可持续扩展**，企业级标准
3. **老工程实现不合理的，用正确的方式重做**

前端 React + xterm.js **原样复用**，通过适配层（桥）对接 Rust 命令面。
**不迁移历史数据**，SQLite 从空库开始。

## 二、架构（分层由 `cargo xtask check-layers` 强制）

```
aivc-core(0) → aivc-infra(1) → {aivc-provider, aivc-gateway, aivc-terminal, aivc-tooling}(2)
             → aivc-ipc(3) → aivc-app(4)          xtask 不参与分层
```

- edition 2024，rust-version 1.88，resolver "3"
- lint：`unsafe_code=forbid`、`missing_docs=warn`、`unwrap_used/expect_used/panic` deny
- `clippy.toml`: `too-many-lines-threshold = 150`（分派表的长度与命令数成正比，非复杂度）
- 集成测试在 `tests/` 下，需逐文件 `#![allow(clippy::panic, clippy::expect_used)]`

## 三、质量门禁（7 项，必须全绿）

```powershell
$root = "D:\code\ai\aivc-rs"
$env:CARGO_HOME  = "$root\.toolchain\cargo"
$env:RUSTUP_HOME = "$root\.toolchain\rustup"
$env:RUSTUP_TOOLCHAIN = "stable-x86_64-pc-windows-msvc"
$env:PATH = "$env:CARGO_HOME\bin;$env:PATH"
Set-Location $root
& "D:\code\ai\aivc-rs\scripts\verify.ps1"
```

fmt → check-layers → check → check --all-features → clippy → clippy --all-features → test。

**当前：全绿，1357 项测试。**

## 四、当前进度

| 维度 | 状态 |
| --- | --- |
| 质量门禁 | ✅ 7 项全绿 |
| 测试 | **1357 项** |
| 命令面 | **85 条 · 已接线 70 · 待接线 15** |
| parity 矩阵 | 44 条纳入自动校验（矩阵共 185） |
| L 接口层审计 | ✅ **完成**（见 `06-l-audit-frontend-vs-bridge.md`） |
| L 行为层（L1–L20） | ❌ **未逐条核对** |

### 待接线的 15 条，构成清楚

| 类别 | 数量 | 说明 |
| --- | ---: | --- |
| **离线做不了** | 5 | 剪贴板 ×2、目录选择、系统通知、Skill 导入——均需新依赖，本环境 TLS 受限无法拉取 |
| **需窗口控制** | 2 | `app:command`、`app:exit-choice` |
| **Qoder 专属** | 1 | `models:list`——**语义已查清**（见 06 文档），需先定位 `listQoderModels` 定义 |
| **终端相关** | 其余 | `terminal:start-system`、Gemini/Claude/Qoder 终端构造器、`terminal:attention-notification` |
| **Gateway 生命周期** | 3 | 开启失败回滚、端口变更重启、稳定性 |

## 五、下一步（按优先级）

1. **`models:list`** —— 先找到 `listQoderModels` 的定义（换搜索方式），读懂语义再实现
2. **剩余命令**：能做的做完后，剩下的都是「离线做不了 / 需窗口控制」
3. **L1–L20 行为层核对** —— 需要人的眼睛，我会给逐条清单
4. **P8**：性能基准、三平台打包、剩余对拍（Claude/Gemini/Qoder 解析）

## 六、本工程的关键约定与陷阱（血泪）

### 编辑代码

**绝不用 PowerShell 字符串替换做结构化 Rust 编辑。** 锚点落在「文档注释 + item」之间时，
插入会把注释挤到错位置——本工程已因此损坏 3 次（`RepositorySource`、`GatewayState` 各一次）。
用 `edit` / `write` 工具。

**PowerShell 双引号里 `"$var.property"` 会被解析成属性访问**，静默得到空串。

### 「静默失败」是本工程最大的敌人

已修 8 个同类缺陷，它们的共同形态是**测试全绿而功能失效**：

| 缺陷 | 表现 |
| --- | --- |
| Tauri `event` 挂在 `core.event`（错）+ 缺 capability 权限 | 事件全部收不到，界面看起来正常 |
| 桥漏映射（`logPerformance` 等 **6 个**） | 「适配层尚未映射方法」，命令明明能用 |
| 桥丢弃 `providerId` 参数（2 处） | 目标列表变成「各平台的」，会话打不开 |
| `App::start` 用**测试版** `LocalBashLauncher` | 发布版**永远看不到 WSL** |
| `main.rs` 的 `discover_bash()` 抢占生产路径 | 同上，且日志只说「未找到 wsl.exe」 |
| WSL worker 一台退出、一台挂死 | 目标探测 **198 秒** |
| `list_distros` 忽略 `exit_code` | 错误文本被当成发行版名解析 |

**因此本工程的规矩是「失败必须可见」**：
- 后端日志落 `logs/app.log`（启动截断、关 ANSI）
- 前端异常 + UI 动作轨迹落 `logs/performance.log`（桥的 `traceAction`）
- 每一处降级都要出声（`detect()` 失败要列出查过的候选路径）

### 已验证的行为（改动时别破坏）

- **PTY**：`kill()` 与 `Drop` 都必须 `kill + wait + release`；ConPTY 会发 `ESC[6n`，
  **不回就不前进**，测试必须用 `drain_answering_dsr` 应答。`pty` feature **默认开启**。
- **WSL**：生产用 `WslExeLauncher`（`bash` 参数只给测试）；`use_worker: false`（worker 不可用）；
  启动时后台预热目标并填缓存，避免用户选平台时等 3.3 秒冷启动。
- **快捷访问**（`list-session-quick-access`）：只读缓存**不触发探测**；`recent` 要排除已进
  `favorites` 的项。

### 前端调用 vs 桥映射

**这是一条可机械核对的检查**，值得反复跑：

```powershell
# 提取前端 codexConsole.* 调用，与 bridge.rs 的 METHODS/EVENTS 求差集
# 注意：事件条目的 ( 与 " 之间可能有换行，正则要吃这个
```

结果写入 `06-l-audit-frontend-vs-bridge.md`。

## 七、已知的「错判模式」（我犯过三次）

**没先查清就下判断：**

1. 以为 `branch_session` 没实现 → 实际早已实现，只差路由一行
2. 以为 `get_session_folder_path` 没实现 → 同上
3. 以为 `list-session-quick-access` 要注入仓储 → `ctx.session_metadata()` 早已存在

**加上一次反向错误**：以为 `codex:set-wsl-codex-home` 命令没接线 →
实际只是桥没映射，我加的分支造成重复。

**规矩：PENDING 只说明「路由器没接」，既不等于「底下没实现」，也不等于「桥上没映射」。先 grep。**
