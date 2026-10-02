# 功能一致（Parity）验收矩阵与迁移计划

> 本文件是「功能一致」的**可验证清单**。Rust 版每完成一项，在此勾选并附证据（测试名 / 对拍报告）。
> 图例：✅ 必须一致 ｜ ⚠️ 行为需保留但实现可改 ｜ 🔼 允许改进（不破坏契约）

---

## A. 桌面外壳与生命周期

| # | 行为 | 级别 | 证据 |
| --- | :-: | :-: | --- |
| A1 | 窗口 1280×820，最小 980×640，标题「AI 可视化控制台」，背景 `#f6f7f9` | ✅ | 配置断言 |
| A2 | `contextIsolation` 等价隔离、无 Node 注入、CSP（打包环境） | ✅ | 安全测试 |
| A3 | 外链走系统浏览器；`will-navigate` 白名单；禁用 webview | ✅ | 集成测试 |
| A4 | 关闭 → 三选一确认（最小化到托盘 / 直接退出 / 取消） | ✅ | UI 测试 |
| A5 | 托盘：单击恢复窗口；右键「显示窗口 / 退出」 | ✅ | UI 测试 |
| A6 | 退出清理顺序：PTY 全停 → Gateway 停 → 日志 flush，完成后才退出 | ✅ | 生命周期测试 |
| A7 | 窗口最小化/失焦时系统通知；按事件签名去重；聚焦时不打扰 | ✅ | 通知测试 |
| A8 | 应用显示名与 AppUserModelId（Windows）与安装包一致 | ✅ | 配置断言 |
| A9 | 原生菜单为空，全部菜单由渲染层实现 | ✅ | 配置断言 |
| A10 | `performance.log` 等价启动埋点（create / ready-to-show / did-finish-load / did-fail-load / render-process-gone / renderer.mounted） | ⚠️ | 日志断言 |

## B. 路径与配置

| # | 行为 | 级别 | 证据 |
| --- | :-: | :-: | --- |
| B1 | 运行根三形态派生（dev=工作目录 / Windows 未打包=release 父目录 / 安装包=安装目录旁） | ✅ | 单测 |
| B2 | `data/`（`app.db`、`cache/`、`vendor-backups/`）、`logs/` 从运行根派生 | ✅ | 单测 |
| B3 | WebView userData 落到 `data/user-data`，不写系统用户目录 | ✅ | 单测 |
| B4 | `settings.json` 全部 9 个字段 + 默认值 + 校验范围一致 | ✅ | 快照测试 |
| B5 | `settings.json` 原子写（临时文件 + rename） | ⚠️ | 单测 |
| B6 | 损坏配置 → 降级空配置 + 备份 `<path>.broken-<ts>` + 缓存降级结果 | ✅ | 单测 |
| B7 | `cachedTargets` 按 provider 合并覆盖（非全量替换） | ✅ | 单测 |
| B8 | `wslCodexHomes` 覆盖优先于自动探测 | ✅ | 单测 |

## C. 数据库

| # | 行为 | 级别 | 证据 |
| --- | :-: | :-: | --- |
| C1 | 10 张表结构、列名、主键、唯一索引一致 | ✅ | schema 对拍 |
| C2 | 唯一索引 `(provider_id, name_norm)` 与 `(provider_id, sort)` | ✅ | 单测 |
| C3 | WAL + `busy_timeout=5000` + `foreign_keys=ON` | ✅ | PRAGMA 断言 |
| C4 | **全新库 `data/aivc.db`**，`refinery` 从 `V1__initial.sql` 起版本化迁移 | ✅ | 迁移测试 |
| C5 | 新库无历史兼容列（无 `ALTER TABLE` + try/catch 隐式升级路径） | ✅ | schema 断言 |
| C6 | 写入全串行（`BEGIN IMMEDIATE` 语义），失败回滚 | ✅ | 并发测试 |
| C7 | `session_cache` 上限 4,000 条 / 64 MB / 单条 1 MB，按 `cached_at` 升序 LRU | ✅ | 单测 |
| C8 | 缓存损坏单条忽略、不整体失效 | ✅ | 单测 |
| C9 | `session_message_index` 只存位置不存正文 | ✅ | schema 断言 |
| C10 | `session_message_index` 以 `byte_offset` 为锚，实现真 O(1) 随机定位 | 🔼 | 基准测试 |
| C11 | 不读取、不写入旧工程 `app.db`（旧库保持原样） | ✅ | 单测 |

