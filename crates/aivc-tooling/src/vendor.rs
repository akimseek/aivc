//! 供应商配置写入：模板渲染、默认模板、备份与原子写入。
//!
//! # 这一块为什么危险
//!
//! 它**改用户已有的 CLI 配置文件**（`~/.codex/auth.json`、`~/.claude/settings.json` 等）。
//! 这些文件里可能还有用户自己写的内容，写坏了 Codex / Claude 直接不能用。因此：
//!
//! 1. **写入前必须备份**（[`backup_before_write`]）；
//! 2. **内容为空必须中止**——空模板会把用户配置清空；
//! 3. **路径必须过白名单**（[`aivc_core::shell::assert_allowed_config_path`]）。
//!
//! # 三个占位符
//!
//! `{{API_KEY}}` / `{{BASE_URL}}` / `{{VENDOR_NAME}}`。用简单字符串替换而不是模板引擎：
//! 模板内容由用户提供，引入模板引擎等于允许在配置文件里执行逻辑。
//!
//! # 默认模板为什么是硬编码的字符串
//!
//! 这些 JSON 也可以用序列化生成，但**手写常量能保证字节级稳定**：用户可能已经
//! 手工格式化过自己的配置，默认模板只在内容为空时使用，因此它必须与现网逐字节一致，
//! 否则第一次启用就会把文件重排一遍。

use std::path::{Path, PathBuf};

use aivc_core::target::ProviderId;

/// 备份目录名。
pub const VENDOR_BACKUP_DIR: &str = "vendor-backups";

/// 供应商名的最大长度（用于备份路径）。
const SAFE_NAME_MAX: usize = 100;

/// Codex 默认的 model provider 名。
pub const CODEX_DEFAULT_MODEL_PROVIDER: &str = "akim";

/// 把名字转成可安全用作路径片段的形式。
///
/// 替换路径分隔符、Windows 保留字符与控制字符；截断到 100 字符；空结果退回 `vendor`。
///
/// 供应商名是**用户输入**，直接拼进备份路径就可能写到预期之外的位置。
#[must_use]
pub fn safe_name(value: &str) -> String {
    let replaced: String = value
        .chars()
        .map(|ch| {
            if matches!(ch, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || ch.is_control()
            {
                '_'
            } else {
                ch
            }
        })
        .collect();
    let truncated: String = replaced.chars().take(SAFE_NAME_MAX).collect();
    if truncated.is_empty() {
        "vendor".to_owned()
    } else {
        truncated
    }
}

/// 渲染模板里的三个占位符。
///
/// **全部替换**（相当于正则的 `g` 标志）：同一个占位符可能出现多次，只替换第一处
/// 会让后续的明文占位符被原样写进配置文件——那是一个看起来像配置错误的怪异状态。
#[must_use]
pub fn render_template(content: &str, api_key: &str, base_url: &str, vendor_name: &str) -> String {
    content
        .replace("{{API_KEY}}", api_key)
        .replace("{{BASE_URL}}", base_url)
        .replace("{{VENDOR_NAME}}", vendor_name)
}

/// 路径是否以某后缀结尾（大小写敏感，与现网一致）。
fn ends_with(target_path: &str, suffix: &str) -> bool {
    // 反斜杠统一成正斜杠后再比较：Windows 路径用 `\` 分隔。
    target_path.replace('\\', "/").ends_with(suffix)
}

/// 某个平台 + 路径的默认配置内容。
///
/// 内容为空时才会用到。返回空串表示「没有默认模板」。
#[must_use]
pub fn default_config_content(provider: ProviderId, target_path: &str) -> &'static str {
    match provider {
        ProviderId::Codex if ends_with(target_path, "/auth.json") => {
            "{\n  \"OPENAI_API_KEY\": \"{{API_KEY}}\"\n}"
        }
        ProviderId::Codex if ends_with(target_path, "/config.toml") => {
            "model_provider = \"akim\"\n\n[model_providers.akim]\nname = \"akim\"\nwire_api = \"responses\"\nrequires_openai_auth = true\nbase_url = \"{{BASE_URL}}\""
        }
        ProviderId::Gemini if ends_with(target_path, "/.env") => {
            "GEMINI_API_KEY={{API_KEY}}\nGOOGLE_GEMINI_BASE_URL={{BASE_URL}}"
        }
        ProviderId::Gemini if ends_with(target_path, "/settings.json") => {
            "{\n  \"security\": {\n    \"auth\": {\n      \"selectedType\": \"gemini-api-key\"\n    }\n  }\n}"
        }
        ProviderId::Claude if ends_with(target_path, "/settings.json") => {
            "{\n  \"env\": {\n    \"ANTHROPIC_BASE_URL\": \"{{BASE_URL}}\",\n    \"ANTHROPIC_AUTH_TOKEN\": \"{{API_KEY}}\"\n  },\n  \"theme\": \"dark\"\n}"
        }
        // Qoder 没有默认模板：它的配置格式因版本而异，猜一个反而更糟。
        ProviderId::Qoder | ProviderId::Codex | ProviderId::Gemini | ProviderId::Claude => "",
    }
}

