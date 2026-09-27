//! Codex Skill 管理的纯逻辑：名称校验、frontmatter 解析、后缀处理。
//!
//! # 为什么先把这部分单独做出来
//!
//! Skill 管理涉及文件复制、原子换入、回收站、WSL 分支，这些都需要真实文件系统。
//! 但其中**判定逻辑**是纯的，而且是安全边界所在：
//!
//! - **名称校验**决定了一个 skill 名能不能拼进路径。放行 `../` 或路径分隔符，
//!   导入就能写到 `skills/` 之外。
//! - **frontmatter 解析**决定导入的内容是否被接受，边界最多（BOM、CRLF、引号、
//!   注释行、缺字段）。
//!
//! 把它们做成纯函数之后可以穷举测试，文件操作那一层就只剩「按校验过的值搬文件」。
//!
//! # 错误文案是契约
//!
//! 全部文案与现网 `skills.ts` 逐字一致（含全角冒号与句末句号）——它们会直接显示给用户。

use std::collections::BTreeMap;

/// `SKILL.md` 体积上限。
pub const MAX_SKILL_MD_BYTES: u64 = 2 * 1024 * 1024;

/// skill 名长度上限（按**字符**计，与现网的 `Array.from(name).length` 一致）。
pub const MAX_SKILL_NAME_CHARS: usize = 64;

/// 停用后缀。
pub const DISABLED_SUFFIX: &str = ".disabled";

/// skill 名称是否安全（可安全拼进路径）。
///
/// 拒绝三类：
///
/// 1. 空、超长（> 64 字符）、`.` 与 `..`；
/// 2. 路径分隔符 `\` `/`；
/// 3. Windows 保留字符 `<>:"|?*` 与全部控制字符。
///
/// 第 2、3 条是**安全边界**：放行它们，`skills/<name>` 就可能指向目录之外。
/// 第 3 条同时覆盖了 Windows 不允许出现在文件名里的字符，因此这个校验对两个平台都成立。
#[must_use]
pub fn is_safe_skill_name(name: &str) -> bool {
    if name.is_empty() || name.chars().count() > MAX_SKILL_NAME_CHARS {
        return false;
    }
    if name == "." || name == ".." {
        return false;
    }
    !name.chars().any(|ch| {
        matches!(
            ch,
            '\\' | '/' | '\0' | '<' | '>' | ':' | '"' | '|' | '?' | '*'
        ) || ch.is_control()
    })
}

/// 校验并归一 skill 名称。
///
/// # Errors
///
/// 名称非法时返回「skill name 无效。」。
pub fn require_safe_skill_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if !is_safe_skill_name(trimmed) {
        return Err("skill name 无效。".to_owned());
    }
    Ok(trimmed.to_owned())
}

/// 去掉 `.disabled` 后缀。
#[must_use]
pub fn strip_disabled_suffix(name: &str) -> String {
    name.strip_suffix(DISABLED_SUFFIX)
        .map_or_else(|| name.to_owned(), str::to_owned)
}

/// 由「是否启用」推导出目录名。
///
/// 停用**不是**删除，而是给目录加 `.disabled` 后缀——Codex 会忽略它，
/// 而用户随时可以恢复。这个设计意味着「启用/停用」是一次重命名，不涉及内容改动。
#[must_use]
pub fn skill_directory_name(name: &str, enabled: bool) -> String {
    let active = strip_disabled_suffix(name);
    if enabled {
        active
    } else {
        format!("{active}{DISABLED_SUFFIX}")
    }
}

/// 由回收站里的目录名还原出 skill 名。
///
/// 回收站条目带时间戳后缀（`<name>-<ts>`），还原时要剥掉。
/// 只剥**末尾的纯数字**——skill 名本身可能含 `-`，不能简单按 `-` 切分。
#[must_use]
pub fn trash_skill_active_name(name: &str) -> String {
    let Some(index) = name.rfind('-') else {
        return name.to_owned();
    };
    let (head, tail) = name.split_at(index);
    let digits = &tail[1..];
    if !digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit()) {
        head.to_owned()
    } else {
        name.to_owned()
    }
}