## D. Provider 注册与目标

| # | 行为 | 级别 | 证据 |
| --- | :-: | :-: | --- |
| D1 | 4 个平台注册顺序 Codex / Gemini / Claude / Qoder 与显示名一致 | ✅ | 契约测试 |
| D2 | 能力矩阵逐项一致（10 项 × 4 平台） | ✅ | 契约测试 |
| D3 | 不支持能力保留**显式拒绝**与文案 | ✅ | 契约测试 |
| D4 | 未知 target 兜底为 Codex（兼容行为，不固化为契约） | ✅ | 契约测试 |
| D5 | Codex 裸 `wsl:<distro>`；其余 `<provider>:wsl:<distro>` | ✅ | 单测 |
| D6 | 解析函数只认本 provider 前缀，不模糊匹配他方 | ✅ | 单测 |
| D7 | 目标探测 30 s TTL + 并发合流为一次探测 | ⚠️ | 并发测试 |
| D8 | Windows 目标列表顺序：WSL 在前、local 在后（非 Windows 相反） | ✅ | 单测 |

## E. 会话读取与分页

| # | 行为 | 级别 | 证据 |
| --- | :-: | :-: | --- |
| E1 | 列表用 64 KB 头部轻量解析；详情用全量流式解析 | ✅ | 单测 + 基准 |
| E2 | 列表预览最多 8 条 + 真实 `messageCount` | ✅ | 快照 |
| E3 | 合成消息块过滤（`<environment_context>` / `<skill>` / `<user_instructions>` / `<developer_instructions>` / 多代理主代理提示） | ✅ | 快照 |
| E4 | 连续同 `role+text` 去重 | ✅ | 快照 |
| E5 | 标题 = 首条合格 user 消息，裁剪至 88 字符（`...` 结尾） | ✅ | 快照 |
| E6 | `session_meta` 提取 `id/timestamp/cwd/model/cli_version` | ✅ | 快照 |
| E7 | 用量提取（Codex `token_count`：total/last/contextWindow/contextPercent/contextLeftPercent/rateLimits） | ✅ | 快照 |
| E8 | 模型状态合并（`session_meta` + `turn_context` 的 `model/reasoning/summaries/modelProvider`） | ✅ | 快照 |
| E9 | 每条详情消息携带 `sourceLine`（原始 JSONL 行号） | ✅ | 快照 |
| E10 | 分页 `offset = -1` 首屏取末尾 `limit` 条 + 回填真实偏移 + `hasMore` 语义 | ✅ | 单测 |
| E11 | 每 100 条可见消息写一个锚点 | ✅ | 单测 |
| E12 | 缓存失效键 = `mtime + ctime + size` 三者全等 | ✅ | 单测 |
| E13 | 列表解析并发 16 | ⚠️ | 基准 |
| E14 | 本机整文件读取上限 32 MB；WSL 详情超时 ≥60 s / 2 s per MiB / ≤5 min | ✅ | 单测 |
| E15 | Claude：telemetry 推上下文窗口（200K / 1M beta 识别） | ✅ | 快照 |
| E16 | Gemini：`tmp` 与项目映射校验 | ✅ | 快照 |
| E17 | Qoder：一会话关联 `projects`/`tasks`/`file-history`/`logs` 多目录 | ✅ | 集成测试 |
| E18 | 搜索：摘要优先、命中失败才读全文、受限并发 8 | ⚠️ | 单测 |

## F. 会话变更

