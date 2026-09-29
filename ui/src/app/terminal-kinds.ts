/**
 * 系统终端的「类型」。
 *
 * # 取值必须与后端逐字一致
 *
 * 后端 `ShellKind::parse` 认这九个字符串，**不认识就报错、不会静默回退**：
 * `auto` / `powershell` / `cmd` / `git-bash` / `wsl` / `wsl-default` / `bash` / `zsh` / `fish`。
 * 写错一个字母的后果是「用户选了 PowerShell，点下去报未知类型」。
 *
 * # 为什么要有这一层
 *
 * 此前界面**根本不传 kind**——`openSystemTerminal` 收这个参数，但没人给过值，
 * 于是用户只能拿到平台默认的那个 shell。旧平台的下拉是这个形状：
 * 默认 / PowerShell / CMD / Git Bash / 默认 WSL（Windows），
 * 或 bash / zsh / fish（其他），WSL 目标再加「当前 WSL」。
 */

export type TerminalKind =
  | "auto"
  | "powershell"
  | "cmd"
  | "git-bash"
  | "wsl"
  | "wsl-default"
  | "bash"
  | "zsh"
  | "fish";

export type TerminalKindOption = { kind: TerminalKind; label: string };

/**
 * 当前平台该列哪些类型。
 *
 * **纯函数**（平台判断由调用方给）：`navigator` 在测试里不可控，
 * 而「Windows 列 PowerShell，其他列 bash/zsh/fish」这条规则本身值得单测。
 */
export function terminalKindOptions({
  isWindows,
  isWslTarget
}: {
  isWindows: boolean;
  isWslTarget: boolean;
}): TerminalKindOption[] {
  const options: TerminalKindOption[] = [{ kind: "auto", label: "默认" }];
  if (isWindows) {
    options.push(
      { kind: "powershell", label: "PowerShell" },
      { kind: "cmd", label: "CMD" },
      { kind: "git-bash", label: "Git Bash" },
      { kind: "wsl-default", label: "默认 WSL" }
    );
  } else {
    options.push(
      { kind: "bash", label: "bash" },
      { kind: "zsh", label: "zsh" },
      { kind: "fish", label: "fish" }
    );
  }
  // 「当前 WSL」只有目标是 WSL 时才有意义——它要那个发行版，本机目标给不出。
  if (isWslTarget) options.push({ kind: "wsl", label: "当前 WSL" });
  return options;
}

/** 新开一路时默认选哪个：WSL 目标用那个发行版，其余交给后端按平台决定。 */
export function defaultTerminalKind(isWslTarget: boolean): TerminalKind {
  return isWslTarget ? "wsl" : "auto";
}

/** 显示名。下拉里找不到时的兜底（例如存下来的值已经不在当前平台的选项里）。 */
export function terminalKindLabel(kind: TerminalKind): string {
  const labels: Record<TerminalKind, string> = {
    auto: "默认",
    powershell: "PowerShell",
    cmd: "CMD",
    "git-bash": "Git Bash",
    wsl: "当前 WSL",
    "wsl-default": "默认 WSL",
    bash: "bash",
    zsh: "zsh",
    fish: "fish"
  };
  return labels[kind];
}

/** 某个类型在当前平台是不是可选；不可选时**退回默认**而不是让后端报错。 */
export function ensureSelectableKind(
  kind: TerminalKind,
  options: TerminalKindOption[]
): TerminalKind {
  return options.some((option) => option.kind === kind) ? kind : "auto";
}

/** 当前是不是 Windows。用 `userAgent` 而不是已废弃的 `navigator.platform`。 */
export function detectWindows(): boolean {
  return /windows/i.test(navigator.userAgent);
}
