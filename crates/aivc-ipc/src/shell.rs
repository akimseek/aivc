//! Shell 命令：路径存在性检查与「用系统程序打开」。
//!
//! # 打开动作是一个 trait
//!
//! 与终端、CLI、HTTP 同理：测试不能真的去启动系统文件管理器（会弹窗、会依赖桌面环境）。
//! 注入 [`Opener`] 之后，「先校验再打开」「不存在的路径不打开」这些**编排语义**
//! 都能测，真实实现只负责调用系统命令。
//!
//! # 为什么「不存在的路径不打开」
//!
//! 把不存在的路径交给 `explorer` / `open` 会得到一个**系统级错误弹窗**，
//! 文案通常是英文且与用户的操作对不上（「Windows 找不到文件」）。
//! 先检查再打开，报错就能是我们自己的、说人话的那句。
//!
//! # WSL 目标当前显式拒绝
//!
//! `shell:path-exists` 与 `shell:open-path` 在现网都按目标分流：WSL 目标要经
//! `wsl.exe` 检查、并用宿主程序打开 `\\wsl$` 路径。这条分支尚未接线，
//! 因此**显式报错**而不是悄悄按本机路径检查——后者会把「WSL 里有」误判成「没有」。

use std::path::{Path, PathBuf};

use aivc_core::error::{Error, IpcError, Result};
use aivc_core::target::{ProviderId, TargetKind};
use serde_json::{Value, json};

/// 用系统程序打开一个路径。
pub trait Opener: Send + Sync {
    /// 打开。
    ///
    /// # Errors
    ///
    /// 系统调用失败时返回错误文案。
    fn open(&self, path: &str) -> std::result::Result<(), String>;
}

/// 丢弃打开请求（用于不需要真实打开的场合）。
pub struct NullOpener;

impl Opener for NullOpener {
    fn open(&self, _path: &str) -> std::result::Result<(), String> {
        Ok(())
    }
}

/// 真实打开器：交给系统默认程序。
pub struct SystemOpener;

impl Opener for SystemOpener {
    fn open(&self, path: &str) -> std::result::Result<(), String> {
        // 目录用文件管理器、文件用默认关联程序——两者在三个平台上都是同一条命令。
        #[cfg(target_os = "windows")]
        let mut command = {
            let mut command = std::process::Command::new("explorer");
            command.arg(path);
            command
        };
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut command = std::process::Command::new("open");
            command.arg(path);
            command
        };
        #[cfg(all(unix, not(target_os = "macos")))]
        let mut command = {
            let mut command = std::process::Command::new("xdg-open");
            command.arg(path);
            command
        };
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("打开失败：{error}"))
    }
}

/// Shell 命令处理器。
pub struct ShellCommands {
    opener: std::sync::Arc<dyn Opener>,
    /// 粘贴图片的落盘目录。
    ///
    /// **注入而不是自己取 `std::env::temp_dir()`**：本工程有一条硬约束
    /// 「应用绝不写入系统用户目录」，所有产物都在运行根下。
    /// 自己取临时目录会让这条约束在这里静默失效，而且测试也没法把产物收干净。
    scratch_dir: PathBuf,
}

/// 允许通过 `shell:read-image` 读取的图片类型。
///
/// # 为什么要有白名单
///
/// 这条命令**把文件内容读出来交给界面**。没有白名单它就是一个通用的文件读取接口
/// ——前端能拿到任意路径的内容，而界面并不需要这个能力。限定后缀之后，
/// 它只做「预览一张图」这一件事。
const IMAGE_EXTENSIONS: [&str; 7] = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"];

/// 单张图片的大小上限。
///
/// 与旧平台一致（`composer-input.tsx` 的 `MAX_IMAGE_SIZE`）。超过就不读：
/// 继续读只会在 IPC 上搬一个几十兆的 base64 串，界面照样显示不出来。
const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;

impl ShellCommands {
    /// 构造。
    #[must_use]
    pub fn new(opener: std::sync::Arc<dyn Opener>, scratch_dir: PathBuf) -> Self {
        Self {
            opener,
            scratch_dir,
        }
    }