/// 内容为空时套用默认模板。
#[must_use]
pub fn normalize_config_content(provider: ProviderId, target_path: &str, content: &str) -> String {
    if !content.trim().is_empty() {
        return content.to_owned();
    }
    default_config_content(provider, target_path).to_owned()
}

/// 某个平台的**标准配置文件清单**：路径 + 默认内容。
///
/// # 为什么需要它
///
/// 「启用供应商」在旧平台的含义是**把它的凭据写进 CLI 的配置文件**（写前备份），
/// 后端 `enable` 也是这么做的——但它要求供应商至少有一个启用的配置模板
/// （`请至少启用一个配置文件。`），而在此之前**没有任何地方**告诉调用方
/// 「这个平台该写哪些文件」：
///
/// - [`default_config_content`] 只按**路径后缀**匹配内容，本身不含路径；
/// - 目录来自各 provider 的 `default_config_dir()`，那是私有的。
///
/// 于是界面只能自己编一份路径表——而路径是 Rust 侧的约定，编一份必然漂移。
/// 这份清单把它变回**一处真值**。
///
/// 返回空向量表示这个平台没有标准模板（Qoder：它的配置格式因版本而异，
/// 猜一个比不猜更糟）。
#[must_use]
pub fn default_config_targets(provider: ProviderId) -> Vec<&'static str> {
    match provider {
        ProviderId::Codex => vec!["~/.codex/auth.json", "~/.codex/config.toml"],
        ProviderId::Gemini => vec!["~/.gemini/.env", "~/.gemini/settings.json"],
        ProviderId::Claude => vec!["~/.claude/settings.json"],
        ProviderId::Qoder => Vec::new(),
    }
}

/// 把平台的默认配置模板组装成 `configs` 的形状（全部启用）。
///
/// 调用方是保存供应商那条路径：调用方没给 `configs` 时用这个兜底，
/// 否则「启用」必然失败，而失败信息（`请至少启用一个配置文件。`）
/// 对用户来说无从下手——他没有「配置文件」这个概念。
#[must_use]
pub fn default_vendor_configs(provider: ProviderId) -> Vec<(String, String)> {
    default_config_targets(provider)
        .into_iter()
        .map(|path| {
            (
                path.to_owned(),
                default_config_content(provider, path).to_owned(),
            )
        })
        .collect()
}

