# aivc-console-ui（界面层）

只与本工程的 Rust 后端通过单一 IPC 接口 `aivc_invoke` 通信的 React 前端。

## 命令

在 `ui/` 下：

- `npm run dev` —— Vite 开发服务器（端口 5273；与旧工程的 5173 错开，避免同时开两个 dev server 时打架）。
- `npm run build` —— `tsc --noEmit && vite build`，产物落 `ui/dist`。
- `npm test` —— Vitest（`jsdom` + `node` 两种环境）。未开 `globals`，RTL 的自动 cleanup 不生效：跨用例渲染同名元素时，查询请用 `within(view.container)` 限定范围。
- 仓库根的 `scripts/verify.ps1 -Quick` 含前端守卫（探针选择器类名、IPC 命令名）。

## 看界面之前先重建二进制

Tauri 在**编译期**把 `ui/dist` 嵌进 `aivc-app` 二进制。只跑 `vite build` 的话，`scripts/ui-check.ps1` / `ui-baseline.ps1` 拍到的仍是旧界面；两个脚本带防呆检查，会要求先 `cargo build -p aivc-app`。

## 测试命名规则

全套件用**中文**写 `describe` / `it`，名字即规格。约定（按既有套件的形制归纳）：

- `describe` 说**被测单元或主题**（组件名、纯函数名，或「什么场景」）；
- `it` 一句话说清**会错成什么样**——宁可长一点，也不要「works / 正常」这种没说清任何边界的名字；
- 断言里最要紧的那个词用 `**加粗**` 标出来（如「**点勾选框不会打开会话**」「**缺键 = 不支持**」）；
- 失败路径的用例名要落到**具体的可观察行为**上（「报出去」「不清空表单」「不发请求」），而不是笼统的「处理异常」；
- 涉及旧平台/裁定背景的用例，在断言旁用注释写明**为什么**（为什么是这个行为、错了会怎样）。

## hooks 依赖数组的正确性靠 review

本工程**不装 ESLint**（本轮整改的硬约束是前端零新增依赖；eslint + typescript-eslint + react-hooks 三个 devDependency 与随之而来的全仓豁免面都超出约束范围）。因此：

- `useEffect` / `useCallback` / `useMemo` 的依赖数组没有静态规则兜底，正确性由 **code review** 保证；
- 「有意省略 / 有意多加」依赖的地方——`terminal-pane.tsx` 的两条信号 effect（不认 `active`）、`terminal-search-bar.tsx` 的 `useMemo`（`revision` 强制重算；`readLines` 由调用方空依赖 `useCallback` 恒稳定）与跳转 effect（不认 `onReveal`）、`workbench-sidebar.tsx` 的 `loadKey`（取数键而非用到的值）——理由都写在紧邻注释里，评审时逐条对照代码即可；
- 将来若引入 ESLint，请先开 `react-hooks/exhaustive-deps` 跑一遍，再把上面这些注释与规则输出逐条对齐，而不是机械地按规则改代码。
