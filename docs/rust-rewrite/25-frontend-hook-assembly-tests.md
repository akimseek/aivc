# 25 · hook 的装配测试（以及为什么现在才装 jsdom）

承接 [24](24-frontend-output-router.md) 第 7 节。

前两轮修的两处缺陷位于**装配**层，而它们当时**只有端到端截图作为证据**：

- `terminal:status` 事件的 `terminalId` 曾经是空串 → 提醒被写进 `""` 这个不存在的键
  → 关注条永远凑不出「在等你的终端」；
- `terminal:exit` 曾经从不自然触发 → 标签永远显示运行中。

装配断掉时，**类型、纯逻辑测试、门禁全都是绿的**。所以这一层必须有测试。

## 1. 引入了什么

```
npm install --save-dev jsdom @testing-library/react @testing-library/dom \
  --legacy-peer-deps --cache .npm-cache
```

`@vitest-environment jsdom` 写在**单个测试文件顶部**，不改全局环境——其余的纯逻辑测试
继续跑在 node 环境里，不必为它们付 jsdom 的启动成本。

## 2. 不去 mock `../ipc/commands`

`listen` 的全部工作就是读 `window.__TAURI__.event.listen`，把 `message.payload`
交给回调。所以测试里装一个**假的 `window.__TAURI__`** 就够了：

```ts
window.__TAURI__ = {
  core: { invoke: async (command, args) => { invocations.push({ command, args }); return {}; } },
  event: { listen: async (channel, handler) => { /* 记下来，返回取消函数 */ } }
};
```

好处是**真实的 `listen` 包装也一起被测到**；mock 掉模块反而会把这段代码排除在外。
副作用是还能顺带断言「后端调用长什么样」和「卸载时有没有取消订阅」。

## 3. 九条测试

| 测试 | 挡住什么 |
| --- | --- |
| 订阅了输出、退出、状态三条通道 | 少挂一条就是静默失效：不报错、也没反应 |
| `terminal:exit` 让标签变成未运行 | **本轮之前永远不会发生** |
| `terminal:exit` 只影响那一路终端 | 关一个终端把另一个也标成退出了 |
| `terminal:status` 带提醒时按 `terminalId` 记下来 | 空 `terminalId` → 关注条永远不出现 |
| `terminal:status` 不带提醒时清掉提醒 | 标签上的小点一直亮着，用户以为还有事 |
| 接收端挂上前的输出，挂上时按顺序补发 | 终端永远空着，直到用户敲一下键盘 |
| 接收端挂上后的输出直接送达 | 补发路径把直送路径挤掉了 |
| 终端启动失败时标记为未运行 | 用户只看到「点了没反应」 |
| 关闭终端后不再收到它的输出 | 关掉的终端还在后台收数据 |

「按 `terminalId` 记下来」那条同时断言了 `attention[""]` 是 `undefined`——
**直接把曾经的故障形态写成断言**，而不只是断言「正确的情况成立」。

## 4. 假绿检查

把两处装配改回故障形态（退出不置 `running: false`；提醒写进 `""`），
9 条里**红了 4 条**，且两处各自带走一对：

```
× terminal:exit 让标签变成未运行
× terminal:exit 只影响那一路终端
× terminal:status 带提醒时按 terminalId 记下来
× terminal:status 不带提醒时清掉已有的提醒
```

## 5. 门禁

| 项 | 结果 |
| --- | --- |
| 前端 `tsc --noEmit` | 通过 |
| 前端 `vitest run` | **45 通过**（原 36，+9） |
| 前端 `vite build` | 通过（产物哈希与上一轮一致，因此无需重跑端到端） |
| 旧平台 `src/` 改动 | 0 |

> **踩坑记录**：jsdom 环境**首次**启动要 39 秒（冷启动），之后就降到 0.8 秒。
> 看到 `environment 39393ms` 不要以为测试卡死了。

## 6. 至此，前端这一层的证据链

| 层 | 证据 |
| --- | --- |
| 纯逻辑（缓冲/分发/标题） | 单元测试，**零新依赖** |
| 事件装配（hook） | jsdom + testing-library 单元测试 |
| 真实应用 | CDP 离屏截图 + 键盘注入（`scripts/ui-check.ps1`） |

还剩下必须靠人的一层：**操作系统原生窗口**（导出「另存到…」、技能导入的文件选择框）。
CDP 看不到它们，这是工具链的已知边界。