/// 解析 `SKILL.md` 的元数据。
///
/// # Errors
///
/// frontmatter 缺失、缺少 `name`/`description`、或名称非法时返回错误。
pub fn read_skill_metadata(content: &str) -> Result<SkillMetadata, String> {
    let fields = extract_frontmatter(content)?;
    let name = fields.get("name").map_or("", |value| value.trim());
    let description = fields.get("description").map_or("", |value| value.trim());

    if name.is_empty() {
        return Err("SKILL.md 缺少 frontmatter 字段：name。".to_owned());
    }
    if description.is_empty() {
        return Err("SKILL.md 缺少 frontmatter 字段：description。".to_owned());
    }
    if !is_safe_skill_name(name) {
        return Err("skill name 不能包含路径分隔符或控制字符，且长度不超过 64。".to_owned());
    }
    Ok(SkillMetadata {
        name: name.to_owned(),
        description: description.to_owned(),
    })
}

/// `SKILL.md` 的元数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillMetadata {
    /// skill 名称。
    pub name: String,
    /// 描述。
    pub description: String,
}

/// 提取 YAML frontmatter 的字段。
///
/// 支持的形态刻意很窄——只认「文件以 `---` 开头、到下一个 `---` 结束」以及
/// `键: 值` 两种结构。这不是通用 YAML 解析器：skill 元数据只有两个标量字段，
/// 引入完整 YAML 解析器会带来远超需求的依赖面与解析歧义。
///
/// # Errors
///
/// 内容不以 frontmatter 开头时返回错误。
pub fn extract_frontmatter(content: &str) -> Result<BTreeMap<String, String>, String> {
    // BOM 会让「以 `---` 开头」这个判断失败，因此先去掉。
    let normalized = content.strip_prefix('\u{feff}').unwrap_or(content);
    let Some(rest) = normalized
        .strip_prefix("---\n")
        .or_else(|| normalized.strip_prefix("---\r\n"))
    else {
        return Err("SKILL.md 必须以 YAML frontmatter 开头。".to_owned());
    };
    // 找结束分隔符：一个独占一行的 `---`。
    let mut fields = BTreeMap::new();
    for line in rest.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.trim_end() == "---" {
            return Ok(fields);
        }
        parse_field(line, &mut fields);
    }
    // 没找到结束分隔符：frontmatter 没有闭合。
    Err("SKILL.md 必须以 YAML frontmatter 开头。".to_owned())
}

/// 解析一行 `键: 值`。
fn parse_field(line: &str, fields: &mut BTreeMap<String, String>) {
    let trimmed = line.trim();
    // 空行与注释行跳过。
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return;
    }
    let Some((key, value)) = line.split_once(':') else {
        return;
    };
    let key = key.trim();
    // 键只允许字母、数字、下划线、连字符——与现网一致。
    if key.is_empty()
        || !key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return;
    }
    fields.insert(key.to_owned(), unquote_yaml_scalar(value));
}

/// 去掉 YAML 标量两端的引号。
///
/// 只处理**成对**的同类引号：`"a'` 这种不成对的保持原样，因为那更可能是值本身含引号，
/// 而不是用户想加引号。
#[must_use]
pub fn unquote_yaml_scalar(value: &str) -> String {
    let trimmed = value.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return trimmed[1..trimmed.len() - 1].to_owned();
        }
    }
    trimmed.to_owned()
}