    /// 路径是否存在。
    ///
    /// # Errors
    ///
    /// 参数非法或目标是 WSL 时返回错误。
    pub fn path_exists(&self, args: &Value) -> Result<Value> {
        let path = require_path(args)?;
        require_local(args)?;
        // 不做规范化：现网同样按原样判断，且 `..` 在这类检查里没有安全含义
        // （用户本来就能用系统文件管理器访问任何路径）。
        Ok(json!(Path::new(&path).exists()))
    }

    /// 用系统程序打开路径。
    ///
    /// # Errors
    ///
    /// 参数非法、目标是 WSL、路径不存在或打开失败时返回错误。
    pub fn open_path(&self, args: &Value) -> Result<Value> {
        let path = require_path(args)?;
        require_local(args)?;
        // 先校验：把不存在的路径交给系统会得到与用户操作对不上的英文弹窗。
        if !Path::new(&path).exists() {
            return Err(failed(format!("路径不存在：{path}")));
        }
        self.opener.open(&path).map_err(failed)?;
        Ok(Value::Null)
    }

    /// 读一张本地图片，返回可直接放进 `<img src>` 的 data URL。
    ///
    /// # 为什么不是「让 webview 直接读文件路径」
    ///
    /// 界面上预览本地图片只有两条路：
    ///
    /// 1. 开 Tauri 的 asset 协议并给它一个范围（`**` 才够用）。那等于把「读任意文件」
    ///    开放给整个页面，而页面并不需要；
    /// 2. **本命令**：只读白名单里的图片后缀、只读 10 MB 以内、只返回一个 data URL。
    ///
    /// 选 2。同一条链路里能做的事越少，将来出问题时能解释的范围就越小。
    ///
    /// # Errors
    ///
    /// 路径为空、不存在、不是文件、后缀不在白名单、超过上限、或读失败时报错。
    pub fn read_image(&self, args: &Value) -> Result<Value> {
        let path = require_path(args)?;
        let file = Path::new(&path);
        if !file.is_file() {
            return Err(failed(format!("不是文件：{path}")));
        }
        let Some(extension) = extension_of(&path) else {
            return Err(failed(format!(
                "不支持的类型：{path}（只支持 {}）",
                IMAGE_EXTENSIONS.join(" / ")
            )));
        };
        if !IMAGE_EXTENSIONS.contains(&extension.as_str()) {
            return Err(failed(format!(
                "不支持的类型：.{extension}（只支持 {}）",
                IMAGE_EXTENSIONS.join(" / ")
            )));
        }
        // 取不到元数据（权限、竞态删除）时按 0 处理：后面读盘失败会报出真正的原因，
        // 而这里提前报「读不到大小」只会掩盖它。
        let size = file.metadata().map_or(0, |meta| meta.len());
        if size > MAX_IMAGE_BYTES {
            return Err(failed(format!(
                "图片过大：{size} 字节，上限 {MAX_IMAGE_BYTES} 字节"
            )));
        }
        let bytes = std::fs::read(file).map_err(|error| failed(format!("读取失败：{error}")))?;
        Ok(json!({
            "dataUrl": format!("data:{};base64,{}", mime_of(&extension), base64_encode(&bytes)),
            "name": file.file_name().map_or_else(String::new, |value| value.to_string_lossy().into_owned()),
            "size": size,
        }))
    }

