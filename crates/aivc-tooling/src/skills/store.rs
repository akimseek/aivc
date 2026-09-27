//! Skill 的文件操作：列表、启停、删除、恢复、清除。
//!
//! # 目录布局
//!
//! ```text
//! $CODEX_HOME/skills/<name>/SKILL.md          活跃
//! $CODEX_HOME/skills/<name>.disabled/SKILL.md 停用
//! $CODEX_HOME/.visual-console-trash/skills/<name>-<时间戳>/SKILL.md  回收站
//! ```
//!
//! # 两个容易搞错的点
//!
//! 1. **停用是一次重命名**，不是删除内容。因此「启用/停用」不会丢数据，
//!    而且必须幂等——对已停用的 skill 再停用一次不该变成 `x.disabled.disabled`。
//! 2. **回收站条目带时间戳后缀**（`<name>-<ts>`），恢复时要剥掉。只剥末尾的**纯数字**：
//!    skill 名本身可能含 `-`（`a-b-c`），按 `-` 切分会切错。
//!
//! # 读不出元数据的目录会被忽略
//!
//! 目录存在但 `SKILL.md` 缺失、解析失败或 frontmatter 非法时，该条目**不出现**在列表里
//! （而不是抛错）。原因：`skills/` 是用户可手改的目录，一个坏目录不该让整个列表打不开。

use std::path::{Path, PathBuf};

use crate::skills::core::{
    DISABLED_SUFFIX, SkillMetadata, extract_frontmatter, is_safe_skill_name, skill_directory_name,
    strip_disabled_suffix, trash_skill_active_name,
};

/// 一条已安装的 skill。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledSkill {
    /// 展示名（frontmatter 的 `name`，缺失时退回目录名）。
    pub name: String,
    /// 描述（frontmatter 的 `description`，缺失时为空串）。
    pub description: String,
    /// skill 目录的绝对路径。
    pub path: String,
    /// 是否启用。
    pub enabled: bool,
    /// 实际目录名（可能与展示名不同：带 `.disabled` 后缀或含时间戳）。
    pub source_name: String,
}

/// 回收站子目录名。
pub const TRASH_DIR: &str = ".visual-console-trash";

/// 活跃 skill 目录。
#[must_use]
pub fn active_dir(codex_home: &str) -> PathBuf {
    Path::new(codex_home).join("skills")
}

/// 回收站 skill 目录。
#[must_use]
pub fn trash_dir(codex_home: &str) -> PathBuf {
    Path::new(codex_home).join(TRASH_DIR).join("skills")
}

/// 列出活跃 skill。
#[must_use]
pub fn list_skills(codex_home: &str) -> Vec<InstalledSkill> {
    list_in(&active_dir(codex_home))
}

/// 列出回收站 skill。
#[must_use]
pub fn list_trash_skills(codex_home: &str) -> Vec<InstalledSkill> {
    list_in(&trash_dir(codex_home))
}

/// 列出某个目录下的全部 skill，按名称排序。
fn list_in(root: &Path) -> Vec<InstalledSkill> {
    let Ok(entries) = std::fs::read_dir(root) else {
        // 目录不存在视为「没有 skill」，而不是错误。
        return Vec::new();
    };
    let mut skills: Vec<InstalledSkill> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| read_installed(&entry.path(), &entry.file_name().to_string_lossy()))
        .collect();
    skills.sort_by(|left, right| left.name.cmp(&right.name));
    skills
}

/// 读取一条已安装的 skill；元数据不可用时返回 [`None`]。
#[must_use]
pub fn read_installed(skill_path: &Path, directory_name: &str) -> Option<InstalledSkill> {
    let enabled = !directory_name.ends_with(DISABLED_SUFFIX);
    let display_name = strip_disabled_suffix(directory_name);
    let content = std::fs::read_to_string(skill_path.join("SKILL.md")).ok()?;
    // 用 `extract_frontmatter` 而不是 `read_skill_metadata`：后者要求两个字段齐全，
    // 而列表展示允许字段缺失（退回目录名 / 空描述）。两者是不同严格度，PR 不能混用。
    let fields = extract_frontmatter(&content).ok()?;
    let name = fields
        .get("name")
        .map_or(display_name.as_str(), |value| value.trim());
    let description = fields.get("description").map_or("", |value| value.trim());
    Some(InstalledSkill {
        name: if name.is_empty() {
            display_name
        } else {
            name.to_owned()
        },
        description: description.to_owned(),
        path: skill_path.to_string_lossy().into_owned(),
        enabled,
        source_name: directory_name.to_owned(),
    })
}

