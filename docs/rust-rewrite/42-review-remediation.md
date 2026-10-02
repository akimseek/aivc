# 42 · 代码审查整改记录（P0 真实缺陷 / P1 一致性 / P2 规范与收尾）

> 日期：2026-10-02 · 提交在 `aivc-rs` 的 `main`（起点 `a8aaea4`，终点头 `2eccb4b`，**36 个提交**）
> 计划：[`../superpowers/plans/2026-10-01-aivc-rs-review-remediation.md`](../superpowers/plans/2026-10-01-aivc-rs-review-remediation.md)
> 前序：[`39-ui-redesign-batch1.md`](39-ui-redesign-batch1.md)、[`40-ui-redesign-batch2.md`](40-ui-redesign-batch2.md)、[`41-ui-redesign-batch3.md`](41-ui-redesign-batch3.md)

## 一、范围与结果

对界面重做（批次一~三）后的**企业级代码审查**（结构/命名、可扩展性、兼容性、性能四维度）所开出的问题，分三档整改：

- **P0 · 真实缺陷（6 项，全部完成）**：A 能力门控 / B 会话详情竞态 / C WSL 恢复路径（**根因在 Rust**，已改 `crates/`）/ D 启动与终端兜底 / E 网关页限流 / F 刷新合并器。
- **P1 · 一致性与成本（12 项，全部完成）**：A 命令名机械守卫 / B 平台清单单点 / C 设置面板整族搬迁 / D 三族组件归位 / E 公共组件抽取（Pager/ContextMenu/Badge 收敛/IconButton）/ F 死代码与 lint 伪影 / G capabilities 收窄 / H 拖放提示改 Tauri 事件 / I 会话卡渲染成本（memo + 回调链）/ J 历史视图上限与折叠 / K composer 渲染期拷贝 / L 终端输出通道（**仅评估**）。
- **P2 · 规范与收尾（6 项，全部完成）**：A 构建卫生 / B emoji→Icon / C errorText 归一 / D sidebar-nav 与 gateway-range 收口 + presets 测试 + 测试命名成文 / E 注释格式清单 / F 本记录 + spec §4.2 口径修订。

**门禁与数字**：`tsc --noEmit` 零错误；`npm test` **83 文件 · 861 用例全绿**（整改起点 79/830）；`vite build` 成功；`scripts/verify.ps1` 八段全绿（含探针选择器守卫与**新增的 IPC 命令名守卫**，前端 93 调用点 vs 后端 97 条表、失败级差异 0）。**零新增依赖**；除 P0-C 根因与 P1-G 权限外未动 `crates/**`。

## 二、提交（`aivc-rs` 仓库，按任务分组）

### P0（真实缺陷）

| 提交 | 内容 |
| --- | --- |
| `9abc0cf` `10c936a` | P0-A 能力门控：四处入口按平台能力放行（含侧栏 WSL 目录表单的同类残留）+ 修复轮 |
| `51df77c` | P0-B 会话详情取数按请求号丢弃旧响应（loading/error 归最新一次请求） |
| `145031a` `71b6948` `00a84c1` | P0-C 恢复目录探测带上目标（WSL 按「还在」处理）+ **Rust 根因**（Codex WSL 编号对齐 `codex:wsl:<distro>`）+ 假运行时忠实复刻 |
| `185c704` `9672b60` `da3b6a1` | P0-D ErrorBoundary 两处挂载 + 主题读存储兜底 + 重试分流（chunk/CSS 预载失败 → 整页 reload）两轮 |
| `62a2171` | P0-E 「最近请求」分页首页 50 + 自动刷新 1Hz + seq 防旧覆盖 |
| `68cc830` | P0-F 工作台刷新改走统一 refresh-coalescer（窗口常量归口） |

### P1（一致性与成本）

| 提交 | 内容 |
| --- | --- |
| `4a54c18` | P1-A `check-command-names.cjs`（注入式自检）接入 `verify.ps1` |
| `d5a46e5` | P1-B 平台清单从 `ai:list-providers` 派生（修 Claude 显示名漂移） |
| `5863337` | P1-C 设置面板整族（21 文件）迁入 `features/settings/panels/` |
| `cadd894` | P1-D 三族组件（10 源文件 + 9 测试）归位 `features/{workspace,sessions,workbench}/` |
| `fa56d29` `0051b12` `1dfe5a3` `c2ad364` `40a718b` | P1-E 公共组件：Pager ①、ContextMenu 外壳 ②、`.tag` 13 处→Badge（含 small）③、IconButton ④、Pager 头注校准 |
| `27ae3c4` `6fe9483` | P1-F 死代码：Toggle/StatusPill/`.pill*`/`.toggle*`（含最后一处硬编码 `#fff`）/Spinner 出口/5 条失效 disable；`ui/README` 成文「deps 靠 review」 |
| `57d7613` | P1-G capabilities 收窄为 `core:default`（删 6 条未调用插件权限 + 重复 event） |
| `6b2a069` `7b0e3a8` | P1-H 输入区拖放提示改听 `tauri://drag-*` + 命中判定（含 dpr 换算） |
| `fdcbf64` `3c7b627` `e5ce884` | P1-I SessionCard memo + 回调按 id 委托；修复轮收敛 app 链整对象 deps（含 `selectedSessionRef` 旁路） |
| `5d85362` `66b1342` | P1-J 历史正文折叠复用弹框那份 + 消息窗口封顶 400（到顶指路弹框） |
| `2d646b5` | P1-K composer 提交载荷 useMemo |
| （无代码） | P1-L 终端输出通道评估（见 `p1l-report`；结论：①Rust 侧合批 → ②base64（零新增依赖）→ ③二进制通道视 profiling） |

