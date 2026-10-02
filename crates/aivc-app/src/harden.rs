//! 窗口加固：导航白名单、外链走系统浏览器、禁用 `<webview>`。
//!
//! # 为什么这几条必须做
//!
//! 渲染层持有 `invoke` 的调用能力。一旦 webview 被导航到外部页面（用户点了个链接、
//! 页面里有个 `<a target="_blank">`），**外部内容就获得了同一套 IPC 能力**。因此：
//!
//! 1. **只有本应用自身的页面是受信任来源**：dev 走 Vite dev server，prod 走 `file://`。
//! 2. **新窗口一律拒绝**——不是「不允许打开」，而是根本不让 webview 创建新窗口。
//! 3. **非受信任的导航一律阻止**，其中 `http(s)` 交给系统浏览器：
//!    用户的意图（打开网页）被满足，但内容不在持有 IPC 能力的 webview 里加载。
//! 4. **禁止挂载 `<webview>`**——嵌套 webview 会绕过上面的所有判断。
//!
//! # 判定为什么做成纯函数
//!
//! 「什么算受信任」是安全边界，必须能穷举测试。URL 解析与字符串前缀比较容易写出
//! 「看起来对」但能被绕过（`file://` 前缀能被 `file://evil` 命中吗？`https://` 的
//! 大小写呢？）的实现，因此这里逐条断言。

/// 导航处置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationDecision {
    /// 允许（受信任来源）。
    Allow,
    /// 阻止，并且交给系统浏览器打开。
    BlockAndOpenExternally,
    /// 阻止，不做其他动作。
    Block,
}

impl NavigationDecision {
    /// 是否允许导航。
    #[must_use]
    pub const fn is_allowed(self) -> bool {
        matches!(self, Self::Allow)
    }

    /// 是否应当交给系统浏览器。
    #[must_use]
    pub const fn opens_externally(self) -> bool {
        matches!(self, Self::BlockAndOpenExternally)
    }
}

/// 判断 URL 是否属于受信任来源。
///
/// `dev_server_url` 为 [`Some`] 时按开发模式处理（前缀匹配 Vite dev server），
/// 否则按发布模式处理（`file://` 前缀）。
///
/// # 为什么是前缀匹配而不是解析后比较来源
///
/// 现网就是前缀匹配，而它对本场景是充分的：`file://` 只可能来自本地打包产物，
/// dev server 前缀由我们自己传入。改成「解析 URL 再比较 origin」会引入
/// `file://` 没有 origin 这类特例，反而更容易出错。
///
/// 但前缀匹配有个必须防的坑：**`file://` 不能命中 `file://evil.com/...`**。
/// 发布模式下允许的是「`file://` 开头的本地路径」，因此这里要求前缀之后是 `/`
/// 或字符串结束——`file://evil.com` 的前缀之后是 `e`，会被拒绝。
#[must_use]
pub fn is_trusted_url(target_url: &str, dev_server_url: Option<&str>) -> bool {
    if let Some(dev) = dev_server_url {
        let dev = dev.trim_end_matches('/');
        if dev.is_empty() {
            return false;
        }
        return target_url == dev
            || target_url
                .strip_prefix(dev)
                .is_some_and(|rest| rest.starts_with('/'));
    }
    is_local_file_url(target_url) || is_tauri_asset_url(target_url)
}

/// 是否是 Tauri 自身的资源协议。
///
/// **这一条不能漏**：Tauri 2 在 Windows 上默认用 `http://tauri.localhost` 提供打包后的
/// 前端资源，而不是 Electron 那样的 `file://`。只认 `file://` 会把应用自己的页面判成
/// 不受信任，于是每次加载都被拦下——窗口一片空白。
///
/// 只认这两个确切的主机名：`tauri.localhost` 是保留名，外部站点无法声明它。
fn is_tauri_asset_url(target_url: &str) -> bool {
    let lower = target_url.to_ascii_lowercase();
    ["tauri://localhost", "http://tauri.localhost"]
        .iter()
        .any(|prefix| {
            lower == *prefix
                || lower
                    .strip_prefix(prefix)
                    .is_some_and(|rest| rest.starts_with('/') || rest.starts_with('?'))
        })
}

/// 是否是本地打包页面的 `file://` URL。
///
/// 见 [`is_trusted_url`] 的说明：要求前缀之后是 `/`，挡住 `file://host/path` 这类
/// 指向远程主机的形式。
fn is_local_file_url(target_url: &str) -> bool {
    // 大小写不敏感：`FILE://` 与 `file://` 在 URL 方案上等价。
    let lower = target_url.to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("file://") else {
        return false;
    };
    // `file:///C:/...`（空主机）与 `file://C:/...` 都是本地路径；
    // `file://evil.com/x` 指向远程主机，必须拒绝。
    rest.starts_with('/') || is_windows_drive_path(rest)
}