/// 备份根目录。`base` 是**父目录**——本函数自己拼一层 [`VENDOR_BACKUP_DIR`]。
///
/// 结果是 `base/vendor-backups/<供应商名>/<时间戳>/`。用时间戳而不是覆盖同名备份：
/// 反复启用同一个供应商时，每一次的历史内容都值得保留。
///
/// # 调用方注意「父目录」这件事
///
/// 传 `data/`，不要传 `AppPaths::vendor_backup_dir()`（那已经是
/// `data/vendor-backups`）——否则会得到 `data/vendor-backups/vendor-backups/...`。
/// 实测踩过：备份真的落在那个重复了一层的目录里。
#[must_use]
pub fn backup_root(base: &Path, vendor_name: &str, enabled_at: &str) -> PathBuf {
    // 时间戳里的 `:` 与 `.` 在 Windows 上非法，替换掉。
    let stamp: String = enabled_at
        .chars()
        .map(|ch| if ch == ':' || ch == '.' { '-' } else { ch })
        .collect();
    base.join(VENDOR_BACKUP_DIR)
        .join(safe_name(vendor_name))
        .join(stamp)
}

/// 备份单个文件的路径。
#[must_use]
pub fn backup_file_path(backup_root: &Path, scope: &str, destination: &str) -> PathBuf {
    backup_root.join(scope).join(safe_name(destination))
}

/// 一次写入的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteOutcome {
    /// 目标文件。
    pub destination: String,
    /// 备份文件；目标原本不存在时为 [`None`]。
    pub backup: Option<String>,
    /// 是否新建了文件。
    pub created: bool,
}

/// 写入一个配置文件，写入前先备份。
///
/// # 为什么要先备份
///
/// 目标文件里可能有用户自己写的内容（额外的环境变量、注释、别的供应商配置）。
/// 覆盖之后没有备份就找不回来了。
///
/// # 为什么空内容必须中止
///
/// 空内容会把用户配置**清空**——这比「写入失败」严重得多，因为它看起来是成功的。
///
/// # Errors
///
/// 内容为空、目标路径非法、或读写失败时返回错误。
pub fn write_config_file(
    target_path: &str,
    content: &str,
    backup_root: &Path,
    scope: &str,
    home: &Path,
) -> Result<WriteOutcome, String> {
    if content.trim().is_empty() {
        return Err(format!("配置文件内容为空，已中止写入：{target_path}"));
    }
    // 路径白名单是最后一道关：即使调用方漏判，也不能写到白名单之外。
    aivc_core::shell::assert_allowed_config_path(target_path)
        .map_err(|error| format!("{error}"))?;
    write_file_with_backup(&expand_home(target_path, home), content, backup_root, scope)
}

/// 写入一个已解析好的目标路径，写入前先备份。
///
/// 与 [`write_config_file`] 分开是刻意的：**「写到哪里」与「怎么写才安全」是两件事**。
/// 前者靠白名单与 `~` 展开决定，后者是「先备份、再写入、空内容拒绝」这套机制。
/// 拆开之后，写入机制可以用临时目录穷举测试，不需要伪造 `HOME` 环境变量
/// （那是进程级全局状态，并行测试里会互相干扰）。
///
/// # Errors
///
/// 内容为空或读写失败时返回错误。
pub fn write_file_with_backup(
    destination: &Path,
    content: &str,
    backup_root: &Path,
    scope: &str,
) -> Result<WriteOutcome, String> {
    if content.trim().is_empty() {
        return Err(format!(
            "配置文件内容为空，已中止写入：{}",
            destination.display()
        ));
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("创建目录失败：{error}"))?;
    }

    // 目标存在才备份：不存在时备份一个空文件没有意义，反而会让「有没有旧内容」
    // 这个判断变得含糊。
    let mut backup = None;
    if destination.is_file() {
        let path = backup_file_path(backup_root, scope, &destination.to_string_lossy());
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("创建备份目录失败：{error}"))?;
        }
        std::fs::copy(destination, &path).map_err(|error| format!("备份失败：{error}"))?;
        backup = Some(path.to_string_lossy().into_owned());
    }

    let created = !destination.is_file();
    std::fs::write(destination, content).map_err(|error| format!("写入配置失败：{error}"))?;
    Ok(WriteOutcome {
        destination: destination.to_string_lossy().into_owned(),
        backup,
        created,
    })
}

