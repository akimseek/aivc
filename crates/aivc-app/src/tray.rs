//! 系统托盘。
//!
//! # 与现网逐条对齐
//!
//! | 行为 | 现网 | 这里 |
//! | --- | --- | --- |
//! | 提示文字 | `AI 可视化控制台` | [`TOOLTIP`] |
//! | 右键菜单 | 显示窗口 / 分隔符 / 退出 | [`menu_spec`] |
//! | 单击 | 恢复窗口 | [`restore_window`] |
//! | 创建次数 | `if (applicationTray) return` —— 只建一次 | 挂在 `setup` 里，只调一次 |
//!
//! # 「退出」走确认流程
//!
//! 现网的托盘「退出」会先弹三选一（最小化到托盘 / 直接退出 / 取消，即矩阵 A4）。
//! 这里与之一致：走 [`crate::exit`] 的确认流程，而不是直接退出。
//!
//! 这条曾经是直接 `app.exit(0)`——既跳过了确认，也**跳过了全部退出清理**
//! （清理挂在 `RunEvent::ExitRequested` 的 `code: None` 分支上，而 `exit(0)` 带码）。
//!
//! # 恢复窗口为什么要三步
//!
//! 最小化的窗口 `show()` 不会自动还原，必须先 `unminimize()`；而 `show()` 之后还要
//! `set_focus()`，否则窗口出现在任务栏里但不是前台。少任何一步，用户点托盘都「没反应」。

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::Manager as _;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Runtime};

/// 托盘提示文字。
pub const TOOLTIP: &str = "AI 可视化控制台";

/// 主窗口标签。
pub const MAIN_WINDOW: &str = "main";

/// 托盘是否已经建起来了。
///
/// # 为什么需要知道这件事
///
/// 「最小化到托盘」会 `hide()` 窗口——之后**只能**通过托盘找回来。托盘没建起来时
/// 这么做会让应用彻底够不着（没有窗口、没有任务栏图标、也没有托盘图标）。
/// 所以那条路径必须先问一句。
///
/// 用全局原子量而不是挂在 `AppState` 上：托盘是**进程级**资源，只建一次，
/// 而 `install` 失败时进程仍在运行。
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// 托盘是否可用。
#[must_use]
pub fn is_installed() -> bool {
    INSTALLED.load(Ordering::Relaxed)
}

/// 菜单项编号。
pub mod ids {
    /// 显示窗口。
    pub const SHOW: &str = "tray-show";
    /// 退出。
    pub const QUIT: &str = "tray-quit";
}

/// 一个菜单项的描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuEntry {
    /// 可点击项。
    Item {
        /// 编号。
        id: &'static str,
        /// 显示文字。
        label: &'static str,
    },
    /// 分隔线。
    Separator,
}

/// 菜单结构。
///
/// 单独抽出来是为了能在**不启动 Tauri** 的情况下断言菜单内容与顺序——
/// 顺序与文字都是用户可见的契约。
#[must_use]
pub fn menu_spec() -> Vec<MenuEntry> {
    vec![
        MenuEntry::Item {
            id: ids::SHOW,
            label: "显示窗口",
        },
        MenuEntry::Separator,
        MenuEntry::Item {
            id: ids::QUIT,
            label: "退出",
        },
    ]
}

/// 恢复并聚焦主窗口。
///
/// 见模块文档：三步缺一不可。
pub fn restore_window<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        // 窗口还没建好（例如启动早期的托盘点击）：什么都不做比 panic 好。
        return;
    };
    // 最小化的窗口不先还原，`show()` 不会让它回到前台。
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

/// 装配托盘。
///
/// # Errors
///
/// 图标缺失或菜单构造失败时返回错误。托盘失败**不阻止应用启动**——没有托盘仍然
/// 可以用窗口，因此调用方应当记日志而不是退出。
pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let mut items: Vec<Box<dyn tauri::menu::IsMenuItem<R>>> = Vec::new();
    let mut owned: Vec<MenuItem<R>> = Vec::new();
    let mut separators: Vec<PredefinedMenuItem<R>> = Vec::new();

    for entry in menu_spec() {
        match entry {
            MenuEntry::Item { id, label } => {
                let item = MenuItem::with_id(app, id, label, true, None::<&str>)?;
                owned.push(item);
            }
            MenuEntry::Separator => {
                separators.push(PredefinedMenuItem::separator(app)?);
            }
        }
    }
    // 按声明的顺序组装：菜单顺序是用户可见的契约，不能因为分组构造而打乱。
    let mut item_iter = owned.iter();
    let mut separator_iter = separators.iter();
    for entry in menu_spec() {
        match entry {
            MenuEntry::Item { .. } => {
                if let Some(item) = item_iter.next() {
                    items.push(Box::new(item.clone()));
                }
            }
            MenuEntry::Separator => {
                if let Some(separator) = separator_iter.next() {
                    items.push(Box::new(separator.clone()));
                }
            }
        }
    }
    let refs: Vec<&dyn tauri::menu::IsMenuItem<R>> = items.iter().map(AsRef::as_ref).collect();
    let menu = Menu::with_items(app, &refs)?;

    let mut builder = TrayIconBuilder::new()
        .tooltip(TOOLTIP)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            ids::SHOW => restore_window(app),
            ids::QUIT => {
                // 走确认流程（A4）。前端不响应时窗口**仍然关得掉**——用户再点一次
                // 关闭即可，见 `crate::exit` 模块的失败安全。
                let state = app.state::<crate::tauri_app::AppState>();
                crate::exit::request_exit(app, &state.exit_flow);
            }
            other => tracing::debug!(id = other, "未处理的托盘菜单项"),
        })
        .on_tray_icon_event(|tray, event| {
            // 只响应左键「抬起」：按下时触发会让一次点击被处理两次。
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                restore_window(tray.app_handle());
            }
        });

    // 图标来自 `tauri.conf.json` 里嵌入的 `bundle.icon`；缺失时托盘建不出来。
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    } else {
        tracing::warn!("没有默认窗口图标，托盘将使用系统默认图标");
    }

    builder.build(app)?;
    // 建成功才记下来：「最小化到托盘」靠它决定是隐藏窗口还是退化成普通最小化。
    INSTALLED.store(true, Ordering::Relaxed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_matches_the_desktop_reference() {
        // 文字与顺序都是用户可见的契约，逐项断言。
        assert_eq!(
            menu_spec(),
            vec![
                MenuEntry::Item {
                    id: "tray-show",
                    label: "显示窗口",
                },
                MenuEntry::Separator,
                MenuEntry::Item {
                    id: "tray-quit",
                    label: "退出",
                },
            ]
        );
    }

    #[test]
    fn the_tooltip_matches_the_desktop_reference() {
        assert_eq!(TOOLTIP, "AI 可视化控制台");
    }

    #[test]
    fn menu_ids_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for entry in menu_spec() {
            if let MenuEntry::Item { id, .. } = entry {
                assert!(seen.insert(id), "菜单编号重复：{id}");
            }
        }
    }

    #[test]
    fn every_item_has_a_non_empty_label() {
        for entry in menu_spec() {
            if let MenuEntry::Item { id, label } = entry {
                assert!(!label.trim().is_empty(), "{id} 的文字为空");
            }
        }
    }

    #[test]
    fn the_separator_sits_between_the_two_items() {
        // 分隔线在首位或末位都没有意义。
        let spec = menu_spec();
        let position = spec
            .iter()
            .position(|entry| matches!(entry, MenuEntry::Separator))
            .expect("应当有分隔线");
        assert!(position > 0 && position < spec.len() - 1, "分隔线位置不对");
    }
}
