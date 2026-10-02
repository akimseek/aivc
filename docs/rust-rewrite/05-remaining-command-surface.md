# 命令面剩余工作盘点

> 盘点日期：本次会话。
> 目的：把「待接线 12 条」按**实际缺口**分类，而不是笼统地叫「未实现」。
>
> **做这份盘点是因为我犯过两次同类错误**：把「路由器没接」当成「领域层没写」，
> 于是重复实现了 `branch_session`（已存在）与 `session_folder_path`（已存在）。
> PENDING 只说明**路由器没接**，不代表底下没有实现。

## 分类

| 类别 | 含义 | 剩余条数 |
| --- | --- | ---: |
| **A. 只差路由器一行** | 领域层已实现，只缺命令面接线 | 0（两条已在本轮接掉） |
| **B. 需要写领域层** | 底下确实没有实现 | 5 |
| **C. 需要新依赖** | 离线环境无法拉取 | 4 |
| **D. 需要窗口控制** | 依赖 Tauri 窗口 API 的具体用法 | 3 |

---

## 本轮已接掉的两条（原属 A 类）

| 命令 | 领域层位置 |
| --- | --- |
| `codex:branch-session` | `aivc-ipc::sessions::SessionCommands::branch_session` |
| `shell:open-session-folder` | `aivc-ipc::sessions::SessionCommands::get_session_folder_path` |

**教训**：这两条都曾被我判断为「需要写实现」。实际只需要在路由器里加一行。

---

## B. 需要写领域层（5 条）

| 命令 | 缺什么 |
| --- | --- |
| `codex:list-session-quick-access` | 需要「最近打开 + 收藏」的**聚合查询**：取最近 N 条与收藏 M 条、去重、收藏优先并从最近中排除。现网有明确上限（12 收藏 / 8 最近），我的仓储里没有这个方法 |
| `models:list` | 需要按目标列出可用模型（与 `vendor:list-models` 不同：那个查供应商的 HTTP 接口，这个查 CLI 自身的模型清单） |
| `performance:log` | 启动埋点写入。需要决定落到哪里（现网写 `performance.log`），以及是否需要轮转 |
| `skill:import` | 领域层的 `import_from_path` **已实现**；缺的是**原生文件选择对话框**（见 C 类） |
| `terminal:start-system` | 需要**系统终端探测**：现网支持 9 种 kind（Windows Terminal / PowerShell / cmd / Git Bash / …），按平台探测可用项。`validate.rs` 里只有 kind 的白名单校验，没有探测 |

## C. 需要新依赖（4 条，离线不可得）

| 命令 | 需要的依赖 |
| --- | --- |
| `dialog:choose-directory` | `tauri-plugin-dialog` 或 `rfd` |
| `clipboard:copy-text` | `tauri-plugin-clipboard-manager` 或 `arboard` |
| `clipboard:read-text` | 同上 |
| `terminal:attention-notification` | 系统通知（`tauri-plugin-notification`） |

**这四条不是「没做」，是「做不了」** —— 当前环境离线，无法拉取 crate。
在能联网的环境里，它们各自是薄封装。

## D. 需要窗口控制（3 条）

| 命令 | 说明 |
| --- | --- |
| `app:command` | 最小化 / 最大化 / 全屏 / 置顶 / 重载。需要 `WebviewWindow` 的具体 API |
| `app:exit-choice` | 关闭三选一的**应答**（`minimize` / `quit` / `cancel`）。它与 `app:request-exit` 事件是一对：事件问、命令答 |
| `skill:open-folder` | 打开 skill 目录。逻辑与 `shell:open-session-folder` 同构，但需要先解析 skill 路径 |

---

## 结论

**剩余 12 条的真实构成**：

- **4 条做不了**（离线缺依赖）
- **5 条需要写领域层**，其中 `list-session-quick-access` 与 `terminal:start-system` 是真正的功能缺口
- **3 条是窗口/系统集成**，各自是薄封装

**没有一条是「大规模未实现」**。命令面已经接近完整：**85 条中 67 条已接线**。

## 另一件更重要的事

命令面之外的剩余**不在这个清单里**，而它们比这 12 条更重：

| 项 | 状态 |
| --- | --- |
| **L 渲染层 20 条** | 前端能渲染，但**交互行为一条都没核对过** |
| **P8 性能基准** | 未开始 |
| **P8 三平台打包** | 未开始 |
| **P8 剩余对拍** | Claude/Gemini/Qoder 解析、分页、网关判定 |
| **A2 CSP / A7 系统通知** | 未做 |
| **Gateway 日志文件轮转** | 未做 |

**这些才是「重构是否完成」的真正决定项**，而不是那 12 条命令。
