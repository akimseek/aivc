# 12 · 运行期布局与「上次用的平台/目标」

> 承接 [11](11-wsl-worker-root-cause.md)。这一轮处理两件事：**与旧工程彻底切割**，
> 以及**省掉每次打开应用都要手动选平台**这一步。

---

## 1. 与旧工程彻底切割

### 曾经的问题

设置文件叫 `data/user-data/settings.json`——与旧 Electron 应用**同名同址**。

实测撞上了：两个应用**写同一个文件**，`cachedTargets` 互相覆盖。文件里同时出现了

- 本工程的 `WSL · Ubuntu-24.04`（`available: false` 语义）
- 旧格式的 `Gemini：WSL：Ubuntu-24.04`（旧标签）

界面上显示谁的目标，取决于**谁最后写**。

而数据文件名是**刻意区分**的（`aivc.db`，有专门的测试守着）。设置文件却没有区分——
既然决策是「全新库、不迁移旧数据」，共用设置文件就没有任何理由。

### 改法

| | 旧工程 | 本工程（旧） | 本工程（新） |
| --- | --- | --- | --- |
| 数据库 | `data/app.db` | `data/aivc.db` | 不变 |
| 设置 | `data/user-data/settings.json` | **同名同址** | **`data/aivc-settings.json`** |
| WebView 目录 | `data/user-data/` | 沿用（但**没有任何代码接它**） | **删除** |
| 旧会话元数据 | `data/user-data/session-metadata.json` | 声明了路径（**无调用方**） | **删除** |

`user_data_dir()` 是个典型的「沿袭旧结构」的残留：**除了拼设置路径，没有任何东西用它**
（WebView 根本没接）。`legacy_session_metadata_path()` 则连调用方都没有——留着只会给
「顺手读一下旧数据」留一个入口。

新布局：

```text
<root>/
├── data/
│   ├── aivc.db
│   ├── aivc-settings.json
│   ├── cache/
│   └── vendor-backups/
└── logs/
    ├── app.log
    ├── gateway.log
    └── performance.log
```

实测确认：运行根下**不再出现 `user-data/`**。

新增测试 `settings_file_differs_from_the_legacy_project_in_name_and_place` 同时钉住
两件事：文件名不同、**且不得出现在 `user-data` 之下**。

---

## 2. 记住上次的平台与目标

### 为什么

实测日志：`listProviders` 在启动后 **14.1 秒**，`listCachedTargets` 在 **17.6 秒**——
中间 **3.5 秒是人在点平台下拉框**。

目标缓存与后端预热都已经就绪，唯一还挡在中间的就是这一步手动选择。

### 做法

放在**前端的 `localStorage`**，键名 `aivc.lastSelection`：

- 它是**界面状态**（用户上次在看哪儿），不是领域数据——不该占用一条 IPC 命令，
  也不该和网关、供应商配置混在同一个设置文件里；
- 键名带 `aivc.` 前缀，不与他人共用命名空间。

恢复逻辑接在已有的选择链上：

1. `providerId` 的初始值来自存储（`useState(readLastSelection)` 惰性初始化）；
2. 记住的 `targetId` 先放进已有的「待选中」槽位 `pendingTargetIdRef`——目标列表要等
   探测回来才有，`applyTargets` 会在那时按它匹配；
3. 若该目标**已不存在**（发行版被移除等），已有的兜底会自动落到第一个可用目标。

### 健壮性

`localStorage` 在隐私模式、配额用尽、企业策略下会直接抛错；里面的数据也可能被人手改坏
或来自另一个版本。

**一条坏记录不该让应用起不来**——那会把「少一点便利」变成「打不开」。因此读失败一律按
「没有记录」处理，写失败静默忽略。

新增 5 条前端测试覆盖：往返、空、**七种损坏输入**、存储抛错（读与写）、以及
`targetId` 非字符串时归一化为空串。

---

## 3. 实测

| | 结果 |
| --- | --- |
| 运行根布局 | 只有 `data/aivc-settings.json`，**无 `user-data/`** |
| 首屏就绪 | **523 ms**（启动 06.976 → 首批命令 07.785），预热仍在后台跑 |
| 前端测试 | 我新增的 5 条全过；`src/renderer` 全绿 |
| 门禁 | 七项全通过 |

**关于前端测试套件的既有失败**：全量 `vitest` 有 3 条失败，全部位于
`src/electron/`（旧 Electron 代码：`terminal-status-detector`、`gateway-failover-rules`）。
本轮的改动只在 `src/renderer/src/hooks/`，两个失败文件只 import 各自的 electron 模块，
**与本轮改动无关**——它们是旧工程的既有失败，不在重构范围内。

---

## 4. 补上缺口：WebView 的数据目录也回到运行根

### 问题

`tauri.conf.json` 没有配数据目录，Tauri 2 默认把 `WebView` 的数据放在
`%LOCALAPPDATA%\com.aivc.console\EBWebView`。于是：

- 本模块开头那条「应用绝不写入系统用户目录」的约束**并不成立**；
- 更要紧的是 **`localStorage` 也跟着跑到系统目录**——而「上次用的平台与目标」
  （见 §2）正好存在那里，应用因此**不便携**。

旧工程用 Electron 的 `app.setPath("userData", …)` 做过同一件事；本工程此前只有一个
`user_data_dir()`，它**只被用来拼设置路径，从未接到 `WebView` 上**——那条约束于是
只是文档里的一句话。

### 改法

`WebviewWindowBuilder::data_directory(paths.webview_data_dir())`，目录为
`data/webview/`（同样不沿用旧工程的 `user-data` 名字），并加入
`required_directories()`——**窗口创建之前它必须存在**。

**平台差异**：只有 Windows（`WebView2`）支持自定义数据目录；macOS / Linux 上该调用
无效果。因此用 `#[cfg(target_os = "windows")]` 只在 Windows 上设置，**不去依赖未定义
行为**。

### 实测

| | 结果 |
| --- | --- |
| 运行根 | `data/webview/EBWebView` 已建立 |
| 系统目录 `%LOCALAPPDATA%\com.aivc.console` | **未被写入**（296 个文件的最后修改时间不变） |
| 前端 | `renderer.mounted` 正常，首批 IPC 命令照常到达——换数据目录没有把窗口换坏 |
| 预热 | 首次启动 `count=3 elapsed_ms=2939` |

### 一次性代价

数据目录一换，**已有的 `localStorage` 会丢一次**。目前那里只有「上次用的平台与目标」，
丢了就等于回到「手动选一次」，可接受；这也正是**现在换比以后换便宜**的原因。