| # | 行为 | 级别 | 证据 |
| --- | :-: | :-: | --- |
| F1 | Codex 分支走官方 `app-server`：`initialize` → `initialized` → `thread/fork{threadId,lastTurnId}` | ✅ | 集成测试（mock app-server） |
| F2 | `findBranchTurnId` 用**前最近** `event_msg.task_started.turn_id`，不用消息内嵌 turn_id | ✅ | 单测 |
| F3 | fork 结果校验：`thread_id` 非空、≠父、`path` 在 `sessions` 内、≠父文件 | ✅ | 单测 |
| F4 | fork 失败清理：最多 6 次 × 100 ms 重试删除 + 清缓存 + 清元数据 | ✅ | 单测 |
| F5 | **禁止**读写 `thread_history_*.sqlite`、调用 `migrate-rollouts`、创建受管 `CODEX_HOME`、手工截断 JSONL | ✅ | 静态检查 + 评审 |
| F6 | 分支元数据：`parentTargetId/parentSessionId/parentMessageLine/createdBy` | ✅ | 单测 |
| F7 | 复制：重写 `session_meta` 的 `id`/`session_id`/`timestamp`，生成 `rollout-<stamp>-<uuid>.jsonl` | ✅ | 快照 |
| F8 | 删除 → `.visual-console-trash` 保持相对路径；恢复时同名冲突拒绝 | ✅ | 单测 |
| F9 | 目录边界断言：拒绝 `..` 逃逸、目录前缀混淆、根目录本身 | ✅ | proptest |
| F10 | 会话 ID 校验：文件头部必须含 `sessionId` | ✅ | 单测 |
| F11 | `filePath` 快路径使用前必须校验在期望目录内且 ID 匹配 | ✅ | 单测 |
| F12 | 批量删除/清除返回 `processed[]`，含 `movedTo`/`deleted` | ✅ | 单测 |
| F13 | 清除后删除会话元数据记录 | ✅ | 单测 |
| F14 | Qoder 分支/复制显式抛错 | ✅ | 契约测试 |
| F15 | 收藏 / 最近打开：轻量元数据、同记录合并更新、互不覆盖 | ✅ | 单测 |
| F16 | 快速访问：上限 12 收藏 / 8 最近；收藏优先并从最近中排除；仅基于已缓存目标与会话构造 | ✅ | 单测 |
| F17 | 分支子会话查询：优先走元数据索引，缺失才全量扫描 | ⚠️ | 单测 |

## G. Gateway

