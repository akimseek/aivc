//! 子进程启动的 Windows 细节。
//!
//! # 控制台窗口：一个真机缺陷（2026-10-02）
//!
//! 从 GUI 进程启动控制台子系统程序（`wsl.exe`、`cmd.exe`、`node.exe`……）时，
//! 若不加 `CREATE_NO_WINDOW`，Windows 会给子进程**配一个可见的控制台窗口**。
//! Win11 的默认终端是 Windows Terminal，于是用户看到的是「弹 cmd/Terminal 面板」。
//!
//! 实测（进程观察器，release 构建）：应用启动后 ~1 秒内 `aivc-app` 直接 spawn 了
//! **3 个 `wsl.exe`**（目标预热，一个目标一次），每个都拉起 conhost + 一个
//! Windows Terminal 窗口；而**常驻的 WSL worker 让最后一个窗口一直不关**。
//! 选择平台触发目标刷新时同样会弹。
//!
//! # 规矩
//!
//! **所有辅助性子进程**（探测、WSL 一次性脚本、环境检查、app-server）都必须经由
//! 这两个帮手创建命令。唯一例外是**要呈现给用户的终端**——那条路走 `ConPTY`
//! （`aivc-terminal`），本来就没有窗口。
//!
//! 非 Windows 平台这些函数是空操作（桌面 Linux/macOS 与终端里运行本就没有这个问题）。

/// 给 `std::process::Command` 加上「不弹控制台窗口」。
///
/// 调用点紧贴创建处，例如：
///
/// ```ignore
/// let mut command = std::process::Command::new("wsl.exe");
/// aivc_infra::process::hide_console_window(&mut command);
/// ```
pub fn hide_console_window(command: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

/// 给 `tokio::process::Command` 加上「不弹控制台窗口」（语义同上）。
pub fn hide_console_window_tokio(command: &mut tokio::process::Command) {
    #[cfg(windows)]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

/// <https://learn.microsoft.com/windows/win32/procthread/process-creation-flags>
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hiding_the_console_window_is_safe_to_call_and_keeps_the_command_usable() {
        // 帮手本身没有跨平台的判据（Windows 上生效、其它平台是空操作），但至少要钉住
        // 「调用后命令还能正常用」——把这行加错地方导致构建不出命令，就是灾难。
        let mut std_command = std::process::Command::new("cmd");
        hide_console_window(&mut std_command);
        let debug = format!("{std_command:?}");
        assert!(debug.contains("cmd"), "命令应仍然指向 cmd：{debug}");

        let mut tokio_command = tokio::process::Command::new("cmd");
        hide_console_window_tokio(&mut tokio_command);
        let debug = format!("{tokio_command:?}");
        assert!(debug.contains("cmd"), "命令应仍然指向 cmd：{debug}");
    }
}