/// 启用或停用一条 skill。
///
/// 幂等：目标目录名与当前相同则直接返回成功。
///
/// # Errors
///
/// 名称非法、源目录不存在或重命名失败时返回错误。
pub fn set_enabled(codex_home: &str, skill_name: &str, enabled: bool) -> Result<(), String> {
    let safe = crate::skills::core::require_safe_skill_name(skill_name)?;
    // 传入的名字可能已经带 `.disabled`（渲染层直接用 sourceName 调），
    // 因此先归一成「活跃名」再算目标名。
    let active = strip_disabled_suffix(&safe);
    let from = active_dir(codex_home).join(skill_directory_name(&active, !enabled));
    let to = active_dir(codex_home).join(skill_directory_name(&active, enabled));
    if from == to {
        return Ok(());
    }
    if !from.is_dir() {
        // 源目录不存在，但目标已经在期望位置上——说明它本来就处于目标状态。
        // 这是**幂等**的关键：对已启用的 skill 再启用一次不该报错，
        // 否则界面上的重复点击会变成「操作失败」。
        if to.is_dir() {
            return Ok(());
        }
        return Err(format!("skill 不存在：{active}"));
    }
    std::fs::rename(&from, &to).map_err(|error| format!("切换 skill 状态失败：{error}"))
}

/// 把一条 skill 移入回收站。
///
/// 目标名带时间戳后缀，因此**同名 skill 可以反复删除**而不会互相覆盖。
///
/// # Errors
///
/// 名称非法、源目录不存在或移动失败时返回错误。
pub fn delete_skill(codex_home: &str, skill_name: &str, now_ms: i64) -> Result<String, String> {
    let safe = crate::skills::core::require_safe_skill_name(skill_name)?;
    let from = active_dir(codex_home).join(&safe);
    if !from.is_dir() {
        return Err(format!("skill 不存在：{safe}"));
    }
    let trash = trash_dir(codex_home);
    std::fs::create_dir_all(&trash).map_err(|error| format!("创建回收站失败：{error}"))?;
    let target = trash.join(format!("{safe}-{now_ms}"));
    std::fs::rename(&from, &target).map_err(|error| format!("移入回收站失败：{error}"))?;
    Ok(target.to_string_lossy().into_owned())
}

/// 从回收站恢复一条 skill。
///
/// # Errors
///
/// 名称非法、回收站条目不存在、或活跃目录已有同名 skill 时返回错误。
pub fn restore_skill(codex_home: &str, trash_name: &str) -> Result<String, String> {
    let safe = crate::skills::core::require_safe_skill_name(trash_name)?;
    let from = trash_dir(codex_home).join(&safe);
    if !from.is_dir() {
        return Err(format!("回收站中不存在：{safe}"));
    }
    // 恢复时剥掉时间戳后缀。
    let active_name = skill_directory_name(&trash_skill_active_name(&safe), true);
    let to = active_dir(codex_home).join(&active_name);
    if to.exists() {
        // 拒绝覆盖：静默覆盖会丢掉用户已有的 skill。
        return Err(format!("目标位置已存在同名 skill：{active_name}"));
    }
    std::fs::create_dir_all(active_dir(codex_home))
        .map_err(|error| format!("创建 skills 目录失败：{error}"))?;
    std::fs::rename(&from, &to).map_err(|error| format!("恢复 skill 失败：{error}"))?;
    Ok(to.to_string_lossy().into_owned())
}