### P2（规范与收尾）

| 提交 | 内容 |
| --- | --- |
| `f88724e` | P2-A engines/.nvmrc、生产关 sourcemap、color-mix 回退、等宽字体统一、bundle.icon 补齐 |
| `0e6d0b4` | P2-B emoji/字形全换 `Icon`（提醒中心/输入区/搜索栏） |
| `26a63de` | P2-C `errorText` 归一出（55 处内联 + 两处私有版并入） |
| `ddacf85` | P2-D `sidebar-nav.ts` 摘除、`gateway-range.ts` 拆出 `gateway-format.ts`、`presets-panel.test.tsx` 补 6 条、测试命名规则成文 |
| `2eccb4b` | P2-E 叠放 JSDoc 合并、双空行、中英夹杂、EOF 空行、「批次二再议」回填为已裁定 |

## 三、行为变化（需知情）

1. **P0-C（Rust）**：Codex 的 WSL 目标编号形制定为 `codex:wsl:<distro>`，旧形 `wsl:<distro>` 双向兼容；golden fixture **有意重定基**（三处留痕防回退）。表现：WSL 恢复会话不再误弹「重设目录」。
2. **P0-D**：错误边界的「重试」在 chunk 类错误（含 Vite `Unable to preload CSS for`）下改为整页 reload——懒加载失败无法自愈，死按钮变活。
3. **P0-E**：「最近请求」只取首页 50 条（后端上限即 50），自动刷新限到 1Hz。
4. **P1-E③**：`.tag` 13 处并入 `Badge`；异常诊断页 timeout 由裸文本 → 中性徽章；accent 徽章补 1px 边框。
5. **P1-G**：主窗口权限收窄为 `core:default`——剪贴板/原生对话框/通知三链路走 `aivc_invoke` → 后端插件 API，**实跑复验过**（含 OS 层剪贴板读回）。
6. **P1-J**：历史视图窗口封顶 400 条（到顶不再给「加载更早」，改指路「消息与分支」）；>4000 字正文折叠（与弹框同件，一个字不丢）。
7. **P2-B**：emoji 全换 `Icon`；「▶ 继续会话」保留文本字形（demo 无对应图形，计划未列）。

## 四、裁定（R-*，节选；全量见整改账本）

- **R-6**：P0-C 允许改 `crates/**`——「前端不改后端」的前提（后端行为正确）被证伪，根因修复归该任务。
- **R-7**：golden 凡因根因修复而变，**显式更新并写明「legacy 此处是缺陷，按新契约修正」**，不得静默改。
- **R-8**：测试的假运行时必须**忠实复刻**后端判据（修完先如实转红、再随修复转绿）。
- **P1-E③**：计划写「12 处 `.tag`」实为 13 处；spec 初稿的 `Tag` 组件**不再实现**——平台徽章与状态标记都由 `Badge`（含 `small`）承担（spec §5.4 已随之修订）。
- **P1-I**：`memo` 要真正生效，回调链的 `useCallback` deps 必须避开 `console`/`terminals`/`workspace`/`toast` **整对象**（每次渲染换身份）；`selectedSession` 派生值经 ref 旁路。组件级 + 侧栏级两层渲染计数用例钉住（判别力实测过）。
- **P1-J**：封顶取「到顶即停 + 指路弹框」，**不采用**计划举例的「继续加载再丢最早一页」——前置插入式翻页里它要么空转、要么把刚装进来的内容闪没（评审接受该偏离）。
- **P2-C**：`ipc/client.ts` 为 `errorText` 的**唯一例外**（最底层不得反向依赖 components，且那里是重新包装 `IpcError` 而非展示）。
- **P2-D**：能力测试迁移时按**后端矩阵**校正旧 fixture（旧断言称 qoder 不支持 trash/batch，与 `target.rs` 相反——旧 fixture 才是错的）。

## 五、遗留（转入后续工作）

> **2026-10-02 批注：本清单已清零**（除第 4 条里明确保留的一项）。逐条处置与证据如下。