/// skill 的 `SKILL.md` 相对路径。
#[must_use]
pub fn skill_file_name() -> &'static str {
    "SKILL.md"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_names_reject_path_separators() {
        // 这是安全边界：放行分隔符就能把导入写到 skills/ 之外。
        for name in ["a/b", "a\\b", "..", ".", "../etc", "a/../b"] {
            assert!(!is_safe_skill_name(name), "{name} 不该被接受");
        }
    }

    #[test]
    fn safe_names_reject_windows_reserved_characters() {
        for name in ["a<b", "a>b", "a:b", "a\"b", "a|b", "a?b", "a*b"] {
            assert!(!is_safe_skill_name(name), "{name} 不该被接受");
        }
        // 控制字符（含 NUL、换行、制表）同样拒绝。
        for name in ["a\0b", "a\nb", "a\tb", "a\u{1f}b"] {
            assert!(!is_safe_skill_name(name), "{name:?} 不该被接受");
        }
    }

    #[test]
    fn safe_names_reject_empty_and_over_long() {
        assert!(!is_safe_skill_name(""));
        assert!(is_safe_skill_name(&"字".repeat(64)));
        assert!(!is_safe_skill_name(&"字".repeat(65)));
        // 长度按字符计，不按字节。
        assert!(is_safe_skill_name(&"a".repeat(64)));
        assert!(!is_safe_skill_name(&"a".repeat(65)));
    }

    #[test]
    fn safe_names_accept_ordinary_names() {
        for name in ["my-skill", "my_skill", "MySkill", "技能", "a.b", "a b"] {
            assert!(is_safe_skill_name(name), "{name} 应当被接受");
        }
    }

    #[test]
    fn require_safe_name_trims_and_reports_the_documented_message() {
        assert_eq!(
            require_safe_skill_name("  my-skill  ").expect("ok"),
            "my-skill"
        );
        assert_eq!(
            require_safe_skill_name("../evil").expect_err("必须拒绝"),
            "skill name 无效。"
        );
        assert_eq!(
            require_safe_skill_name("").expect_err("必须拒绝"),
            "skill name 无效。"
        );
    }

    #[test]
    fn the_disabled_suffix_is_added_and_stripped() {
        assert_eq!(skill_directory_name("my-skill", false), "my-skill.disabled");
        assert_eq!(skill_directory_name("my-skill", true), "my-skill");
        assert_eq!(strip_disabled_suffix("my-skill.disabled"), "my-skill");
        assert_eq!(strip_disabled_suffix("my-skill"), "my-skill");
        // 幂等：对已停用的名字再算一次不会叠加后缀。
        assert_eq!(
            skill_directory_name("my-skill.disabled", false),
            "my-skill.disabled"
        );
        assert_eq!(skill_directory_name("my-skill.disabled", true), "my-skill");
    }

    #[test]
    fn trash_names_lose_only_the_trailing_timestamp() {
        assert_eq!(
            trash_skill_active_name("my-skill-1758794400000"),
            "my-skill"
        );
        // 名字本身含连字符时要保留。
        assert_eq!(trash_skill_active_name("a-b-c-123"), "a-b-c");
        // 末尾不是纯数字时不动。
        assert_eq!(trash_skill_active_name("my-skill"), "my-skill");
        assert_eq!(trash_skill_active_name("my-skill-abc"), "my-skill-abc");
        // 末尾是空的连字符也不动。
        assert_eq!(trash_skill_active_name("my-skill-"), "my-skill-");
        // 没有连字符。
        assert_eq!(trash_skill_active_name("skill"), "skill");
    }

    #[test]
    fn frontmatter_is_parsed_with_name_and_description() {
        let content = "---\nname: my-skill\ndescription: 一个示例技能\n---\n\n# 正文\n";
        let metadata = read_skill_metadata(content).expect("ok");
        assert_eq!(metadata.name, "my-skill");
        assert_eq!(metadata.description, "一个示例技能");
    }

    #[test]
    fn a_bom_does_not_break_the_frontmatter_check() {
        // 编辑器常给文件加 BOM；不处理会让「以 --- 开头」判断失败。
        let content = "\u{feff}---\nname: a\ndescription: b\n---\n";
        assert_eq!(read_skill_metadata(content).expect("ok").name, "a");
    }

    #[test]
    fn crlf_line_endings_are_accepted() {
        let content = "---\r\nname: a\r\ndescription: b\r\n---\r\n";
        let metadata = read_skill_metadata(content).expect("ok");
        assert_eq!(metadata.name, "a");
        assert_eq!(metadata.description, "b");
    }

    #[test]
    fn quoted_scalars_lose_their_quotes() {
        let content = "---\nname: \"my-skill\"\ndescription: 'a description'\n---\n";
        let metadata = read_skill_metadata(content).expect("ok");
        assert_eq!(metadata.name, "my-skill");
        assert_eq!(metadata.description, "a description");
    }

    #[test]
    fn unmatched_quotes_are_kept() {
        // 不成对的引号更可能是值本身含引号，而不是用户想加引号。
        assert_eq!(unquote_yaml_scalar("\"a'"), "\"a'");
        assert_eq!(unquote_yaml_scalar("\""), "\"");
        assert_eq!(unquote_yaml_scalar("plain"), "plain");
        assert_eq!(unquote_yaml_scalar("  spaced  "), "spaced");
    }

    #[test]
    fn comment_and_blank_lines_are_skipped() {
        let content = "---\n# 这是注释\n\nname: a\ndescription: b\n---\n";
        assert_eq!(read_skill_metadata(content).expect("ok").name, "a");
    }

    #[test]
    fn a_missing_frontmatter_is_rejected_with_the_documented_message() {
        let error = read_skill_metadata("# 直接就是正文\n").expect_err("必须拒绝");
        assert_eq!(error, "SKILL.md 必须以 YAML frontmatter 开头。");
    }

    #[test]
    fn an_unterminated_frontmatter_is_rejected() {
        // 没有闭合的 `---`：这是损坏的文件，不能当成「元数据到文件末尾」。
        let error = read_skill_metadata("---\nname: a\ndescription: b\n").expect_err("必须拒绝");
        assert_eq!(error, "SKILL.md 必须以 YAML frontmatter 开头。");
    }

    #[test]
    fn missing_fields_report_their_own_messages() {
        assert_eq!(
            read_skill_metadata("---\ndescription: b\n---\n").expect_err("必须拒绝"),
            "SKILL.md 缺少 frontmatter 字段：name。"
        );
        assert_eq!(
            read_skill_metadata("---\nname: a\n---\n").expect_err("必须拒绝"),
            "SKILL.md 缺少 frontmatter 字段：description。"
        );
        // 只有空白也算缺失。
        assert_eq!(
            read_skill_metadata("---\nname:   \ndescription: b\n---\n").expect_err("必须拒绝"),
            "SKILL.md 缺少 frontmatter 字段：name。"
        );
    }

    #[test]
    fn an_unsafe_name_in_the_file_is_rejected() {
        let error =
            read_skill_metadata("---\nname: ../evil\ndescription: b\n---\n").expect_err("必须拒绝");
        assert_eq!(
            error,
            "skill name 不能包含路径分隔符或控制字符，且长度不超过 64。"
        );
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let content = "---\nname: a\ndescription: b\nversion: 1\n---\n";
        let metadata = read_skill_metadata(content).expect("ok");
        assert_eq!(metadata.name, "a");
    }

    #[test]
    fn a_line_without_a_colon_is_ignored() {
        let content = "---\nname: a\ndescription: b\nthis line has no colon\n---\n";
        assert_eq!(read_skill_metadata(content).expect("ok").name, "a");
    }

    #[test]
    fn a_key_with_illegal_characters_is_ignored() {
        // 键只允许字母数字下划线连字符；带空格的键不是合法字段。
        let fields = extract_frontmatter("---\nmy key: value\nname: a\n---\n").expect("ok");
        assert!(!fields.contains_key("my key"));
        assert_eq!(fields.get("name").map(String::as_str), Some("a"));
    }

    #[test]
    fn the_skill_file_name_is_stable() {
        assert_eq!(skill_file_name(), "SKILL.md");
        assert_eq!(MAX_SKILL_MD_BYTES, 2 * 1024 * 1024);
    }
}