/// 把开头的 `~/` 展开成**给定的**用户主目录。
///
/// # 为什么主目录必须显式传进来，而不是在这里读环境变量
///
/// 这里原来自己读 `HOME` / `USERPROFILE`（进程级全局状态）。后果实测到了：
/// 一条 `aivc-ipc` 的测试走到了「启用供应商」的成功路径，而它**真的把用户的
/// `~/.codex/auth.json` 与 `config.toml` 覆盖成了测试里的假值**
/// （`sk-test` / `https://api.example.com`）。运行根是临时目录也拦不住——
/// 这个函数看的是真实 HOME；备份则写进了随 `TempDir` 一起删掉的目录，无法恢复。
///
/// 现在主目录由调用方给：生产传用户的 HOME，测试传临时目录。
/// **测试因此不可能写到临时目录之外**——这是结构上的保证，不是纪律上的。
///
/// 取不到主目录时**保持原样**：拼一个猜测的路径比明确失败更糟。
#[must_use]
pub fn expand_home(path: &str, home: &Path) -> PathBuf {
    let trimmed = path.trim();
    let rest = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
        // 白名单同时接受 `$HOME/` 前缀，展开也必须认。
        .or_else(|| trimmed.strip_prefix("$HOME/"))
        .or_else(|| trimmed.strip_prefix("$HOME\\"));
    let Some(rest) = rest else {
        return PathBuf::from(trimmed);
    };
    home.join(rest)
}

/// 解析当前用户的 HOME。
///
/// **只该由生产入口调用一次**，然后把结果显式传下去。测试不要用这个函数——
/// 那正是上面那场事故的来源。
#[must_use]
pub fn real_home() -> Option<PathBuf> {
    aivc_infra::config::paths::real_home()
}

