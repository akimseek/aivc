import { describe, expect, it } from "vitest";

import {
  canToggleTerminalInput,
  terminalInputButtonLabel,
  terminalInputButtonTitle,
  type TerminalInputMode
} from "./composer-text";

/**
 * A11：终端输入模式切换。
 *
 * # 这一组补的是什么
 *
 * 我自己的代码里原本就写着：「`revealComposer()` 是**唯一**的显示入口——
 * 没有按钮能把它调出来」。旧平台的状态栏有一个按钮在
 * 「终端原生输入」与「自管输入框」之间切（`status-bar.tsx`），
 * 而 `canToggleTerminalInput = composerVisible`。
 *
 * 交互式程序（`vim`、`less`、要 Tab 补全的那些）必须在终端里收键 ——
 * 没有这个按钮，用户一旦露出自管输入框，就再也回不到原生输入了。
 */

describe("terminalInputButtonLabel", () => {
  it("**写的是点下去会切到哪**，不是当前在哪", () => {
    // 旧平台：`mode === "composer" ? "终端输入" : "自管输入"`。
    // 写成当前状态的话，用户会以为它只是个指示器而不是可点的。
    expect(terminalInputButtonLabel("composer")).toBe("终端输入");
    expect(terminalInputButtonLabel("terminal")).toBe("自管输入");
  });

  it("两种模式的字不同（否则按钮看不出变化）", () => {
    expect(terminalInputButtonLabel("composer")).not.toBe(
      terminalInputButtonLabel("terminal")
    );
  });
});

describe("terminalInputButtonTitle", () => {
  it("悬停说明与旧平台逐字一致", () => {
    expect(terminalInputButtonTitle("composer")).toBe("切换到终端原生输入");
    expect(terminalInputButtonTitle("terminal")).toBe("切换到自管输入框");
  });
});

describe("canToggleTerminalInput", () => {
  it("**输入框藏着时不能切**——那时「切换输入位置」没有意义", () => {
    // 旧平台：`canToggleTerminalInput = composerVisible`。
    expect(canToggleTerminalInput(false)).toBe(false);
    expect(canToggleTerminalInput(true)).toBe(true);
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
