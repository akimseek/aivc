# L 分区审计结果：前端调用 vs 桥映射

> 审计方式：把渲染层调用的**每一个** `codexConsole.*` 方法，与适配层的映射逐一对照。
>
> 桥对未映射的方法会抛「适配层尚未映射方法：X」，因此**差集就是缺陷清单**。
> 这是机械的、彻底的，而且不依赖目视界面。

## 为什么用这个方法

L1–L20 是**交互行为**，看起来只能靠人在界面上逐条试。但有一层是可以机械核对的：

**渲染层需要的每一个后端方法，适配层是否提供了。**

前端是原样复用的现网产物，它调用什么由代码决定，不由人眼决定。
因此「前端调用集 − 桥映射集」是一份**完备且可重复**的缺口清单。

## 结果

| 指标 | 数量 |
| --- | ---: |
| 前端调用的方法 | **83** |
| 桥映射的方法 | **80** |
| **剩余缺口** | **10** |

### 本轮据此修掉的（5 个）

全部是 **「命令已接线、桥漏映射」**——与 `logPerformance` 同一类缺陷：

| 方法 | 命令 | 性质 |
| --- | --- | --- |
| `checkCliEnvironment` | `cli:check-environment` | 命令已接线，桥漏映射 |
| `installCli` | `cli:install` | 命令已接线，桥漏映射 |
| `setGatewayPort` | `gateway:set-port` | 命令已接线，桥漏映射 |
| `setWslCodexHome` | `codex:set-wsl-codex-home` | 命令已接线，桥漏映射 |
| `clearWslCodexHome` | `codex:clear-wsl-codex-home` | 命令已接线，桥漏映射 |

**这一类缺陷的共同形态**：命令面完全可用、领域层完全实现、测试全绿，
**但前端调不到**——它拿到的是「适配层尚未映射方法：X」。
任何只测 Rust 侧的验证都发现不了它。

### 两处我犯的错，如实记录

1. **假阳性**：对照脚本第一次报 `onTerminalAttentionNotificationClicked` 缺失。
   实际是正则 `\("(on\w+)"` 的缺陷——事件条目的 `(` 与 `"` 之间**有换行**。
   我照假阳性「补齐」，造成重复定义。已修正正则并重跑。
2. **误判层次**：我一度以为 `codex:set/clear-wsl-codex-home` **命令**没接线，
   于是往路由器里加了分支——结果与既有分支重复（编译报 `unreachable pattern`）。
   **它们早就接线了，缺的只是桥映射。** 这是「PENDING 只说明路由器没接」的
   反向错误：我这次反过来把「桥没映射」当成了「路由器没接」。

### 剩余 10 个缺口

| 方法 | 命令 | 为什么没做 |
| --- | --- | --- |
| `chooseDirectory` | `dialog:choose-directory` | **离线做不了**：需 `tauri-plugin-dialog` 或 `rfd` |
| `copyText` | `clipboard:copy-text` | **离线做不了**：需剪贴板插件 |
| `readText` | `clipboard:read-text` | **离线做不了**：同上 |
| `notifyTerminalAttention` | `terminal:attention-notification` | **离线做不了**：需系统通知插件 |
| `importSkill` | `skill:import` | **离线做不了**：领域层已实现，缺文件选择对话框 |
| `appCommand` | `app:command` | 需窗口控制（最小化/最大化/全屏/置顶/重载） |
| `chooseExitAction` | `app:exit-choice` | 需窗口控制；与 `app:request-exit` 事件是一对（事件问、命令答） |
| `listModels` | `models:list` | **Qoder 专属**，见下节 |
| `listSessionQuickAccess` | `codex:list-session-quick-access` | ✅ **已完成** |
| `openSkillFolder` | `skill:open-folder` | ✅ **已完成** |

**构成**：5 条离线做不了 · 2 条需窗口控制 · 1 条 Qoder 专属（`models:list`）· 2 条已完成

### `models:list` 的语义（已查清，尚未实现）

它不是「列出可用模型」的通用接口，而是 **Qoder 专属**：

```ts
// src/electron/ipc/vendors.ts:109-113
ipcMain.handle("models:list", async (_event, targetId) => {
  if (getProviderIdFromTargetId(targetId) !== "qoder")
    throw new Error("当前目标不支持 CLI 模型列表。");
  return listQoderModels(targetId);
});
```

**要点**：

- **非 Qoder 目标必须报错**，文案「当前目标不支持 CLI 模型列表。」——不是返回空列表
- 数据源是 **Qoder CLI 自身的模型清单**，与 `vendor:list-models`（查供应商 HTTP 接口）是两件事
- 前端只在 `composer-input.tsx:127` 用它