/// 是否是 `C:/...` 形式的盘符路径。
fn is_windows_drive_path(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(ch) if ch.is_ascii_alphabetic())
        && matches!(chars.next(), Some(':'))
}

/// 是否是应当交给系统浏览器的外部链接。
///
/// 只认 `http` 与 `https`：`mailto:`、`tel:`、自定义协议都不放行——
/// 放行自定义协议等于允许页面唤起任意本地程序。
#[must_use]
pub fn is_external_link(target_url: &str) -> bool {
    let lower = target_url.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// 判定一次导航请求。
///
/// 与现网 `will-navigate` 的处理逐条对应。
#[must_use]
pub fn decide_navigation(target_url: &str, dev_server_url: Option<&str>) -> NavigationDecision {
    if is_trusted_url(target_url, dev_server_url) {
        return NavigationDecision::Allow;
    }
    if is_external_link(target_url) {
        NavigationDecision::BlockAndOpenExternally
    } else {
        NavigationDecision::Block
    }
}

/// 判定一次「新窗口」请求。
///
/// 现网 `setWindowOpenHandler` **一律拒绝**，只把 `http(s)` 转给系统浏览器——
/// 与导航的区别在于：导航可能落在受信任来源上（放行），新窗口永远不放行。
#[must_use]
pub fn decide_new_window(target_url: &str) -> NavigationDecision {
    if is_external_link(target_url) {
        NavigationDecision::BlockAndOpenExternally
    } else {
        NavigationDecision::Block
    }
}

/// Tauri 导航回调的适配层。
///
/// 返回 `true` 表示放行。非受信任且是外链时，顺便交给系统浏览器——用户的意图
/// （打开网页）被满足，但内容不在持有 IPC 能力的 webview 里加载。
///
/// 单独抽出来是为了让判定逻辑与 Tauri 的类型解耦，可被测试直接调用。
#[must_use]
pub fn attach_navigation_handler(target_url: &str, dev_server_url: Option<&str>) -> bool {
    let decision = decide_navigation(target_url, dev_server_url);
    if decision.opens_externally()
        && let Err(error) = open_externally(target_url)
    {
        tracing::warn!(%error, url = target_url, "交给系统浏览器失败");
    }
    decision.is_allowed()
}
/// 用系统默认程序打开链接。
///
/// # 为什么只允许 `http(s)`
///
/// 再次确认协议——这个函数是最后一道关。即使调用方漏判，也不能让页面唤起
/// 任意本地程序（`file://`、自定义协议都可能被用来执行本地命令）。
///
/// # Errors
///
/// 链接不是 `http(s)`，或系统调用失败时返回错误。
pub fn open_externally(target_url: &str) -> Result<(), String> {
    if !is_external_link(target_url) {
        return Err(format!("拒绝用系统程序打开非 HTTP(S) 链接：{target_url}"));
    }
    #[cfg(target_os = "windows")]
    let result = {
        // `cmd /c start` 是辅助动作：不给它配控制台窗口（否则会闪一个黑框）。
        let mut command = std::process::Command::new("cmd");
        aivc_infra::process::hide_console_window(&mut command);
        command.args(["/c", "start", "", target_url]).spawn()
    };
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(target_url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open")
        .arg(target_url)
        .spawn();

    result
        .map(|_| ())
        .map_err(|error| format!("打开外部链接失败：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEV: &str = "http://127.0.0.1:5173";

    #[test]
    fn the_dev_server_is_the_only_trusted_source_in_development() {
        assert!(is_trusted_url(DEV, Some(DEV)));
        assert!(is_trusted_url("http://127.0.0.1:5173/", Some(DEV)));
        assert!(is_trusted_url(
            "http://127.0.0.1:5173/index.html",
            Some(DEV)
        ));
        // 尾斜杠不该影响匹配。
        assert!(is_trusted_url(
            "http://127.0.0.1:5173/x",
            Some("http://127.0.0.1:5173/")
        ));
        // 其他来源一律不受信任。
        assert!(!is_trusted_url("http://127.0.0.1:5174/", Some(DEV)));
        assert!(!is_trusted_url("https://example.com/", Some(DEV)));
        assert!(!is_trusted_url("file:///C:/app/index.html", Some(DEV)));
    }

    #[test]
    fn a_prefix_that_is_not_a_path_boundary_is_rejected() {
        // 关键：`http://127.0.0.1:5173` 不能命中 `http://127.0.0.1:51730`——
        // 单纯的前缀比较会把它放行。
        assert!(!is_trusted_url("http://127.0.0.1:51730/", Some(DEV)));
        assert!(!is_trusted_url(
            "http://127.0.0.1:5173.evil.com/",
            Some(DEV)
        ));
    }

    #[test]
    fn local_files_are_trusted_in_production() {
        assert!(is_trusted_url("file:///C:/app/index.html", None));
        assert!(is_trusted_url("file:///home/u/app/index.html", None));
        assert!(is_trusted_url("FILE:///C:/app/index.html", None));
        // Windows 盘符形式（无前导斜杠）。
        assert!(is_trusted_url("file://C:/app/index.html", None));
    }

    #[test]
    fn a_remote_file_url_is_not_trusted() {
        // `file://evil.com/x` 指向远程主机，必须拒绝——这是 `file://` 前缀比较
        // 最容易漏掉的一种绕过。
        assert!(!is_trusted_url("file://evil.com/x", None));
        assert!(!is_trusted_url("file://attacker/share/index.html", None));
    }

    #[test]
    fn tauri_asset_urls_are_trusted_in_production() {
        // Tauri 2 在 Windows 上用 `http://tauri.localhost` 提供资源。漏掉它会导致
        // 应用自己的页面被拦下——窗口空白，且极难定位。
        assert!(is_trusted_url("http://tauri.localhost", None));
        assert!(is_trusted_url("http://tauri.localhost/", None));
        assert!(is_trusted_url("http://tauri.localhost/index.html", None));
        assert!(is_trusted_url("http://tauri.localhost/?x=1", None));
        assert!(is_trusted_url("tauri://localhost/index.html", None));
        // 近似的域名不是它。
        assert!(!is_trusted_url("http://tauri.localhost.evil.com/", None));
        assert!(!is_trusted_url("http://not-tauri.localhost/", None));
    }

    #[test]
    fn production_rejects_remote_http_sources() {
        assert!(!is_trusted_url("http://127.0.0.1:5173/", None));
        assert!(!is_trusted_url("https://example.com/", None));
    }

    #[test]
    fn an_empty_dev_server_url_trusts_nothing() {
        // 空串前缀会让「所有 URL 都以它开头」，是最危险的配置错误。
        assert!(!is_trusted_url("https://example.com/", Some("")));
        assert!(!is_trusted_url("https://example.com/", Some("   ")));
    }

    #[test]
    fn only_http_and_https_are_external_links() {
        assert!(is_external_link("http://example.com"));
        assert!(is_external_link("https://example.com"));
        assert!(is_external_link("HTTPS://EXAMPLE.COM"));
        // 其他协议一律不算——放行它们等于允许页面唤起任意本地程序。
        assert!(!is_external_link("file:///etc/passwd"));
        assert!(!is_external_link("mailto:a@b.c"));
        assert!(!is_external_link("javascript:alert(1)"));
        assert!(!is_external_link("ms-settings:"));
        assert!(!is_external_link("ftp://example.com"));
        assert!(!is_external_link(""));
    }

    #[test]
    fn navigation_decisions_match_the_desktop_reference() {
        // 受信任 → 放行
        assert_eq!(
            decide_navigation("file:///C:/app/index.html", None),
            NavigationDecision::Allow
        );
        // 外链 → 阻止 + 交给系统浏览器
        assert_eq!(
            decide_navigation("https://example.com", None),
            NavigationDecision::BlockAndOpenExternally
        );
        // 其他 → 只阻止
        assert_eq!(
            decide_navigation("file://evil.com/x", None),
            NavigationDecision::Block
        );
        assert_eq!(
            decide_navigation("mailto:a@b.c", None),
            NavigationDecision::Block
        );
    }

    #[test]
    fn new_windows_are_never_allowed() {
        // 与导航的区别：导航可能落在受信任来源上，新窗口永远不放行。
        for url in [
            "file:///C:/app/index.html",
            "https://example.com",
            "mailto:a@b.c",
            "about:blank",
        ] {
            assert!(
                !decide_new_window(url).is_allowed(),
                "{url} 不该被允许开新窗口"
            );
        }
        assert!(decide_new_window("https://example.com").opens_externally());
        assert!(!decide_new_window("about:blank").opens_externally());
    }

    #[test]
    fn the_decision_helpers_agree_with_their_predicates() {
        assert!(NavigationDecision::Allow.is_allowed());
        assert!(!NavigationDecision::Block.is_allowed());
        assert!(NavigationDecision::BlockAndOpenExternally.opens_externally());
        assert!(!NavigationDecision::Block.opens_externally());
    }

    #[test]
    fn opening_a_non_http_link_is_refused() {
        // 最后一道关：即使调用方漏判，也不能唤起本地程序。
        for url in ["file:///etc/passwd", "mailto:a@b.c", "javascript:alert(1)"] {
            let error = open_externally(url).expect_err("必须拒绝");
            assert!(error.contains("非 HTTP(S)"), "{error}");
        }
    }
}
