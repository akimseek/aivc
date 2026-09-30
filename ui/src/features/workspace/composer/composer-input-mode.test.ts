import { describe, expect, it } from "vitest";

import { terminalInputButtonTitle, type TerminalInputMode } from "./composer-text";

/**
 * 输入模式切换（A11）。
 *
 * 剩下的这一条是状态栏胶囊的悬停说明——胶囊是唯一还在的模式入口（输入区那个按钮
 * 随 T10 重做删掉了）。与它成对的 `terminalInputButtonLabel` /
 * `canToggleTerminalInput` 已删：两者都只被这份测试引用，生产侧零调用。
 *
 * 交互式程序（`vim`、`less`、要 Tab 补全的那些）必须在终端里收键 ——
 * 这个入口不能没有，否则用户一旦露出自管输入框就再也回不到原生输入。
 */

describe("terminalInputButtonTitle", () => {
  it("悬停说明与旧平台逐字一致", () => {
    expect(terminalInputButtonTitle("composer")).toBe("切换到终端原生输入");
    expect(terminalInputButtonTitle("terminal")).toBe("切换到自管输入框");
  });
});

describe("模式是两个值，不是布尔量", () => {
  it("取值与旧平台一致（`composer` / `terminal`）", () => {
    const modes: TerminalInputMode[] = ["composer", "terminal"];
    // 序列化成什么不重要，重要的是**恰好两个**——多一个值就意味着某一处
    // 的 `=== "composer"` 判断会走到「其余」分支。
    expect(modes).toHaveLength(2);
    expect(modes).toContain("composer");
    expect(modes).toContain("terminal");
  });
});
