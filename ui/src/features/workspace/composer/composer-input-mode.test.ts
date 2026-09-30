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
  it("每个取值都有悬停说明（`satisfies` 映射：多一个 / 少一个都过不了 typecheck）", () => {
    /*
      原来的写法是 `const modes: TerminalInputMode[] = ["composer", "terminal"]`
      再断言它长度为 2——那是**自建字面量的同义反复**：删掉一个取值它照样绿。
      改成 `Record<TerminalInputMode, string>` 的 `satisfies` 映射：`TerminalInputMode`
      新增一个成员时这里少键会报错，多写一个不存在的键也会被多余属性检查拦下。
    */
    const titles = {
      composer: terminalInputButtonTitle("composer"),
      terminal: terminalInputButtonTitle("terminal")
    } satisfies Record<TerminalInputMode, string>;

    expect(Object.keys(titles).sort()).toEqual(["composer", "terminal"]);
  });
});
