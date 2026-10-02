# 46 · 终端区两处界面修正：去掉会话信息条、修「底部被自管输入框遮挡」

> 2026-10-02。用户在安装版实测后提出的两条（升级到「单击会话行 = 进入终端」之后）。
> spec §9.2 终端视图一节已同步改写。

## 一、去掉终端上方的会话信息条

- **口径**：终端视图不再有常驻的「标题 · 短编号 · 目标 · 供应商 · 网关」一行。
  这几项在**状态栏**（供应商/网关）与设置中心里都有，会话标题在标签与侧栏上。
- **实现**：删 `.terminal-info-strip`（JSX + CSS）与只喂它的 props 链
  （`TerminalView.session/target/gateway/vendors` → `WorkspaceStage.infoSession/target/
  gateway/vendors` → `SessionWorkspace` 同名 props → `app.tsx` 的 `tabSession` 派生）。
  `tabSession` / `infoSession` 整条「标签口径的第二会话来源」随之下线——R32 里
  「画布跟选中项走」只剩历史视图一个消费方，更简单了。
- **测试**：`terminal-view.test.tsx` 的 5 条信息条用例删除（依赖的信息源已不存在）；
  `workspace-stage.test.tsx` 的「两个来源各归各的」用例收编为「历史视图吃选中项：
  标题与消息同源」。**净 -5 +1 条**。

## 二、修「进入终端后底部被自管输入框遮挡」——FitAddon 把宿主 padding 算进了可用高度

### 复现（诊断探针 `scripts/probes/terminal-layout.js`，真机数字）

| 量 | 修复前 | 修复后 |
| --- | --- | --- |
| `.xterm` 底 − `.terminal-host` 底（>0 = 被 `overflow: hidden` 裁） | **+6px**（最后一行半截） | **−26px**（完全在内，留白正常） |
| 宿主底 − 输入框顶（>0 = 重叠） | 0 | 0 |
| 行数 × 行高 vs 宿主内容高 | 34×15=510 > 505 | 34×15=510 < 537 |

### 根因

`FitAddon` 量的是 `term.open(el)` 那个元素的**父元素**，且只减 xterm 自身的 padding；
而全站 `box-sizing: border-box` 下 `getComputedStyle(host).height` 返回的是**含 padding
的框高**。挂载点直接带 `padding: 8px 10px 10px`（原 `.terminal-host`）→ 可用高度被多算
18px（横向同理多算 20px），行/列各多出一档，最后一行落在宿主外被裁——看起来就是
「底部被自管输入框挡住」。

### 修法

挂载点分两层：`.terminal-host`（保留 8/10/10 留白，`overflow: hidden`）＞
`.terminal-mount`（**无 padding**，`flex: 1; min-height: 0`，`term.open` 挂它）。
FitAddon 量的父元素从此就是真正的可用区。

### 证据

- 真机探针修复前 `REPRO`（`lastRowOverHost: 6`）→ 修复后 **`OK`**（`-26`；
  截图 `shots/probe-terminal-layout.png`：信息条没了、终端完整、输入框在下方不重叠）。
- 结构回归：`terminal-pane.test.tsx` 新增一条——`term.open` 收到的元素必须是
  `.terminal-mount` 且其父元素是 `.terminal-host`（jsdom 没有排版，真实几何由探针钉）。
- 探针选择器守卫：`.xterm-screen` 进白名单（第三方库 DOM，理由写在守卫文件里）。
- 全量：88 文件 / 872 用例（净 -4）、tsc / build / `verify.ps1 -Quick` 八段全绿。

## 三、顺带记录的现场

- 复验中该会话的 resume 又见一次
  `Model provider 'akim_gateway' not found`——属该条会话创建环境的配置残留
  （45 号 §四已记），与会话资产和本次改动无关。