| # | 行为 | 级别 | 证据 |
| --- | :-: | :-: | --- |
| G1 | 路由 `/gateway/<provider>/<routeId>`，`routeId` 为第三段 O(1) 命中；前缀不匹配返回 404 | ✅ | 集成测试 |
| G2 | 监听 `127.0.0.1:<configuredPort>`；被占用回退随机并记 `fallbackFromPort`；配置 0 时冲突为真异常 | ✅ | 集成测试 |
| G3 | 每 route 独立 32 字节 hex 本地令牌；接受 `Bearer` / `x-api-key` / `x-goog-api-key`；常量时间比较 | ✅ | 安全测试 |
| G4 | 未授权 401、路由不存在 404、供应商不可用 503、超时 504、其他 502 | ✅ | 集成测试 |
| G5 | 上游认证：claude→`x-api-key`，gemini→`x-goog-api-key`，其余→`Bearer` | ✅ | 集成测试 |
| G6 | 转发剔除请求头 `host/authorization/x-api-key/x-goog-api-key/content-length/transfer-encoding/connection` | ✅ | 集成测试 |
| G7 | 回传剔除响应头 `content-length/transfer-encoding/connection` | ✅ | 集成测试 |
| G8 | `joinUpstreamUrl`：拒绝非 HTTP(S)、拒绝 base 带 query/hash、`/v1` 去重 | ✅ | 单测 |
| G9 | 供应商地址指向本地 Gateway 时拒绝 | ✅ | 单测 |
| G10 | 候选池按 `providerId` 隔离；资格 = provider 匹配 ∧ enabled ∧ key 非空 ∧ url 非空 ∧ 未熔断 | ✅ | 单测 |
| G11 | `chooseVendor`：首选当前路由供应商，否则按 `sort` 升序（`createdAt` 次键） | ✅ | 单测 |
| G12 | `chooseNextVendor`：在**完整**排序中环形前进，跳过 attempted 与熔断项 | ✅ | 单测 |
| G13 | `maxAttempts = bufferedBody && retryableMethod ? threshold × candidateCount : 1` | ✅ | 单测 |
| G14 | 仅请求体已缓冲（声明 ≤2 MB）才可重试；否则零缓冲透传 | ✅ | 集成测试 |
| G15 | 请求体上限 32 MB，超限 abort | ✅ | 集成测试 |
| G16 | `mode=locked` 只重试指定供应商且**绕过熔断**，绝不故障转移 | ✅ | 集成测试 |
| G17 | 路由切换不影响在途请求（请求开始即快照） | ✅ | 并发测试 |
| G18 | 每次上游尝试重新取快照（已关闭供应商不进入本轮） | ✅ | 并发测试 |
| G19 | HTTP 200 错误体探测：≤64 KB 或首个 `\n\n` 边界；已读字节原样接回 | ✅ | 集成测试 |
| G20 | 结构化载荷（可 JSON 解析）不因正文含限流短语而误判 | ✅ | 集成测试 |
| G21 | 已向客户端写出内容后**绝不重放**；无候选时保留原始上游响应 + `UPSTREAM_RESPONSE_ERROR` | ✅ | 集成测试 |
| G22 | 自定义响应规则：未写出内容前返回配置状态码与 JSON 体 | ✅ | 集成测试 |
| G23 | 内置默认规则：7 条文本模式 + 10 个状态码 | ✅ | 单测 |
| G24 | 自定义规则作用域 `global/provider/vendor`；大小写不敏感包含匹配；模式 ≤200 字符；≤100 条；自定义体 ≤32 KB 且合法 JSON；状态码 400–599 | ✅ | 单测 |
| G25 | 规则内存快照 + 版本失效；DB 不可用时默认规则仍工作 | ✅ | 单测 |
| G26 | 熔断：失败窗口 5 min；阈值默认 3（1–20）；时长默认 60 s（10–86400）；读时转 half-open | ✅ | 单测 |
| G27 | 成功写入节流 5 s；状态变化立即写；DB 不可用降级内存且不重试建表 | ✅ | 单测 |
| G28 | 失败阈值默认 1（1–10） | ✅ | 单测 |
| G29 | 用量捕获前 512 KB，一次性解码；token 合并取逐项 max | ✅ | 单测 |
| G30 | 用量字段族：OpenAI / Anthropic / Gemini 三套别名 + `total_tokens` 回退为 in+out | ✅ | 快照 |
| G31 | 费用 = tokens/1e6 × 单价；缺单价不估算 | ✅ | 单测 |
| G32 | 客户端断开 → `client-aborted` 并 abort 上游；超时 → `timeout` + 504 | ✅ | 集成测试 |
| G33 | 上游超时 10 min | ✅ | 配置断言 |
| G34 | `outcome` 判定：HTTP ≥400 或 `UPSTREAM_RESPONSE_ERROR` → `error`；无响应体也按状态码记录 | ✅ | 集成测试 |
| G35 | 请求日志字段与长度截断（path ≤500、errorMessage ≤500） | ✅ | 单测 |
| G36 | 文本日志 `gateway.log` + 轮转 `gateway.log.1`，单文件 2 MB；环形缓冲 200/100；flush 200 ms | ✅ | 单测 |
| G37 | **绝不记录** API Key / 请求正文 / 响应正文 / 终端输出 | ✅ | 评审 + 日志断言 |
| G38 | 用量报表：按供应商 + 按模型（模型键含 provider）汇总；平均耗时 | ✅ | 单测 |
| G39 | 诊断分页：`outcome IN ('error','timeout')` + provider/vendor/outcome/时间筛选，SQL 层过滤，默认 10 条上限 50 | ✅ | 单测 |
| G40 | 最近异常固定 3 条 | ✅ | 单测 |
| G41 | 对外 API：默认关闭；令牌只存 sha256；只展示一次；`?key=` 从日志与上游剔除 | ✅ | 安全测试 |
| G42 | 常驻网关：`gatewayEnabled` 默认 `false`（显式）但 `!== false` 语义；有活动路由时拒绝停止并返回 `routes-active` | ✅ | 集成测试 |
| G43 | 开启失败回滚开关并返回 `start-failed`；状态快照反映真实监听状态 | ✅ | 集成测试 |
| G44 | 端口变更：运行中且 `routeCount === 0` 立即重启；有路由时保留旧监听；配置 0 不重启 | ✅ | 集成测试 |
| G45 | `gateway:request-recorded` 仅携带元数据且落库成功后发送 | ✅ | 集成测试 |
| G46 | 供应商切换事件含 `reason`（manual / candidate-pool / failure）与 `mode` | ✅ | 集成测试 |
| G47 | 路由未绑定终端时暂存切换原因，绑定后补发 | ✅ | 单测 |

## H. 供应商管理

