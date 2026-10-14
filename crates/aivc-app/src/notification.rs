//! Native notification delivery with a stable identity for portable releases.

/// Register the unpackaged application's own AUMID, including when running
/// directly from target/release. Never borrow PowerShell's notification identity.
#[cfg(windows)]
pub fn register() -> anyhow::Result<()> {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};
    const APP_ID: &str = "com.aivc.console";
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(format!(r"Software\Classes\AppUserModelId\{APP_ID}"))?;
    key.set_value("DisplayName", &"AI 可视化控制台")?;
    key.set_value("IconBackgroundColor", &"0")?;
    // A real image file is required for IconUri (an executable resource isn't a URI).
    let directory = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("LOCALAPPDATA unavailable"))?
        .join("com.aivc.console");
    std::fs::create_dir_all(&directory)?;
    let icon = directory.join("notification-icon.png");
    let bytes = include_bytes!("../icons/128x128.png");
    if std::fs::read(&icon).ok().as_deref() != Some(bytes.as_slice()) {
        std::fs::write(&icon, bytes)?;
    }
    key.set_value("IconUri", &icon.to_string_lossy().as_ref())?;
    Ok(())
}

/// Unlike the plugin's detached delivery task, propagate the actual WinRT error.
#[cfg(windows)]
pub async fn show(
    _app: &tauri::AppHandle,
    title: &str,
    detail: Option<&str>,
) -> Result<(), String> {
    let title = title.to_owned();
    let detail = detail.map(str::to_owned);
    tokio::task::spawn_blocking(move || {
        register().map_err(|error| error.to_string())?;
        let mut toast = tauri_winrt_notification::Toast::new("com.aivc.console").title(&title);
        if let Some(detail) = detail {
            toast = toast.text1(&detail);
        }
        toast.show().map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(not(windows))]
pub fn register() -> anyhow::Result<()> {
    Ok(())
}

#[cfg(not(windows))]
pub async fn show(app: &tauri::AppHandle, title: &str, detail: Option<&str>) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt as _;
    let mut builder = app.notification().builder().title(title);
    if let Some(detail) = detail {
        builder = builder.body(detail);
    }
    builder.show().map_err(|error| error.to_string())
}
