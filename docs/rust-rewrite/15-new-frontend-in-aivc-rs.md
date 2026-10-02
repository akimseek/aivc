# 15 · 新前端落地：aivc-rs 拥有了自己的界面

> 决策路径：先是「完全重构，不能用旧平台的东西」→ 选「在 aivc-rs 里从零重写前端」→
> 设计探索（4 套 + 3 套方案）被否 → 回到「**按旧平台的风格实现**」，方式仍是
> **在 aivc-rs 里重新实现**（不复制旧源码）。
>
> 本文记录这一轮的实际落地。

---

## 1. 先解决了依赖装不上（真实阻塞）

`npm install` 报 `Cannot read properties of null (reading 'edgesOut')`。逐步查下来的结论：

| 现象 | 真正原因 | 处理 |
| --- | --- | --- |
| `edgesOut` 崩溃 | **npm 10.9.3 在解析 `vitest` 的 peer 集时崩**（栈停在 `#loadPeerSet`，计时器停在 `idealTree:node_modules/vitest`） | `--legacy-peer-deps` |
| 随后 `EPERM`（exit -4048） | 卡在 **`esbuild` 的 postinstall**——沙箱禁止其管道 stdio | 放行一次 |
| 一度怀疑缓存 | **不是**：把缓存指到工作区内仍然崩 | 排除 |

顺带发现环境里的一个坑：`node` 是 **v24.9.0**，而 npm 实际跑在 **v22.20.0** 上（这台机器有
多套 node）。排查时容易看错版本。

最终装成 95 个包：react 19.3.0 / vite 7.3.6 / typescript 5.9.3 / vitest 4.1.11。

## 2. 新前端：`aivc-rs/ui/`

```text
ui/
├── package.json  tsconfig.json  vite.config.ts  index.html
├── src/
│   ├── main.tsx
│   ├── types.ts              领域类型（只覆盖主流程）
│   ├── ipc/
│   │   ├── client.ts         与后端通信的**唯一入口**
│   │   └── commands.ts       命令面：每个函数就是一行 invoke
│   └── app/
│       ├── app.tsx           外壳（只负责画）
│       ├── use-console.ts    取数顺序（缓存先行 → 探测刷新 → 自动选中）
│       └── app.css           设计令牌 + 区域样式
└── design/                   三套布局方案（保留作参考，本轮未采用）
```

### IPC：单一入口

后端只注册了一个 Tauri 命令 `aivc_invoke`，收 `{ command, args }`。前端因此只需要维护
**命令名**这一层，收敛在 `ipc/commands.ts` 里——别处不出现命令字符串。

（顺带记下：`tauri_app.rs` 里 `aivc_invoke` 的文档注释写的是 `invoke("aivc:invoke", …)`，
而**实际注册名是 `aivc_invoke`**。按注释写会得到一个「命令不存在」的运行时错误。）

### 视觉基线取自旧平台，结构是新的

取的是**设计语言**，不是样式代码：

| 令牌 | 值 |
| --- | --- |
| 强调色 | `#168c78`（青绿）——整套界面只有这一个彩色 |
| 背景 / 面板 | `#f4f6f8` / `#ffffff` |
| 边框 | `#dfe5eb`，分隔线 `#edf0f4` |
| 文字 / 次要 | `#17212b` / `#71808e` |
| 终端底 | `#0f172a` |
| 布局 | 菜单栏 32px →〔侧栏 **360px** + 工作区〕→ 状态栏 |
| 字体 | Inter / system-ui |

结构上的差别：令牌集中在 `:root`，其余按区域分段——不是旧工程那种一个八千行的 CSS 文件。

## 3. 跑通了主流程（日志为证）

真实应用启动后，后端收到的命令序列**正是设计意图**：

```text
ai:list-providers
gateway:get-port
performance:log
codex:list-cached-targets     ← 缓存先行：立刻出内容
codex:list-targets            ← 再探测刷新
codex:list-cached-sessions    ← 同样缓存先行
codex:list-sessions
codex:get-session-messages-page
```

顺序不能反：探测要碰文件系统与 WSL（冷启动 5–7 秒），先等它首屏就空着。

## 4. 删掉了 `bridge.rs`：命令面的**第二个真相来源**

`bridge.rs` 是「前端零改动复用」时代的适配层——一张 **83 项的手写方法表**，把
`window.codexConsole.listCachedSessions` 映射到 `codex:list-cached-sessions`，再用
`initialization_script` 注入。

新前端直接用 `window.__TAURI__.core.invoke`，不再需要它。留着就是**同一份命令面维护两遍**
（一张 Rust 表 + 一个前端字符串），删掉是对的。

删除后七项门禁全通过，真实应用照常跑通。

## 5. 与旧工程彻底解耦

| | 之前 | 现在 |
| --- | --- | --- |
| `frontendDist` | `../../../dist/renderer`（旧工程产物） | **`../../ui/dist`** |
| 前端源码 | `D:\code\ai\src\renderer` | **`aivc-rs/ui/src`** |
| 命令面适配 | `bridge.rs`（83 项表） | 直接 invoke + `ipc/commands.ts` |

**旧工程的 `src/renderer` 一行未动。** 现在删掉 `D:\code\ai\src\renderer` 也不影响新应用。

产物大小：**231.5 KB**（旧产物 436 KB）。

## 6. 仍未做（按优先级）

这一轮只做了**主流程切片**：平台 → 目标 → 会话 → 消息。旧平台还有大量功能未搬：

| 功能 | 状态 |
| --- | --- |
| 消息分页 / 加载更多 | **未做**（只取了最后一页） |
| 搜索（当前目标 / 跨平台） | 未做 |
| 收藏 / 回收站操作 / 批量操作 | 未做 |
| 分支 / 复制 / 导出 / 重命名 | 未做 |
| 内嵌终端（xterm）+ 系统终端 + 标签 | 未做 |
| 供应商管理 / 网关 / 技能 / 工作台 / 设置 | 未做 |
| 命令面板 ⌘K | 未做（界面上写着「待接入」） |
| 平台状态 / 诊断面板 | 未做 |
| 事件推送（终端输出等） | 未做 |
| 前端测试 | 未做（vitest 已装好） |
