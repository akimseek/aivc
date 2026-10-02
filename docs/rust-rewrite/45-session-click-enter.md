# 45 · 行为修正：单击会话行 = 直接进入终端

> 2026-10-02。修正 39 号（批次一）§三.1 的行为口径：单击会话行从「打开标签看历史、
> 恢复另行显式」改为 **「直接进入该会话的终端」**；详情走卡片/标签栏的
> 「消息与分支」。用户裁定。
> spec（`docs/superpowers/specs/2026-09-30-aivc-rs-ui-redesign-design.md`）§9.1
> 行为表的「点击会话行」一行已同步改写。

## 一、口径

| 点击时的状态 | 落点 |
| --- | --- |
| 该会话的终端**在跑** | 切到它的标签（不重启进程） |
| 没在跑（没开过） | `shell:path-exists` 探原目录（带目标）→ 在就带 `--resume` 起一路 |
| 没在跑（异常退出的残场） | 同上——新终端挂到同一标签，残场被 `attachTerminalTab` 回收 |
| 原目录不在 | 弹「设置工作目录」（`继续打开`），与旧平台一致 |
| 恢复**在途**时的连点/双击第二次点击 | 空操作（`resumingRef` 守卫），不产生第二路进程 |
| 详情（消息 + 分支） | 卡片上的「消息与分支」按钮 / 标签栏同名按钮——**不再途经画布历史正文** |

双击不再有独立绑定：单击已承载该动作，双击 = 两次单击（第二次被守卫收敛）。

## 二、实现（aivc-rs 本次提交）

- `app.tsx`：`openSessionById` 改为「在跑 → 激活；否则 `beginResumeSession`」；
  `beginResumeSession` 增 `resumingRef` 防重入（在途重复触发只当没发生）。
- `session-card.tsx` / `sessions-sidebar.tsx`：`onResume` 契约整体下线；随双击绑定
  一起删除的还有三个动作按钮上为防 `dblclick` 而加的 `stopPropagation`（那层保护
  只因卡片有双击处理器才有意义）。
- 历史视图**保留**：恢复在途与「设置工作目录」对话框背后仍用它；它的
  「继续会话」按钮继续可用（对正在显示历史的场景是一条合法入口）。
- 探针同步：`exit-reentry` / `resume-e2e` / `startup-and-detail` 的「双击恢复」
  改「单击」；新增 `probes/session-click-enter.js` 做端到端复验。

## 三、证据

- **App 级回归** `src/app/app-session-click.test.tsx` 3 条（先写红——`terminal:start`
  与 `shell:path-exists` 一次都没发生；实现后转绿）：单击起 resume 链、终端在跑不
  重启、在途连点被守卫挡住。**守卫判别力实测**：临时禁用守卫后，连点用例变红
  （探测发了 2 次）。
- **真机探针** `scripts/probes/session-click-enter.js`：PASS——单击后
  `stage=terminal`、14 条 `terminal:data`、未途经历史正文；截图
  `shots/probe-session-click-enter.png` 里是真实的 `codex resume` 交互现场
  （该会话当时停在自己的更新提示上）。
- **全量**：88 文件 / 876 用例全绿（+3）；tsc / build / `verify.ps1 -Quick` 八段全绿。
- **安装包重打**（按 44 号配方，2026-10-02 13:01）：nsis 5.55 MiB
  （SHA256 `b5abfb50…e1a20`）/ msi 7.96 MiB（SHA256 `cfa4e8fa…b6f93`）；
  release 编译 + 双 bundle 一趟跑通。首轮 P8 产物见 44 号 §一（哈希为那一轮）。
- 既有测试同步：`app-resume.test.tsx`（P0-C 的 pathExists 调用链）触发方式
  双击→单击，断言未变；`session-card.test.tsx` 的「双击恢复」用例改为
  「`dblclick` 不再有独立动作」。

## 四、附带发现（未改，记录备查）

- 「新建会话」的终端标签在拿到会话编号前没有 `sessionId`（不做 re-key，批次一
  已记录），点它列表里的行会另起一路 `--resume`——**与旧平台同形，已知并接受**。
- 复验现场看到一条会话 resume 报
  `Model provider 'akim_gateway' not found`（该会话创建时的供应商配置在当前配置里
  不存在）——与会话资产本身无关，属该会话的创建环境。