**实现前需先读 `listQoderModels` 的实现**（本轮按 `function listQoderModels` 搜索未命中，
可能命名或导出形式不同）。

### `models:list` 的完整语义（已查清）

定义在 `src/electron/providers/qoder/qoder-provider.ts:79-99`，
经 `src/electron/ipc/vendors.ts:11` 以 `listModels as listQoderModels` 引入。

```text
listModels(targetId):
  1. 解析目标上下文（local 或 wsl）
  2. 执行 qodercn --list-models
       wsl   → runWslShell(distro, "qodercn --list-models", 20_000)
       local → execFile("qodercn.cmd"(win32) / "qodercn", ["--list-models"], {timeout: 20_000, maxBuffer: 1 MB})
  3. parseQoderModelList(output)
```

解析规则（`parseQoderModelList`）：

```text
按行 split(/\r?\n/)
  → 去掉 ANSI 转义序列
  → trim
  → 丢掉空行
  → 丢掉整行等于 "MODEL" 的（表头）
  → 其余每行即 { id }
```

### 已完成的部分

**`aivc-tooling::qoder_models::parse_model_list`** —— 纯函数，**8 项测试**覆盖：

| 测试 | 守住的性质 |
| --- | --- |
| 表头与空行被丢掉 | 基本解析 |
| **表头匹配大小写不敏感** | 不能假设一定是大写 |
| **ANSI 转义被剥离** | 否则得到 `\u{1b}[32mqwen\u{1b}[0m` 这样的「模型名」 |
| `\r` 被 trim | Windows 输出 |
| **含 MODEL 字样的模型保留** | 只丢整行等于表头的情况 |
| **顺序保留、不去重** | 篡改顺序会让「与 CLI 输出一致」失效 |
| 空输出 → 空列表 | |
| 孤立 `ESC` 不吞掉后面内容 | |

### 尚未完成的部分

**执行 `qodercn --list-models` 并接线**。需要的：

1. Qoder provider 目前**没有命令执行能力**（无 `CommandRunner` 之类）
2. 本地与 WSL 两条路径：本地是 `qodercn.cmd`（win32）/ `qodercn`；WSL 走 `runWslShell`
3. 非 Qoder 目标必须报错「当前目标不支持 CLI 模型列表。」——**不是返回空列表**
4. 超时 20 秒、输出上限 1 MB

**建议**：先给 Qoder provider 加一个可注入的 `CommandRunner`（与 `aivc-terminal` 的
`CommandRunner`、`aivc-ipc` 的 `HttpFetcher` 同一模式），再接线。

### `codex:list-session-quick-access` 的实现记录

**我曾判断它需要「给命令层注入元数据仓储」——错的。** `ctx.session_metadata()` 早已存在，
而所有命令都接收 `ctx`，直接加方法即可。

算法（与现网 `ai-providers.ts:177-237` 一致）：

```text
候选 = favorite || lastOpenedAt
排序三级兜底：lastOpenedAt → session.updatedAt → session.createdAt
favorites = 前 12
recent    = lastOpenedAt 有值 且 不在 favorites 里，前 8
```

两个特意处理的地方：

1. **`recent` 排除已进 `favorites` 的项**（按 `targetId:sessionId`）——否则同一条会话出现两次
2. **只读缓存、不触发探测**——它要立即显示，而全量探测实测 3.3 秒。集成测试断言耗时 < 1 秒

## 这个方法的局限

它只覆盖「**接口是否存在**」。**返回的数据形状是否匹配**是另一层，
由「形状审计」单独覆盖（见下节）。两者都覆盖不到**行为是否正确**——
那只能靠人的眼睛。

## 形状审计（第二阶段）

**动机**：只核对「方法是否存在」会漏掉「方法存在、字段不对」——
这类缺陷让前端**拿到数据却渲染不出来**，而 TS 对多余或缺失的可选字段都不报错。

方法：逐类型对照前端的 TS 定义与 Rust 的 serde 序列化。

| 类型 | 对照结果 |
| --- | --- |
| `AiTarget` / `CodexTarget` | ✅ **逐字段一致**：`id`/`provider`/`label`/`kind`/`distro?`/`codexHome?`/`available`/`detail?` |
| `AiSession` / `CodexSession` | ✅ **逐字段一致**（含 `sourceTitle?`/`modelStatus?`/`fileMtimeMs?`/`fileChangeMs?`/`fileSize?`/`previewOffset?`） |
| `AiMessage` / `CodexMessage` | ✅ **一致**；`Role` 用 `rename_all = "lowercase"` → `"user"｜"assistant"｜"system"｜"tool"｜"unknown"` |
| `InstalledSkill` | ✅ **一致**。Rust 的 `InstalledSkill` 无 serde 派生，由 `aivc-ipc::skills::to_values` 手工转 JSON，用的是 `sourceName` |
| `SessionQuickAccessItem` | ❌ **曾有缺陷**（见下） |