| # | 行为 | 级别 | 证据 |
| --- | :-: | :-: | --- |
| H1 | CRUD + 唯一索引冲突处理 | ✅ | 单测 |
| H2 | 启用 = 写入用户 CLI 配置文件（`~/.codex`、`~/.claude`、`~/.gemini`） | ✅ | 集成测试 |
| H3 | 路径白名单：只允许 `~/` 或 `$HOME/` 下 `.codex/`、`.gemini/`、`.claude/`；`..` 穿越防御；反斜杠归一 | ✅ | proptest |
| H4 | 写入前备份到 `data/vendor-backups/<vendor>/<ts>/`（本地与 WSL 分目录） | ✅ | 单测 |
| H5 | 内容为空则中止写入 | ✅ | 单测 |
| H6 | 模型查询：15 s 超时 / 2 MB / 最多 500 条 | ✅ | 单测 |
| H7 | 余额查询：`auto/generic/new-api/custom` 模板 + 点号 JSON Path 提取 + 倍率 | ✅ | 快照 |
| H8 | 认证模式 `bearer/x-api-key/x-goog-api-key/api-key/query/none` | ✅ | 单测 |
| H9 | DeepSeek 预设：Anthropic 兼容入口 + 强制切 `claude` 平台；不覆盖用户 Key 与排序；强制重建配置模板 | ✅ | 单测 |
| H10 | 模型查询配置 JSON ≤32 KB；请求头名校验模式 | ✅ | 单测 |

## I. 终端运行时

| # | 行为 | 级别 | 证据 |
| --- | :-: | :-: | --- |
| I1 | PTY `xterm-256color`，默认 100×30，resize 下限 10×4 | ✅ | 单测 |
| I2 | 输出 16 ms 合并后推送 | ⚠️ | 时序测试 |
| I3 | `resumeKey` 单写入者互斥，重复打开同一历史会话抛「该历史会话正在打开，请勿重复操作。」 | ✅ | 单测 |
| I4 | 终端退出：清检测器 → flush → 移除 → 释放 resumeKey → 销毁 route → 发 `terminal:exit` | ✅ | 集成测试 |
| I5 | 应用退出 kill 全部 PTY，等待 ≤3 s | ✅ | 生命周期测试 |
| I6 | 注入 `OPENAI_API_KEY` 时删除 `CODEX_API_KEY` / `CODEX_ACCESS_TOKEN` | ✅ | 单测 |
| I7 | `TERM=xterm-256color` 强制覆盖 | ✅ | 单测 |
| I8 | Codex Windows CLI 解析顺序（where → PATH → APPDATA\npm → LOCALAPPDATA\npm → ProgramFiles → NVM_HOME\*） | ✅ | 单测 |
| I9 | 未找到 Codex CLI 时给出安装/重启提示文案 | ✅ | 单测 |
| I10 | Codex 路由参数 `-c` 6 项（`model_provider` / `name` / `wire_api=responses` / `requires_openai_auth=true` / `env_key=OPENAI_API_KEY` / `base_url`） | ✅ | argv 断言 |
| I11 | 恢复命令：Codex `resume --all <id>`；Claude/Gemini `--resume <id>`；Qoder `--resume <id>` + 可选 `--cwd` | ✅ | argv 断言 |
| I12 | cwd 缺省 `~/.akim` 并 `mkdir -p`；Codex 支持 `-C`（`useCodexCwdFlag`） | ✅ | argv 断言 |
| I13 | 系统终端 9 种 kind；Git Bash 4 个固定路径探测 | ✅ | 单测 |
| I14 | POSIX `'\''` 转义；cmd `^` 转义元字符 | ✅ | proptest |
| I15 | CLI 参数模板解析：单/双引号、反斜杠转义、未闭合抛错 | ✅ | proptest |
| I16 | WSL 命令经 base64 透传；写文件走 stdin；Codex WSL 前置 `unset CODEX_API_KEY CODEX_ACCESS_TOKEN` | ✅ | argv 断言 |
| I17 | 状态检测器：32 KB 缓冲 + 提示符状态转移 + turnId + 同状态去重 | ✅ | 对拍 |
| I18 | `markExit(0)` 不产生「本轮已完成」；非 0 且用户提交过输入 → `error` | ✅ | 单测 |
| I19 | 确认提示识别集合（Codex 选项式 / `y/n` / 中英文授权） | ✅ | 对拍（含修正既存 2 项失败） |
| I20 | 错误识别 5 类模式 | ✅ | 对拍 |
| I21 | 同一轮内后续按键不重置缓冲（除 `awaiting-confirmation`） | ✅ | 单测 |
| I22 | 活动标签门禁：非活动标签不转发字符/粘贴/Ctrl+C | ✅ | UI 测试 |

## J. WSL

