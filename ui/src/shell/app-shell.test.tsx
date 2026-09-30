// @vitest-environment jsdom
import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { AppShell } from "./app-shell";

/**
 * `AppShell` 的骨架契约。
 *
 * # 为什么这五条嵌套值得单独钉住
 *
 * 五区（标题栏 / 活动栏 / 侧栏 / 主区 / 状态栏）的行列关系**只由 `app-shell.tsx` 的
 * DOM 嵌套 + `app.css` 的网格决定**：`.app-body` 是中间那一行（活动栏 + 侧栏 + 主区
 * 三列），`.app-main` 是其中最后一列。槽位挂错一层（把状态栏放进 `.app-body`、
 * 把主区直接塞进 `.app-shell`）在类型上完全合法，**只有渲染出来才看得见**。
 *
 * 这里不渲染整棵 App：骨架不持有任何状态（见它的文档），单测它就是最小的那条断言。
 */
describe("AppShell 的五区骨架", () => {
  it("五个槽位各自落在对应的容器里", () => {
    const view = render(
      <AppShell
        titleBar={<span data-testid="title" />}
        activityBar={<span data-testid="activity" />}
        sidebar={<span data-testid="sidebar" />}
        main={<span data-testid="main" />}
        statusBar={<span data-testid="status" />}
      />
    );

    const shell = view.container.querySelector<HTMLElement>(".app-shell")!;
    expect(shell).toBeTruthy();

    // 外壳是「标题栏 / 内容行 / 状态栏」三行——顺序也是契约，状态栏掉到内容行上面
    // 就是一条贴在中间的横条。
    const rows = [...shell.children];
    expect(rows).toHaveLength(3);
    expect(rows[0]!.getAttribute("data-testid")).toBe("title");
    expect(rows[1]!.className).toBe("app-body");
    expect(rows[2]!.getAttribute("data-testid")).toBe("status");

    // 内容行里依次是活动栏 / 侧栏 / 主区；主区被 `.app-main` 包着（高度链的那一环）。
    const body = rows[1] as HTMLElement;
    const columns = [...body.children];
    expect(columns).toHaveLength(3);
    expect(columns[0]!.getAttribute("data-testid")).toBe("activity");
    expect(columns[1]!.getAttribute("data-testid")).toBe("sidebar");
    expect(columns[2]!.className).toBe("app-main");
    expect(columns[2]!.querySelector("[data-testid='main']")).toBeTruthy();
  });
});