### 形状审计抓到的缺陷：内部排序键泄漏

前端声明：

```ts
type SessionQuickAccessItem = { target; session; favorite; lastOpenedAt? }
```

而我 `list_quick_access` 的返回里多了 **`_sort` / `_label` / `_title`** 三个
**只用于内部排序的键**。

TS 不会因多余字段报错，**测试与界面都不会暴露它**——但它让契约变模糊、载荷无谓变大。

**已修**：抽出 `SessionCommands::public_item` 在返回前剥掉内部键，
并在集成测试里加了防泄漏断言：

```rust
for leaked in ["_sort", "_label", "_title"] {
    assert!(item.get(leaked).is_none(), "{list} 里泄漏了内部键 {leaked}");
}
```

### 尚未审计的形状

| 类型 | 命令 |
| --- | --- |
| `AiProviderSummary` + `AiProviderCapabilities` | `listProviders` |
| `ApiVendor` 及各类摘要 | `listApiVendors`、余额、用量报告 |
| `SessionMessagePage` | `getSessionMessagesPage` |
| 网关各摘要（用量、健康、失败诊断） | `gateway:*` |
| `SessionBatchMutationResult`、`ExportResult` 等操作结果 | 批量与导出 |

## 建议的后续

1. **先做那 3 条可实现的**
2. 剩下的各自需要新依赖或窗口控制，留到有网络/需要时
3. **L1–L20 的行为层**仍需目视核对，但范围已经小得多

### 已完成（本轮）

| 项 | 结果 |
| --- | --- |
| 桥映射补齐 | `checkCliEnvironment`、`installCli`、`setGatewayPort`、`setWslCodexHome`、`clearWslCodexHome` |
| `skill:open-folder` | **已实现**。新增 `SkillCommands::skill_root`（返回 `$CODEX_HOME/skills`），路由分支与 `shell:open-session-folder` 同构 |

命令面：**68 → 69 / 85**。剩余缺口 **9**。

---

## `codex:list-session-quick-access` 的实现方案（已查清，可直接动手）

### 现网算法（`src/electron/providers/ai-providers.ts:177-237`）

```text
1. entries = 全部会话元数据
2. candidates = entries 中 favorite === true 或 lastOpenedAt 有值的
3. targetsById = 已缓存的目标；丢掉 targetId 不在其中的候选
4. 按 targetId 分组，每组取该目标的**已缓存会话**（active）
5. items = 候选 ∩ 能查到会话的那些
6. favorites = items 中 favorite 的，按
       (lastOpenedAt || session.updatedAt || session.createdAt) 倒序
     → target.label → session.title
   取前 MAX_QUICK_ACCESS_FAVORITES = 12
7. recent = items 中 lastOpenedAt 有值、且**不在 favorites 里**的，同样排序
   取前 MAX_QUICK_ACCESS_RECENT = 8
8. 返回 { favorites, recent }
```

### 两个容易漏的点

- **`recent` 要排除已经进 `favorites` 的项**（按 `target.id:session.id` 去重），否则同一条会话会出现在两个列表里。
- **排序的兜底链**是三级：`lastOpenedAt` → `session.updatedAt` → `session.createdAt`。任意一级都可能缺。

### 可实现性

| 需要的东西 | 状态 |
| --- | --- |
| `SessionMetadataRepo::list_all()` | ✅ 已有（`aivc-infra/src/db/repos/session_metadata.rs:86`） |
| 已缓存目标 | ✅ `list_cached_targets` |
| 已缓存会话 | ✅ `list_cached_sessions(target_id, "active")` |
| 上限常量 | ✅ `MAX_QUICK_ACCESS_FAVORITES = 12`、`MAX_QUICK_ACCESS_RECENT = 8` |

**唯一要处理的接线问题**：`SessionCommands` 目前只有 `registry` + `targets`，
**拿不到元数据仓储**。需要给它加一个 `Arc<SessionMetadataRepo>`（或经由 `ProviderContext`），
再在组装根注入。这与 `with_gateway` 的做法一致。

**本项因此不是「加一个方法」，而是「给命令层加一条数据通路」**，需要一整轮。