| # | 行为 | 级别 | 证据 |
| --- | :-: | :-: | --- |
| J1 | `wsl.exe` 5 个候选路径探测 | ✅ | 单测 |
| J2 | 统一 60 s 兜底超时，超时 SIGKILL | ✅ | 单测 |
| J3 | 输出上限 shell 16 MB / 读文件 32 MB / stderr 1 MB | ✅ | 单测 |
| J4 | 排除 `docker-desktop` / `docker-desktop-data` | ✅ | 单测 |
| J5 | `-l -q` 按 UTF-16 解码 + 去 `\0` + 去 `*` 前缀 | ✅ | 单测 |
| J6 | 宿主网关地址探测顺序（/dev/tcp → ip route → resolv.conf → 127.0.0.1） | ✅ | 单测 |
| J7 | 网关 base URL 按 `(distro, port)` 缓存，端口变化失效 | ✅ | 单测 |
| J8 | 🔼 持久 worker 协议 + 一次性回退 + 降级开关 | 🔼 | 基准测试 |

## K. Skill 与 CLI 环境

| # | 行为 | 级别 | 证据 |
| --- | :-: | :-: | --- |
| K1 | Skill 根 `$CODEX_HOME/skills`；回收站 `<home>/.visual-console-trash/skills/<name>-<ts>` | ✅ | 单测 |
| K2 | 停用 = 追加 `.disabled`；恢复剥后缀 | ✅ | 单测 |
| K3 | 导入需 `SKILL.md`（≤2 MB）+ YAML frontmatter + `name`/`description`；临时目录原子换入 | ✅ | 单测 |
| K4 | 环境探测：node/npm/nvm/CLI 版本；推荐 Node 24；Gemini/Qoder 最低 20 | ✅ | 单测 |
| K5 | 安装命令 `npm install -g …`，超时 15 min | ✅ | 单测 |
| K6 | WSL 侧用 `bash -lc` 探测（nvm/fnm 只在登录 shell 注入 PATH） | ✅ | 单测 |

## L. 渲染层

| # | 行为 | 级别 | 证据 |
| --- | :-: | :-: | --- |
| L1 | 布局：菜单栏 / 侧栏 360px / 终端工作区 / 状态栏；最小 980×640 | ✅ | 视觉回归 |
| L2 | 侧栏一级「工作台 / 会话」；二级「当前会话 / 收藏夹 / 回收站」（回收站固定展示，不可用时禁用 + `title`） | ✅ | UI 测试 |
| L3 | 收藏/最近区用 `max-height` 而非 `flex-basis` 百分比 | ✅ | CSS 断言 |
| L4 | 工作台 2×2 指标卡 + 速率限制窗口 + 今日用量 + 最近异常 3 条 + 诊断弹窗 | ✅ | UI 测试 |
| L5 | 工作台**不轮询**：靠 `gateway:request-recorded`（250 ms 合并）+ 挂载 + 手动刷新 | ✅ | 代码断言 |
| L6 | 当前供应商使用**按标签路由映射**，不以候选池首项推断 | ✅ | UI 测试 |
| L7 | 状态栏：会话/目录/供应商/用量分组；窄窗口隐藏低优先级 | ✅ | 视觉回归 |
| L8 | 终端标签保存创建时完整 `AiTarget` 快照，不从左侧列表反推 | ✅ | UI 测试 |
| L9 | 标签同步只更新内存，不触发探测/缓存读取/文件扫描 | ✅ | 代码断言 |
| L10 | 自管输入：模型选择、附件、`Alt+Enter` 多行、IME 组合态 Enter、括号粘贴合并写入（Qoder 除外） | ✅ | UI 测试 |
| L11 | 焦点只由活动标签切换 / 显式请求 / 输入模式切换触发 | ✅ | UI 测试 |
| L12 | 终端搜索、复制、粘贴、右键菜单（`TerminalTextContextMenu` 唯一实现，仅复制/粘贴） | ✅ | UI 测试 |
| L13 | 会话详情：首屏定位末尾、向上加载保持阅读位置、`sourceLine` 分支入口 | ✅ | UI 测试 |
| L14 | 全局检索：250 ms 防抖、受限并发、筛选/输入变化使过期结果失效、跨目标恢复携带目标快照 | ✅ | 单测 |
| L15 | 供应商表格：列宽拖拽、分页、候选池开关、余额刷新、故障规则独立弹框 | ✅ | UI 测试 |
| L16 | 命令面板 `Ctrl/Cmd+K` 递归复用菜单 action；原生终端输入模式不拦截 | ✅ | UI 测试 |
| L17 | 提醒中心：铃铛展开、点击切标签并标已读、`X` 真正移除、全移除后入口卸载 | ✅ | UI 测试 |
| L18 | 设计 token 12 项一致；`ui-button` 三态；`IconButton` 唯一纯图标；统一 `Dialog` 关闭按钮在标题栏右上 | ✅ | 视觉回归 |
| L19 | 图标统一 Lucide；图标按钮带 `title` + `aria-label` | ✅ | a11y 测试 |
| L20 | 分页组件：10/20/50，默认 10，上限 50，首页/末页/页码窗口/可访问性 | ✅ | 单测 |

