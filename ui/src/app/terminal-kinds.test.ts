import { describe, expect, it } from "vitest";

import {
  defaultTerminalKind,
  ensureSelectableKind,
  terminalKindLabel,
  terminalKindOptions,
  type TerminalKind
} from "./terminal-kinds";

/**
 * 系统终端的类型。
 *
 * # 这一组是为什么来的
 *
 * `openSystemTerminal(target, cwd, kind?)` **一直收这个参数**，但界面从来不传——
 * 于是用户只能拿到平台默认的那个 shell。旧平台有一个类型下拉
 * （默认 / PowerShell / CMD / Git Bash / 默认 WSL，WSL 目标再加「当前 WSL」）。
 *
 * # 取值必须与后端逐字一致
 *
 * 后端 `ShellKind::parse` **未知取值直接报错、不静默回退**。因此这里逐个钉住
 * 那九个字符串——错一个字母的后果是「选了 PowerShell，点下去报未知类型」。
 */

const WINDOWS = { isWindows: true, isWslTarget: false };
const LINUX = { isWindows: false, isWslTarget: false };

describe("terminalKindOptions", () => {
  it("第一项永远是「默认」", () => {
    expect(terminalKindOptions(WINDOWS)[0]).toEqual({ kind: "auto", label: "默认" });
    expect(terminalKindOptions(LINUX)[0]).toEqual({ kind: "auto", label: "默认" });
  });

  it("Windows 上列出 PowerShell / CMD / Git Bash / 默认 WSL", () => {
    expect(terminalKindOptions(WINDOWS).map((item) => item.label)).toEqual([
      "默认",
      "PowerShell",
      "CMD",
      "Git Bash",
      "默认 WSL"
    ]);
  });

  it("非 Windows 上列出 bash / zsh / fish", () => {
    expect(terminalKindOptions(LINUX).map((item) => item.label)).toEqual([
      "默认",
      "bash",
      "zsh",
      "fish"
    ]);
  });

  it("**「当前 WSL」只在 WSL 目标上出现**——本机目标给不出那个发行版", () => {
    expect(terminalKindOptions(WINDOWS).some((item) => item.kind === "wsl")).toBe(false);
    expect(
      terminalKindOptions({ isWindows: true, isWslTarget: true }).map((item) => item.kind)
    ).toContain("wsl");
  });

  it("取值与后端的九个字符串逐字一致", () => {
    const all = new Set([
      ...terminalKindOptions({ isWindows: true, isWslTarget: true }).map((item) => item.kind),
      ...terminalKindOptions(LINUX).map((item) => item.kind)
    ]);
    // 后端 `ShellKind::parse` 认的就是这九个。
    for (const kind of [
      "auto",
      "powershell",
      "cmd",
      "git-bash",
      "wsl",
      "wsl-default",
      "bash",
      "zsh",
      "fish"
    ]) {
      expect(all.has(kind as TerminalKind), kind).toBe(true);
    }
    expect(all.size).toBe(9);
  });
});

describe("defaultTerminalKind", () => {
  it("WSL 目标默认开那个发行版的 shell", () => {
    expect(defaultTerminalKind(true)).toBe("wsl");
  });

  it("其余交给后端按平台决定", () => {
    expect(defaultTerminalKind(false)).toBe("auto");
  });
});

describe("ensureSelectableKind", () => {
  it("可选的保持不变", () => {
    expect(ensureSelectableKind("powershell", terminalKindOptions(WINDOWS))).toBe("powershell");
  });

  it("**不可选的退回默认**——不要带着一个后端会拒绝的值去点新建", () => {
    // 从 WSL 目标切到本机之后，「当前 WSL」就不在选项里了。
    expect(ensureSelectableKind("wsl", terminalKindOptions(WINDOWS))).toBe("auto");
    // 在 Linux 上存着 powershell 也一样。
    expect(ensureSelectableKind("powershell", terminalKindOptions(LINUX))).toBe("auto");
  });
});

describe("terminalKindLabel", () => {
  it("九个取值都有中文显示名", () => {
    for (const kind of [
      "auto",
      "powershell",
      "cmd",
      "git-bash",
      "wsl",
      "wsl-default",
      "bash",
      "zsh",
      "fish"
    ] as TerminalKind[]) {
      expect(terminalKindLabel(kind).length).toBeGreaterThan(0);
    }
  });

  it("「当前 WSL」与「默认 WSL」是两个不同的显示名", () => {
    expect(terminalKindLabel("wsl")).not.toBe(terminalKindLabel("wsl-default"));
  });
});