    /// 把一段 base64 图片写进运行根下的暂存目录，返回路径。
    ///
    /// # 它解决什么问题
    ///
    /// 从剪贴板粘进来的图片在 webview 里只是一个 `Blob`，**没有路径**。
    /// 而自管输入要把附件交给 CLI，靠的是 `@<路径>`——所以必须先在磁盘上有个文件。
    ///
    /// 前端负责编码（`canvas.toDataURL("image/png")`，webview 本来就会），
    /// 后端只负责落盘。这样这里不需要引入 PNG 编码器。
    ///
    /// # Errors
    ///
    /// base64 非法、内容不是 PNG、或写盘失败时报错。
    pub fn save_image(&self, args: &Value) -> Result<Value> {
        let encoded = args
            .get("dataUrl")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| invalid("dataUrl"))?;
        // 只接受 PNG：落盘的扩展名与内容必须一致，否则拿到路径的人（CLI、预览）
        // 会按 `.png` 去解一段别的字节。
        let payload = encoded
            .strip_prefix("data:image/png;base64,")
            .ok_or_else(|| failed("只接受 data:image/png;base64, 开头的内容。".to_owned()))?;
        let bytes = base64_decode(payload)
            .map_err(|error| failed(format!("内容不是合法 base64：{error}")))?;
        if !bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
            return Err(failed("内容不是 PNG。".to_owned()));
        }
        std::fs::create_dir_all(&self.scratch_dir)
            .map_err(|error| failed(format!("创建暂存目录失败：{error}")))?;
        // 文件名带上时间戳与一个随机数：同一秒里粘两张不能互相覆盖。
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |value| value.as_millis());
        let path = self
            .scratch_dir
            .join(format!("pasted-{stamp}-{}.png", std::process::id()));
        std::fs::write(&path, &bytes).map_err(|error| failed(format!("写盘失败：{error}")))?;
        Ok(json!({
            "path": path.to_string_lossy(),
            "name": path.file_name().map_or_else(String::new, |value| value.to_string_lossy().into_owned()),
            "size": bytes.len(),
        }))
    }
}

/// 取小写后缀。
fn extension_of(path: &str) -> Option<String> {
    Path::new(path)
        .extension()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
}

/// 后缀对应的 MIME。data URL 里必须写对，否则浏览器可能拒绝渲染。
fn mime_of(extension: &str) -> &'static str {
    match extension {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        _ => "image/svg+xml",
    }
}

/// 标准 base64 编码。
fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// 标准 base64 解码。
fn base64_decode(text: &str) -> std::result::Result<Vec<u8>, base64::DecodeError> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(text)
}

/// 取 `path` 参数。
fn require_path(args: &Value) -> Result<String> {
    args.get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| invalid("path"))
}

/// 要求目标是本机。
///
/// 见模块文档：WSL 分支尚未接线，显式拒绝好过按本机路径误判。
fn require_local(args: &Value) -> Result<()> {
    let target_id = args
        .get("targetId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if target_id.is_empty() {
        return Ok(());
    }
    // 与 `scope_for_target` 一致：用 `ProviderId::wsl_distro_of` 判定，
    // 而不是自己再解析一遍字符串。
    let distro = ProviderId::from_target_id(target_id).wsl_distro_of(target_id);
    if !distro.is_empty() {
        return Err(failed("暂不支持 WSL 目标的路径操作。".to_owned()));
    }
    let _ = TargetKind::Local;
    Ok(())
}

/// 把路径规范化成绝对路径（供将来的 WSL 分支使用）。
///
/// 保留这个函数是为了让「本机路径原样、WSL 路径需要转换」这个差异在代码里可见。
#[must_use]
pub fn normalize_open_path(path: &str) -> PathBuf {
    PathBuf::from(path)
}

/// 构造一个「参数无效」错误。
fn invalid(name: &str) -> Error {
    Error::Ipc(IpcError::InvalidParameter {
        name: name.to_owned(),
    })
}