/// 从回收站彻底删除一条 skill。
///
/// # Errors
///
/// 名称非法、条目不存在或删除失败时返回错误。
pub fn purge_skill(codex_home: &str, trash_name: &str) -> Result<String, String> {
    let safe = crate::skills::core::require_safe_skill_name(trash_name)?;
    let target = trash_dir(codex_home).join(&safe);
    if !target.is_dir() {
        return Err(format!("回收站中不存在：{safe}"));
    }
    std::fs::remove_dir_all(&target).map_err(|error| format!("删除 skill 失败：{error}"))?;
    Ok(target.to_string_lossy().into_owned())
}

/// skill 目录是否已存在（导入前检查用）。
#[must_use]
pub fn skill_exists(codex_home: &str, skill_name: &str) -> bool {
    if !is_safe_skill_name(skill_name) {
        return false;
    }
    let active = active_dir(codex_home).join(skill_name);
    let disabled = active_dir(codex_home).join(format!("{skill_name}{DISABLED_SUFFIX}"));
    active.exists() || disabled.exists()
}

/// 由 `SKILL.md` 内容推出导入目标目录名与元数据。
///
/// # Errors
///
/// frontmatter 缺失、字段不全或名称非法时返回错误。
pub fn plan_destination(
    codex_home: &str,
    content: &str,
) -> Result<(PathBuf, SkillMetadata), String> {
    let metadata = crate::skills::core::read_skill_metadata(content)?;
    let destination = active_dir(codex_home).join(&metadata.name);
    Ok((destination, metadata))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 建一个 skill 目录（含 `SKILL.md`）。
    fn write_skill(root: &Path, dir_name: &str, name: &str, description: &str) {
        let dir = root.join("skills").join(dir_name);
        std::fs::create_dir_all(&dir).expect("目录");
        let content = format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n");
        std::fs::write(dir.join("SKILL.md"), content).expect("写入");
    }

    #[test]
    fn an_empty_home_has_no_skills() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_string_lossy().into_owned();
        assert!(list_skills(&home).is_empty());
        assert!(list_trash_skills(&home).is_empty());
    }

    #[test]
    fn skills_are_listed_with_their_metadata() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_string_lossy().into_owned();
        write_skill(dir.path(), "beta", "beta", "第二个");
        write_skill(dir.path(), "alpha", "alpha", "第一个");

        let skills = list_skills(&home);
        assert_eq!(skills.len(), 2);
        // 按名称排序。
        assert_eq!(skills[0].name, "alpha");
        assert_eq!(skills[1].name, "beta");
        assert_eq!(skills[0].description, "第一个");
        assert!(skills[0].enabled);
        assert_eq!(skills[0].source_name, "alpha");
        assert!(skills[0].path.contains("alpha"));
    }

    #[test]
    fn a_disabled_directory_shows_up_as_disabled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_string_lossy().into_owned();
        write_skill(dir.path(), "alpha.disabled", "alpha", "描述");

        let skills = list_skills(&home);
        assert_eq!(skills.len(), 1);
        assert!(!skills[0].enabled, "带 .disabled 后缀的应当是停用状态");
        // 展示名是剥掉后缀的。
        assert_eq!(skills[0].name, "alpha");
        // 但目录名保留后缀，供启停时定位。
        assert_eq!(skills[0].source_name, "alpha.disabled");
    }

    #[test]
    fn a_directory_without_skill_md_is_ignored() {
        // `skills/` 是用户可手改的目录，一个坏目录不该让整个列表打不开。
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_string_lossy().into_owned();
        std::fs::create_dir_all(dir.path().join("skills").join("broken")).expect("目录");
        write_skill(dir.path(), "good", "good", "好的");

        let skills = list_skills(&home);
        assert_eq!(skills.len(), 1, "坏目录应当被忽略而不是让列表失败");
        assert_eq!(skills[0].name, "good");
    }

    #[test]
    fn a_directory_with_bad_frontmatter_is_ignored() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_string_lossy().into_owned();
        let bad = dir.path().join("skills").join("bad");
        std::fs::create_dir_all(&bad).expect("目录");
        std::fs::write(bad.join("SKILL.md"), "没有 frontmatter\n").expect("写入");
        write_skill(dir.path(), "good", "good", "好的");
        assert_eq!(list_skills(&home).len(), 1);
    }

    #[test]
    fn an_empty_frontmatter_name_falls_back_to_the_directory_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_string_lossy().into_owned();
        let target = dir.path().join("skills").join("dirname");
        std::fs::create_dir_all(&target).expect("目录");
        std::fs::write(target.join("SKILL.md"), "---\nname:   \n---\n").expect("写入");
        let skills = list_skills(&home);
        assert_eq!(skills[0].name, "dirname", "名称为空时退回目录名");
        assert!(skills[0].description.is_empty());
    }

    #[test]
    fn files_in_the_skills_directory_are_not_listed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_string_lossy().into_owned();
        std::fs::create_dir_all(dir.path().join("skills")).expect("目录");
        std::fs::write(dir.path().join("skills").join("readme.md"), "x").expect("写入");
        assert!(list_skills(&home).is_empty());
    }

    #[test]
    fn enabling_and_disabling_renames_the_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_string_lossy().into_owned();
        write_skill(dir.path(), "alpha", "alpha", "描述");

        set_enabled(&home, "alpha", false).expect("停用");
        assert!(!active_dir(&home).join("alpha").exists());
        assert!(active_dir(&home).join("alpha.disabled").is_dir());
        assert!(!list_skills(&home)[0].enabled);

        set_enabled(&home, "alpha", true).expect("启用");
        assert!(active_dir(&home).join("alpha").is_dir());
        assert!(list_skills(&home)[0].enabled);
    }

    #[test]
    fn enabling_is_idempotent() {
        // 对已启用的 skill 再启用一次不该产生 alpha.disabled.disabled 这类目录。
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_string_lossy().into_owned();
        write_skill(dir.path(), "alpha", "alpha", "描述");
        set_enabled(&home, "alpha", true).expect("启用");
        set_enabled(&home, "alpha", true).expect("再次启用");
        set_enabled(&home, "alpha", false).expect("停用");
        set_enabled(&home, "alpha", false).expect("再次停用");
        assert!(active_dir(&home).join("alpha.disabled").is_dir());
        assert!(!active_dir(&home).join("alpha.disabled.disabled").exists());
    }

    #[test]
    fn the_disabled_name_can_be_passed_in_directly() {
        // 渲染层可能直接用 sourceName（带后缀）调启停。
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_string_lossy().into_owned();
        write_skill(dir.path(), "alpha", "alpha", "描述");
        set_enabled(&home, "alpha.disabled", false).expect("停用");
        assert!(active_dir(&home).join("alpha.disabled").is_dir());
    }

    #[test]
    fn enabling_a_missing_skill_reports_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_string_lossy().into_owned();
        let error = set_enabled(&home, "missing", false).expect_err("必须失败");
        assert!(error.contains("skill 不存在"), "{error}");
    }

    #[test]
    fn an_invalid_name_is_rejected_before_touching_the_filesystem() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_string_lossy().into_owned();
        for name in ["../evil", "a/b", ""] {
            let error = set_enabled(&home, name, false).expect_err("必须拒绝");
            assert_eq!(error, "skill name 无效。");
        }
    }

    #[test]
    fn deleting_moves_the_skill_to_the_trash_with_a_timestamp() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_string_lossy().into_owned();
        write_skill(dir.path(), "alpha", "alpha", "描述");

        let moved = delete_skill(&home, "alpha", 1_758_794_400_000).expect("删除");
        assert!(moved.ends_with("alpha-1758794400000"), "{moved}");
        assert!(!active_dir(&home).join("alpha").exists());
        assert_eq!(list_skills(&home).len(), 0);
        assert_eq!(list_trash_skills(&home).len(), 1);
    }

    #[test]
    fn the_same_skill_can_be_deleted_twice() {
        // 时间戳后缀让同名条目不会互相覆盖。
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_string_lossy().into_owned();
        write_skill(dir.path(), "alpha", "alpha", "一");
        delete_skill(&home, "alpha", 1).expect("第一次");
        write_skill(dir.path(), "alpha", "alpha", "二");
        delete_skill(&home, "alpha", 2).expect("第二次");
        assert_eq!(list_trash_skills(&home).len(), 2);
    }

    #[test]
    fn deleting_a_missing_skill_reports_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_string_lossy().into_owned();
        let error = delete_skill(&home, "missing", 0).expect_err("必须失败");
        assert!(error.contains("skill 不存在"), "{error}");
    }

    #[test]
    fn restoring_strips_the_timestamp_and_returns_the_original_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_string_lossy().into_owned();
        write_skill(dir.path(), "a-b", "a-b", "含连字符的名字");
        delete_skill(&home, "a-b", 1_758_794_400_000).expect("删除");

        let restored = restore_skill(&home, "a-b-1758794400000").expect("恢复");
        assert!(restored.ends_with("a-b"), "{restored}");
        assert!(active_dir(&home).join("a-b").is_dir());
        assert_eq!(list_skills(&home)[0].name, "a-b");
    }

    #[test]
    fn restoring_refuses_to_overwrite_an_existing_skill() {
        // 静默覆盖会丢掉用户已有的 skill。
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_string_lossy().into_owned();
        write_skill(dir.path(), "alpha", "alpha", "旧");
        delete_skill(&home, "alpha", 1).expect("删除");
        write_skill(dir.path(), "alpha", "alpha", "新的");

        let error = restore_skill(&home, "alpha-1").expect_err("必须拒绝");
        assert!(error.contains("已存在同名 skill"), "{error}");
        // 已有的那份必须原样保留。
        assert_eq!(list_skills(&home)[0].description, "新的");
    }

    #[test]
    fn restoring_a_missing_trash_entry_reports_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_string_lossy().into_owned();
        let error = restore_skill(&home, "alpha-1").expect_err("必须失败");
        assert!(error.contains("回收站中不存在"), "{error}");
    }

    #[test]
    fn purging_removes_the_directory_entirely() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_string_lossy().into_owned();
        write_skill(dir.path(), "alpha", "alpha", "描述");
        delete_skill(&home, "alpha", 7).expect("删除");

        let purged = purge_skill(&home, "alpha-7").expect("清除");
        assert!(purged.contains("alpha-7"));
        assert!(list_trash_skills(&home).is_empty());
    }

    #[test]
    fn purging_a_missing_entry_reports_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_string_lossy().into_owned();
        assert!(purge_skill(&home, "nope-1").is_err());
    }

    #[test]
    fn existence_check_covers_both_states() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_string_lossy().into_owned();
        assert!(!skill_exists(&home, "alpha"));
        write_skill(dir.path(), "alpha", "alpha", "描述");
        assert!(skill_exists(&home, "alpha"));
        set_enabled(&home, "alpha", false).expect("停用");
        assert!(skill_exists(&home, "alpha"), "停用状态下也算存在");
        // 非法名称一律视为不存在，而不是去拼路径。
        assert!(!skill_exists(&home, "../evil"));
    }

    #[test]
    fn the_import_destination_comes_from_the_frontmatter_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_string_lossy().into_owned();
        let content = "---\nname: my-skill\ndescription: 描述\n---\n";
        let (destination, metadata) = plan_destination(&home, content).expect("ok");
        assert_eq!(metadata.name, "my-skill");
        assert!(
            destination.ends_with("skills/my-skill") || destination.ends_with("skills\\my-skill")
        );
    }

    #[test]
    fn an_invalid_import_is_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_string_lossy().into_owned();
        assert!(plan_destination(&home, "# 没有 frontmatter\n").is_err());
        assert!(
            plan_destination(&home, "---\nname: ../evil\ndescription: d\n---\n").is_err(),
            "路径分隔符必须被拒绝"
        );
    }

    #[test]
    fn the_trash_path_is_inside_the_codex_home() {
        // 回收站必须在 codex home 之内：跑到外面就等于写到用户没预期的地方。
        let trash = trash_dir("/home/u/.codex");
        assert!(trash.starts_with("/home/u/.codex"));
        assert!(trash.ends_with("skills"));
    }
}