---

## 迁移计划（执行顺序）

### 阶段 0：准备（0.5 天）
- 冻结现网行为基线：补测 `pnpm test` 当前真实通过数，记录 2 项既存失败。
- 建立 `fixtures/` 语料库（各 Provider 脱敏 JSONL 样本、settings 样本、既有 `app.db` 样本、上游 mock 响应集）。
- 采集性能基线（启动、内存、体积、会话列表、WSL 文件操作、网关延迟）。

### 阶段 1：Rust 骨架 + 领域层（P0）
`aivc-core` 领域类型/端口/错误（文案快照）+ workspace + CI 门禁 + Tauri 空壳 + specta 管线。

### 阶段 2：配置与存储（P1）
运行根、settings、**全新 SQLite schema（`refinery` V1 迁移）+ 仓储**、writer task + 读池。**验收：新库 `data/aivc.db` 创建成功且 schema 断言通过。**

### 阶段 3：平台层（P2）
本地 fs SessionStore、WSL 持久 worker（含回退）、引号/参数/路径校验（proptest）。

### 阶段 4：Provider（P3）
注册表 + 子 trait → Codex 全链路（含 app-server 分支）→ Claude/Gemini/Qoder → 元数据/收藏/导出。
**验收：解析器 `insta` 快照与 TS 版逐字节一致。**

### 阶段 5：Gateway（P4）
server/route/upstream/probe/failover/health/rules/usage/log/external 十个模块。
**验收：G1–G47 全绿 + 延迟基准达标。**

### 阶段 6：终端（P5）
PTY + 命令构造 + 状态检测器 + 路由绑定。
**验收：I1–I22 全绿；检测器与 TS 版对拍。**

### 阶段 7：Skill / CLI 环境（P6）

### 阶段 8：IPC + 前端桥接（P7）
81 个 command + specta bindings + `window.codexConsole` 适配层。
**验收：现有渲染层零改动跑通全部功能（L1–L20 视觉与交互回归）。**

### 阶段 9：对拍与打包（P8）
`xtask diff-test` 全量对拍 → 性能基准对比 → 三平台打包。
**验收：Parity 矩阵 A–L 全绿；性能目标达成。**

### 阶段 10（可选）：前端重构 / 原生 UI（P9）

---

## 风险登记

| 风险 | 影响 | 缓解 |
| --- | --- | --- |
| Codex `app-server` 协议随 CLI 版本变化 | 分支功能失效 | 协议版本探测 + 明确错误文案 + 不介入 CLI 私有存储（保持现状策略） |
| Tauri WebView 在 Windows/Linux/macOS 渲染差异 | 视觉/交互回归 | 三平台视觉回归；关键 CSS 避开 WebView 差异特性 |
| WSL 持久 worker 稳定性 | 文件操作失败 | 一次性回退 + 保活重连 + 降级开关 |
| 分页索引字节偏移 | 与旧库无关联，无一致性问题 | 新库直接以 `byte_offset` 为唯一锚点；首次运行时后台重建索引 |
| 自定义标题/收藏/分支关系不随旧库带入 | 用户可见的元数据丢失 | 已在方案确认中接受；可选 `xtask import-metadata` 一次性导入（非必需） |
| 终端状态检测对拍不一致 | 提醒误报/漏报 | 以 TS 版为 oracle 全量对拍；既存 2 项失败一并修正 |
| 渲染层适配层遗漏成员 | 功能缺失 | `satisfies CodexConsoleApi` 编译期约束 + 契约测试 |
| 性能目标未达成 | 价值主张受损 | 每阶段附基准；P8 前不允许回归 |