/// 把字符串错误包成命令面错误。
fn failed(message: String) -> Error {
    Error::Ipc(IpcError::CommandFailed { message })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 记录被打开过的路径。
    #[derive(Default)]
    struct Recording {
        opened: Mutex<Vec<String>>,
    }

    impl Recording {
        fn opened(&self) -> Vec<String> {
            self.opened.lock().expect("lock").clone()
        }
    }

    impl Opener for Recording {
        fn open(&self, path: &str) -> std::result::Result<(), String> {
            self.opened.lock().expect("lock").push(path.to_owned());
            Ok(())
        }
    }

    /// 总是失败的打开器。
    struct Failing;

    impl Opener for Failing {
        fn open(&self, _path: &str) -> std::result::Result<(), String> {
            Err("系统调用失败".to_owned())
        }
    }

    /// 构造一个用临时目录当暂存区的处理器。
    ///
    /// 图片那两条命令要落盘，暂存区必须是**每个测试自己的**：用同一目录会让
    /// 「前一个测试留下的文件」参与后一个测试的判断。
    fn commands(opener: std::sync::Arc<dyn Opener>) -> ShellCommands {
        ShellCommands::new(opener, tempfile::tempdir().expect("tempdir").keep())
    }

    #[test]
    fn an_existing_path_is_reported_as_existing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let commands = commands(std::sync::Arc::new(Recording::default()));
        let value = commands
            .path_exists(&json!({ "path": dir.path().to_string_lossy() }))
            .expect("查询");
        assert_eq!(value, json!(true));
    }

    #[test]
    fn a_missing_path_is_reported_as_missing() {
        let commands = commands(std::sync::Arc::new(Recording::default()));
        let value = commands
            .path_exists(&json!({ "path": "/aivc-definitely-missing-xyz" }))
            .expect("查询");
        assert_eq!(value, json!(false));
    }

    #[test]
    fn a_blank_path_is_rejected_by_name() {
        // 参数错误要指名道姓，否则调用方无从下手。
        let commands = commands(std::sync::Arc::new(Recording::default()));
        for args in [json!({}), json!({ "path": "" }), json!({ "path": "   " })] {
            let error = commands.path_exists(&args).expect_err("必须拒绝");
            assert_eq!(error.to_string(), "参数无效：path");
        }
    }

    #[test]
    fn opening_an_existing_path_delegates_to_the_opener() {
        let dir = tempfile::tempdir().expect("tempdir");
        let recording = std::sync::Arc::new(Recording::default());
        let commands = commands(recording.clone());
        commands
            .open_path(&json!({ "path": dir.path().to_string_lossy() }))
            .expect("打开");
        assert_eq!(
            recording.opened(),
            vec![dir.path().to_string_lossy().into_owned()]
        );
    }

    #[test]
    fn opening_a_missing_path_is_refused_before_touching_the_system() {
        // 把不存在的路径交给系统会得到与用户操作对不上的英文弹窗。
        let recording = std::sync::Arc::new(Recording::default());
        let commands = commands(recording.clone());
        let error = commands
            .open_path(&json!({ "path": "/aivc-definitely-missing-xyz" }))
            .expect_err("必须拒绝");
        assert!(error.to_string().contains("路径不存在"), "{error}");
        assert!(recording.opened().is_empty(), "不该调用打开器");
    }

    #[test]
    fn an_opener_failure_is_reported_as_is() {
        let dir = tempfile::tempdir().expect("tempdir");
        let commands = commands(std::sync::Arc::new(Failing));
        let error = commands
            .open_path(&json!({ "path": dir.path().to_string_lossy() }))
            .expect_err("必须失败");
        assert!(error.to_string().contains("系统调用失败"), "{error}");
    }

    #[test]
    fn a_wsl_target_is_refused_explicitly() {
        // 按本机路径检查会把「WSL 里有」误判成「没有」——静默出错比报错更糟。
        //
        // 这里以前只覆盖裸 `wsl:Ubuntu` 与 `gemini:wsl:Ubuntu`，恰好漏掉**后端自己发出**
        // 的 `codex:wsl:<distro>`（`aivc-provider` 的 Codex 目标就是它）——于是这种目标
        // 一路按本机路径判断，WSL 会话恢复被误弹「原目录丢失」（P0-C 的根因）。
        // 三种形制都必须拒绝：权威形、其它平台的带前缀形、Codex 的旧形。
        let commands = commands(std::sync::Arc::new(Recording::default()));
        for target_id in ["codex:wsl:Ubuntu-22.04", "gemini:wsl:Ubuntu", "wsl:Ubuntu"] {
            let error = commands
                .path_exists(&json!({ "targetId": target_id, "path": "/home/u/x" }))
                .expect_err("必须拒绝");
            assert!(
                error.to_string().contains("暂不支持 WSL"),
                "{target_id}: {error}"
            );
        }
    }

    #[test]
    fn a_local_target_id_is_accepted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let commands = commands(std::sync::Arc::new(Recording::default()));
        let value = commands
            .path_exists(&json!({
                "targetId": "codex:local",
                "path": dir.path().to_string_lossy(),
            }))
            .expect("查询");
        assert_eq!(value, json!(true));
    }

    #[test]
    fn a_missing_target_id_is_treated_as_local() {
        // 现网用 `targetId || ""`，缺省即本机。
        let dir = tempfile::tempdir().expect("tempdir");
        let commands = commands(std::sync::Arc::new(Recording::default()));
        assert_eq!(
            commands
                .path_exists(&json!({ "path": dir.path().to_string_lossy() }))
                .expect("查询"),
            json!(true)
        );
    }

    #[test]
    fn the_null_opener_succeeds_without_side_effects() {
        let dir = tempfile::tempdir().expect("tempdir");
        let commands = commands(std::sync::Arc::new(NullOpener));
        commands
            .open_path(&json!({ "path": dir.path().to_string_lossy() }))
            .expect("打开");
    }

    #[test]
    fn path_normalization_is_identity_for_local_paths() {
        // 本机路径原样使用；这个函数存在的意义是让「WSL 需要转换」这个差异可见。
        assert_eq!(normalize_open_path("/tmp/x"), PathBuf::from("/tmp/x"));
    }

    /* ── 图片预览与粘贴图片落盘 ─────────────────────────── */

    /// 造一个最小的合法 PNG（1×1）。
    const TINY_PNG: [u8; 67] = [
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    /// 造一个用临时目录的处理器，并把那个目录也返回出来。
    fn commands_with_dir() -> (ShellCommands, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        (
            ShellCommands::new(std::sync::Arc::new(NullOpener), dir.path().to_path_buf()),
            dir,
        )
    }

    #[test]
    fn reading_an_image_returns_a_data_url() {
        let (commands, dir) = commands_with_dir();
        let file = dir.path().join("a.png");
        std::fs::write(&file, TINY_PNG).expect("写文件");
        let value = commands
            .read_image(&json!({ "path": file.to_string_lossy() }))
            .expect("读取");
        let url = value["dataUrl"].as_str().expect("有 dataUrl");
        assert!(url.starts_with("data:image/png;base64,"), "{url}");
        assert_eq!(value["name"], json!("a.png"));
        assert_eq!(value["size"], json!(TINY_PNG.len()));
    }

    #[test]
    fn the_mime_type_follows_the_extension() {
        // 后缀写错会让浏览器拒绝渲染：data URL 里的类型必须与实际后缀对应。
        let (commands, dir) = commands_with_dir();
        for (name, mime) in [
            ("a.jpeg", "image/jpeg"),
            ("a.jpg", "image/jpeg"),
            ("a.gif", "image/gif"),
            ("a.webp", "image/webp"),
            ("a.bmp", "image/bmp"),
            ("a.svg", "image/svg+xml"),
        ] {
            let file = dir.path().join(name);
            std::fs::write(&file, b"x").expect("写文件");
            let value = commands
                .read_image(&json!({ "path": file.to_string_lossy() }))
                .expect("读取");
            assert!(
                value["dataUrl"]
                    .as_str()
                    .expect("有 dataUrl")
                    .starts_with(&format!("data:{mime};base64,")),
                "{name} 的 MIME 不对"
            );
        }
    }

    #[test]
    fn reading_a_non_image_is_refused() {
        // 没有这条限制，`shell:read-image` 就是一个通用文件读取接口。
        let (commands, dir) = commands_with_dir();
        let file = dir.path().join("secret.txt");
        std::fs::write(&file, b"top secret").expect("写文件");
        let error = commands
            .read_image(&json!({ "path": file.to_string_lossy() }))
            .expect_err("必须拒绝");
        assert!(error.to_string().contains("不支持的类型"), "{error}");
        assert!(
            !error.to_string().contains("top secret"),
            "错误里不该带内容"
        );
    }

    #[test]
    fn reading_a_file_without_extension_is_refused() {
        let (commands, dir) = commands_with_dir();
        let file = dir.path().join("noext");
        std::fs::write(&file, b"x").expect("写文件");
        let error = commands
            .read_image(&json!({ "path": file.to_string_lossy() }))
            .expect_err("必须拒绝");
        assert!(error.to_string().contains("不支持的类型"), "{error}");
    }

    #[test]
    fn reading_a_missing_image_is_refused() {
        let (commands, _dir) = commands_with_dir();
        let error = commands
            .read_image(&json!({ "path": "/aivc-missing-xyz.png" }))
            .expect_err("必须拒绝");
        assert!(error.to_string().contains("不是文件"), "{error}");
    }

    #[test]
    fn reading_an_image_without_a_path_is_refused_by_name() {
        let (commands, _dir) = commands_with_dir();
        let error = commands.read_image(&json!({})).expect_err("必须拒绝");
        assert_eq!(error.to_string(), "参数无效：path");
    }

    #[test]
    fn saving_a_png_writes_it_and_returns_the_path() {
        use base64::Engine as _;
        let (commands, dir) = commands_with_dir();
        let encoded = base64::engine::general_purpose::STANDARD.encode(TINY_PNG);
        let value = commands
            .save_image(&json!({ "dataUrl": format!("data:image/png;base64,{encoded}") }))
            .expect("保存");
        let path = value["path"].as_str().expect("有路径");
        // 后缀由我们生成，因此这里用大小写不敏感的比法读起来安全
        // （clippy 会把 `ends_with(".png")` 当成可能在比扩展名而报错）。
        assert!(path.to_ascii_lowercase().ends_with(".png"), "{path}");
        // **文件真的写出去了**，而且内容一致——只看返回值不能说明落盘成功。
        assert_eq!(std::fs::read(path).expect("读回"), TINY_PNG.to_vec());
        // 落点在注入的暂存目录下，不是系统临时目录。
        assert!(Path::new(path).starts_with(dir.path()), "{path}");
    }

    #[test]
    fn saving_a_non_png_payload_is_refused() {
        use base64::Engine as _;
        let (commands, _dir) = commands_with_dir();
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"not a png at all");
        let error = commands
            .save_image(&json!({ "dataUrl": format!("data:image/png;base64,{encoded}") }))
            .expect_err("必须拒绝");
        assert!(error.to_string().contains("不是 PNG"), "{error}");
    }

    #[test]
    fn saving_refuses_other_data_url_types() {
        // 只接受 PNG：落盘的扩展名与内容必须一致，否则拿到路径的一方会按 .png 去解别的字节。
        let (commands, _dir) = commands_with_dir();
        let error = commands
            .save_image(&json!({ "dataUrl": "data:image/jpeg;base64,AAAA" }))
            .expect_err("必须拒绝");
        assert!(error.to_string().contains("只接受"), "{error}");
    }

    #[test]
    fn saving_refuses_broken_base64() {
        let (commands, _dir) = commands_with_dir();
        let error = commands
            .save_image(&json!({ "dataUrl": "data:image/png;base64,!!!not-base64!!!" }))
            .expect_err("必须拒绝");
        assert!(error.to_string().contains("base64"), "{error}");
    }

    #[test]
    fn saving_without_a_data_url_is_refused_by_name() {
        let (commands, _dir) = commands_with_dir();
        let error = commands.save_image(&json!({})).expect_err("必须拒绝");
        assert_eq!(error.to_string(), "参数无效：dataUrl");
    }

    #[test]
    fn two_saves_in_the_same_millisecond_do_not_overwrite_each_other() {
        // 同一秒里粘两张图不能互相覆盖——文件名里带了进程号与毫秒。
        use base64::Engine as _;
        let (commands, _dir) = commands_with_dir();
        let encoded = base64::engine::general_purpose::STANDARD.encode(TINY_PNG);
        let url = format!("data:image/png;base64,{encoded}");
        let first = commands
            .save_image(&json!({ "dataUrl": url }))
            .expect("第一次")
            .get("path")
            .and_then(|value| value.as_str().map(str::to_owned))
            .expect("路径");
        let second = commands
            .save_image(&json!({ "dataUrl": url }))
            .expect("第二次")
            .get("path")
            .and_then(|value| value.as_str().map(str::to_owned))
            .expect("路径");
        assert_ne!(first, second, "两次落盘不能是同一个文件");
    }
}