1. **历史窗口头页重叠**（P1-J 评审发现，**先于该任务存在**）：`0 < messageStart < 80` 时请求 `offset=0` 拿到整页 `[0,80)`，与已有窗口重叠——条数非 80 倍数的会话（如 125 条）往前翻到头会看到**重复消息**。修法：prepend 前裁掉 `messageStart - page.offset` 之前的部分，或按 `sourceLine` 去重。
   → **已修（commit `e436c80`）**：prepend 前按 `messageStart - page.offset` 裁页；判别测试「翻到头（0 < messageStart < 页大小）不出现重复消息」钉住（125 条会话，条数唯一性断言）。
2. **`loadEarlierMessages` 无「会话未变」判别**：翻页途中切会话，旧会话的页会插进新窗口（P0-B 的 seq 模式未覆盖此处）。
   → **已修（commit `e436c80`）**：`messagesSeqRef` 代际——选中会话/目标一变即 +1，在途页回来不等于当前值就整页丢弃（失败也不许盖新窗口的错误位）；挂起页测试钉住。
3. **终端输出通道**（P1-L）：按评估三档推进——① Rust 侧合批（零协议、前端零改）② base64 载荷（crate 已在依赖树、前端 `atob` 内置，**零新增依赖**）③ 真二进制通道（Tauri 事件 JSON-only，需框架外通道 + 三平台验证）；实施需后端协同。
   → **①②已落地（commit `f166c5e`）**：合批=「前沿发送 + 16 ms 窗口 + 64 KiB 上限 + 关闭排空」，base64=`{encoding:"base64", data:string}`（不认识的标签整条丢弃）+ 前端 `decodeTerminalData` + 探针同步；合批对旧实现验证过判别力（RED）。③**不采用**：二进制通道需框架外通道，收益（相对 ①② 已压掉 >2/3 体积与全部逐元素 parse）不足以承担三平台验证成本。
4. **各类 minor（账本 deferred 项摘录）**：概览卡对 50 条上限无可见提示；`context-menu.tsx` 无自有单测；App 链 deps 缺自动防线（引入 `eslint-plugin-react-hooks` 可部分兜住——`ui/README` 有对齐步骤）；`Spinner` 可再收为模块私有；P0-A 侧栏成对测试不判别「用错能力键」；等。
   → **已落地四项（commit `e436c80`）**：概览卡提示带上限数；context-menu 五个单测（含 Escape 让位与动作顺序）；`Spinner` 收回模块私有；侧栏 fixture 改**反配**（键名换位即红）。**保留一项**：不引入 `eslint-plugin-react-hooks`——整改全程的「零新增依赖」约束仍在。
5. **P8 打包前置已就位**：`bundle.icon` 已补 `icon.icns` + 32/128/256 PNG（PNG 由 `icon.ico` 的 PNG 帧按字节切出，非重采样）；`bundle.active` 仍为 `false`（打包动作不在本范畴）。
   → **已清零（commit `6fb138c` + `44-p8-packaging.md`）**：`bundle.active` → `true`；Windows **nsis 5.55 MiB / msi 7.96 MiB 实测产出**（均在 02 §11「≤15 MB」目标内）；dmg/AppImage/deb 与性能基准需对应平台构建机，属 P8 其余部分，见 44 号 §六。

## 六、验证与证据

- **测试**：83 文件 / 861 用例（起点 79/830，净增 31 条，删除 8 条被覆盖的旧侧栏测试）。
- **守卫**：`verify.ps1` 八段（探针选择器类名、IPC 命令名、fmt、layers、check、clippy×2、cargo test 全量）。
- **基线截图**：`shots/b3-p1c-relocate`、`b3-p1d-relocate`（搬迁复验）、`b3-p1e`（E①②③④ 五步）+ `p1e-diagnostics.png`（15 徽章探针）+ `p1g-chains.png`（三链路探针图）。
- **最终视觉验收（2026-10-02，用户确认后）**：`shots/b-final`（基线五步，探针全对）+ `shots/final-walk`（设置十页明暗 + 暗色主界面共 13 张，逐张读图核对：徽章/胶囊/mono 列对齐/暗色对比度均正常；异常诊断与请求日志为空态——干净运行无网关流量，其表格徽章由 `p1e-diagnostics` 探针与单测覆盖）。
- **三链路实跑**：状态栏复制 → `clipboard:read-text` 读回与界面显示**逐字相等** + PowerShell `Get-Clipboard` OS 层独立确认；通知返回 ok；choose-directory 探针挂起（原生框已弹出等待用户）。
- **逐任务报告与评审记录**（含每条的判别力 RED 证据、评审裁定与修复轮回填）：`aivc-rs/.superpowers/sdd/2026-10-01-aivc-rs-review-remediation/`（`p0a-report.md` … `p2 各报告` + `progress.md` 账本）。