/// 读取一个配置文件的内容。
///
/// # 读取失败返回空串而不是错误
///
/// 配置预览属于**辅助信息**：文件不存在、没有读权限、WSL 暂不可用，都不该阻断
/// 供应商表单——用户仍然应该能编辑并保存。把它做成错误会让「第一次配置」变成
/// 不可能完成的操作。
///
/// # 路径白名单仍然强制
///
/// 读取同样受白名单约束：不加限制的话，这个命令就成了「读取任意文件」的入口
/// ——配置文件里通常有 API Key。
///
/// # Errors
///
/// 仅当路径不在白名单内时返回错误。
pub fn read_config_file(target_path: &str, home: &Path) -> Result<String, String> {
    aivc_core::shell::assert_allowed_config_path(target_path)
        .map_err(|error| format!("{error}"))?;
    let path = expand_home(target_path, home);
    Ok(std::fs::read_to_string(path).unwrap_or_default())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_name_replaces_path_separators_and_reserved_characters() {
        // 供应商名是用户输入，直接拼进备份路径就可能写到预期之外的位置。
        assert_eq!(safe_name("a/b"), "a_b");
        assert_eq!(safe_name("a\\b"), "a_b");
        assert_eq!(safe_name("a:b"), "a_b");
        for ch in ["*", "?", "\"", "<", ">", "|"] {
            assert_eq!(safe_name(&format!("a{ch}b")), "a_b", "{ch} 应当被替换");
        }
        assert!(safe_name("a\nb").contains('_'), "控制字符应当被替换");
    }

    #[test]
    fn safe_name_truncates_and_falls_back() {
        assert_eq!(safe_name("").chars().count(), "vendor".chars().count());
        assert_eq!(safe_name(""), "vendor");
        assert_eq!(safe_name(&"x".repeat(200)).chars().count(), SAFE_NAME_MAX);
        // 普通名字原样保留。
        assert_eq!(safe_name("我的供应商"), "我的供应商");
    }

    #[test]
    fn rendering_replaces_every_occurrence() {
        // 只替换第一处会让后续的明文占位符被写进配置文件——一个看起来像配置错误的
        // 怪异状态。
        let rendered = render_template(
            "{{API_KEY}} and {{API_KEY}} and {{BASE_URL}}",
            "sk-1",
            "https://x",
            "名字",
        );
        assert_eq!(rendered, "sk-1 and sk-1 and https://x");
        assert_eq!(render_template("{{VENDOR_NAME}}", "k", "u", "我的"), "我的");
    }

    #[test]
    fn rendering_leaves_unknown_placeholders_alone() {
        // 未知占位符保持原样：静默删除会让用户以为自己写错了。
        assert_eq!(render_template("{{OTHER}}", "k", "u", "n"), "{{OTHER}}");
    }

    #[test]
    fn the_codex_defaults_match_the_desktop_reference() {
        // 字节级一致：默认模板只在内容为空时使用，但它必须与现网相同，
        // 否则第一次启用就会把文件重排一遍。
        assert_eq!(
            default_config_content(ProviderId::Codex, "~/.codex/auth.json"),
            "{\n  \"OPENAI_API_KEY\": \"{{API_KEY}}\"\n}"
        );
        let toml = default_config_content(ProviderId::Codex, "~/.codex/config.toml");
        assert!(toml.starts_with("model_provider = \"akim\""));
        assert!(toml.contains("[model_providers.akim]"));
        assert!(toml.contains("wire_api = \"responses\""));
        assert!(toml.contains("base_url = \"{{BASE_URL}}\""));
    }

    #[test]
    fn the_gemini_and_claude_defaults_match_the_desktop_reference() {
        assert_eq!(
            default_config_content(ProviderId::Gemini, "~/.gemini/.env"),
            "GEMINI_API_KEY={{API_KEY}}\nGOOGLE_GEMINI_BASE_URL={{BASE_URL}}"
        );
        let gemini = default_config_content(ProviderId::Gemini, "~/.gemini/settings.json");
        assert!(gemini.contains("\"selectedType\": \"gemini-api-key\""));

        let claude = default_config_content(ProviderId::Claude, "~/.claude/settings.json");
        assert!(claude.contains("\"ANTHROPIC_BASE_URL\": \"{{BASE_URL}}\""));
        assert!(claude.contains("\"ANTHROPIC_AUTH_TOKEN\": \"{{API_KEY}}\""));
        assert!(claude.contains("\"theme\": \"dark\""));
    }

    #[test]
    fn qoder_has_no_default_template() {
        // 它的配置格式因版本而异，猜一个反而更糟。
        assert_eq!(
            default_config_content(ProviderId::Qoder, "~/.qoder/config.json"),
            ""
        );
    }

    #[test]
    fn a_path_without_a_known_suffix_has_no_default() {
        assert_eq!(
            default_config_content(ProviderId::Codex, "~/.codex/other.json"),
            ""
        );
    }

    /// **两条清单不许漂移**。
    ///
    /// [`default_config_targets`] 给路径、[`default_config_content`] 按后缀给内容。
    /// 它们是分开写的，因此完全可能对不上：给某个平台加一条路径却忘了加模板，
    /// 或者改模板后缀却忘了改路径。
    ///
    /// 对不上的后果很隐蔽——`normalize_config_content` 会返回**空串**，
    /// 而 `enable` 遇到空内容会中止写入（`配置文件内容为空，已中止写入`）。
    /// 也就是说用户点「启用」会失败，而错误信息指向一个他没写过的东西。
    /// 这条测试把那种情况挡在编译期之后、运行之前。
    #[test]
    fn every_default_target_has_a_default_template() {
        for provider in [
            ProviderId::Codex,
            ProviderId::Gemini,
            ProviderId::Claude,
            ProviderId::Qoder,
        ] {
            for path in default_config_targets(provider) {
                assert!(
                    !default_config_content(provider, path).trim().is_empty(),
                    "{provider:?} 的默认目标 {path} 没有对应模板"
                );
            }
        }
    }

    /// 有默认模板的平台必须给出目标路径——否则「启用」永远失败。
    #[test]
    fn platforms_with_templates_all_have_targets() {
        // Codex / Gemini / Claude 都有模板，因此都必须有路径。
        for provider in [ProviderId::Codex, ProviderId::Gemini, ProviderId::Claude] {
            assert!(
                !default_config_targets(provider).is_empty(),
                "{provider:?} 有默认模板却没有目标路径"
            );
        }
        // Qoder 两者都没有：不能只补一半。
        assert!(default_config_targets(ProviderId::Qoder).is_empty());
    }

    /// 组装出来的 `configs` 形状：路径与内容一一对应，且都是非空的。
    #[test]
    fn assembling_default_configs_pairs_paths_with_content() {
        let configs = default_vendor_configs(ProviderId::Codex);
        assert_eq!(configs.len(), 2);
        for (path, content) in &configs {
            assert!(path.starts_with("~/"), "{path} 应当是可展开的 HOME 路径");
            assert!(content.contains("{{API_KEY}}") || content.contains("{{BASE_URL}}"));
        }
        // 没有模板的平台组装出来是空的——调用方据此知道「这个平台写不了配置」。
        assert!(default_vendor_configs(ProviderId::Qoder).is_empty());
    }

    #[test]
    fn windows_separators_are_recognized_in_path_matching() {
        // Windows 路径用 `\` 分隔，不统一就会匹配不到默认模板。
        assert_eq!(
            default_config_content(ProviderId::Codex, "C:\\Users\\u\\.codex\\auth.json"),
            default_config_content(ProviderId::Codex, "~/.codex/auth.json")
        );
    }

    #[test]
    fn normalizing_keeps_non_empty_content() {
        // 用户写了内容就绝不覆盖成默认模板。
        assert_eq!(
            normalize_config_content(ProviderId::Codex, "~/.codex/auth.json", "我的内容"),
            "我的内容"
        );
        // 只有空白也算空。
        assert_eq!(
            normalize_config_content(ProviderId::Codex, "~/.codex/auth.json", "   "),
            default_config_content(ProviderId::Codex, "~/.codex/auth.json")
        );
    }

    #[test]
    fn the_backup_root_is_timestamped_and_sanitized() {
        let root = backup_root(
            Path::new("/data"),
            "我的/供应商",
            "2026-09-25T10:00:00.000Z",
        );
        let text = root.to_string_lossy();
        assert!(text.contains("vendor-backups"));
        // 名字里的分隔符被替换。
        assert!(text.contains("我的_供应商"));
        // 时间戳里的 `:` 与 `.` 在 Windows 上非法，必须替换。
        assert!(!text.contains(':'), "{text}");
        assert!(text.ends_with("2026-09-25T10-00-00-000Z"), "{text}");
    }

    #[test]
    fn writing_refuses_empty_content() {
        // 空内容会把用户配置**清空**——比写入失败严重得多，因为它看起来是成功的。
        let dir = tempfile::tempdir().expect("tempdir");
        // 主目录也给临时目录：这条测试在走到「内容为空」检查之前不该碰任何真实路径。
        let error = write_config_file(
            "~/.codex/auth.json",
            "   \n  ",
            dir.path(),
            "local",
            dir.path(),
        )
        .expect_err("必须拒绝");
        assert!(error.contains("内容为空"), "{error}");
    }

    #[test]
    fn writing_refuses_a_path_outside_the_whitelist() {
        // 路径白名单是最后一道关。
        let dir = tempfile::tempdir().expect("tempdir");
        let error = write_config_file("/etc/passwd", "内容", dir.path(), "local", dir.path())
            .expect_err("必须拒绝");
        assert!(!error.is_empty());
    }

    #[test]
    fn a_new_file_is_written_without_a_backup() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path();
        let target = home.join(".codex").join("auth.json");
        let path = target.to_string_lossy().into_owned();
        let outcome =
            write_file_with_backup(Path::new(&path), "内容", dir.path(), "local").expect("写入");
        assert!(outcome.created);
        assert_eq!(outcome.backup, None, "新建文件没有旧内容可备份");
        assert!(target.is_file());
    }

    #[test]
    fn an_existing_file_is_backed_up_before_being_overwritten() {
        // 目标里可能有用户自己写的内容，覆盖之后没有备份就找不回来了。
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path();
        let target = home.join(".codex").join("auth.json");
        std::fs::create_dir_all(target.parent().expect("父目录")).expect("目录");
        std::fs::write(&target, "旧内容").expect("写入");

        let path = target.to_string_lossy().into_owned();
        let outcome =
            write_file_with_backup(Path::new(&path), "新内容", dir.path(), "local").expect("写入");
        let backup = outcome.backup.expect("应当有备份");
        assert_eq!(std::fs::read_to_string(&backup).expect("读备份"), "旧内容");
        assert_eq!(std::fs::read_to_string(&target).expect("读目标"), "新内容");
        assert!(!outcome.created);
    }

    #[test]
    fn the_backup_sits_under_the_given_root() {
        // 备份跑到别的地方就等于没备份。
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path();
        let target = home.join("cfg").join("settings.json");
        std::fs::create_dir_all(target.parent().expect("父目录")).expect("目录");
        std::fs::write(&target, "旧").expect("写入");
        let root = home.join("backups");
        let outcome = write_file_with_backup(&target, "新", &root, "local").expect("写入");
        let backup = outcome.backup.expect("备份");
        assert!(
            Path::new(&backup).starts_with(&root),
            "备份应当在给定根目录下：{backup}"
        );
    }

    #[test]
    fn expand_home_handles_both_separators() {
        // 主目录**由调用方给**。测试一律给临时目录，因此不可能写到真实 HOME。
        let home = Path::new("/tmp/aivc-test-home");
        let expanded = expand_home("~/.codex/auth.json", home);
        assert_eq!(expanded, home.join(".codex/auth.json"));
        // 反斜杠形式同样认。
        assert_eq!(
            expand_home("~\\.codex\\auth.json", home),
            home.join(".codex\\auth.json")
        );
        // `$HOME/` 前缀也认——白名单接受它，展开就必须认。
        assert_eq!(
            expand_home("$HOME/.codex/auth.json", home),
            home.join(".codex/auth.json")
        );
        // 没有 `~` 的路径原样返回，不受主目录影响。
        assert_eq!(expand_home("/abs/path", home), PathBuf::from("/abs/path"));
    }

    #[test]
    fn reading_a_missing_file_yields_empty_content() {
        // 读取失败返回空串而不是错误：配置预览是辅助信息，不该阻断表单。
        //
        // 用一个**空的临时主目录**，这样读的必然是「不存在」而不是用户真实的文件。
        let home = tempfile::tempdir().expect("tempdir");
        let content =
            read_config_file("~/.codex/aivc-does-not-exist.json", home.path()).expect("应当成功");
        assert!(content.is_empty());
    }

    #[test]
    fn reading_outside_the_whitelist_is_refused() {
        // 不加限制的话，这个命令就成了「读取任意文件」的入口——配置文件里通常有 API Key。
        let home = tempfile::tempdir().expect("tempdir");
        for path in ["/etc/passwd", "~/.ssh/id_rsa", "C:/Windows/win.ini"] {
            assert!(
                read_config_file(path, home.path()).is_err(),
                "{path} 应当被拒绝"
            );
        }
    }

    #[test]
    fn writing_then_reading_round_trips() {
        // 端到端：写入后能读回同样的内容。
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("cfg").join("settings.json");
        write_file_with_backup(&target, "内容", dir.path(), "local").expect("写入");
        let read = std::fs::read_to_string(&target).expect("读取");
        assert_eq!(read, "内容");
    }
}
