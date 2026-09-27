//! Skill 导入：临时目录 → 备份 → 原子换入 → 失败回滚。
//!
//! # 为什么不能直接往目标位置写
//!
//! 复制一个 skill 目录可能要移动几十个文件、耗时可观。中途失败（磁盘满、权限、
//! 用户取消）会在 `skills/<name>` 留下**半个 skill**——而半个 skill 比没有更糟：
//! 它看起来像装好了，Codex 会尝试加载它，报出的错与真实原因无关。
//!
//! 因此流程是：**先在临时目录里拼完整，再一次性换过去**。
//!
//! ```text
//! 1. 复制到 .import-<唯一后缀>          ← 目标位置此刻完全没动
//! 2. 若有旧版本：目标 → .backup-<唯一后缀>
//! 3. 临时目录 → 目标                    ← 这一步是原子 rename
//! 4. 删除备份
//! ```
//!
//! # 失败时为什么要回滚
//!
//! 第 3 步失败时，旧版本已经被挪到备份位置了。若不挪回去，用户会看到
//! 「导入失败，而且原来的 skill 也没了」——比单纯失败严重得多。
//!
//! # 复制时排除的两个目录
//!
//! `.git` 与 `node_modules`：它们是开发产物，体积可能远大于 skill 本身，
//! 而且与 skill 的运行无关。排除它们是有意的——现网同样如此。
//!
//! # WSL 未覆盖
//!
//! 本模块只处理本机文件系统。WSL 目标的导入需要走 `wsl.exe`，
//! 尚未实现（会显式报错而不是悄悄写到本机路径）。

use std::path::{Path, PathBuf};

use crate::skills::core::{MAX_SKILL_MD_BYTES, read_skill_metadata};
use crate::skills::store::active_dir;

/// 复制时排除的目录名。
pub const EXCLUDED_DIR_NAMES: [&str; 2] = [".git", "node_modules"];

/// 导入源的形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportKind {
    /// 单个 Markdown 文件（会被放进 `<目标>/SKILL.md`）。
    File,
    /// 一个 skill 目录。
    Directory,
}

/// 一次导入请求。
#[derive(Debug, Clone)]
pub struct ImportRequest {
    /// 源路径。
    pub source: PathBuf,
    /// `codex_home`。
    pub codex_home: String,
    /// 源形态。
    pub kind: ImportKind,
    /// 已存在时是否覆盖。
    pub overwrite: bool,
    /// 临时目录与备份目录的唯一后缀。
    ///
    /// 由调用方注入而不是在这里读时钟：测试要能构造「同一次导入的两次尝试」
    /// 而不依赖真实时间。
    pub suffix: String,
}

/// 导入结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportOutcome {
    /// skill 名称（来自 frontmatter）。
    pub skill_name: String,
    /// 目标目录。
    pub destination_path: String,
}

/// 执行导入。
///
/// # Errors
///
/// 源不存在、形态不符、`SKILL.md` 缺失或过大、frontmatter 非法、
/// 目标已存在且未允许覆盖、复制或换入失败时返回错误。
pub fn import_skill(request: &ImportRequest) -> Result<ImportOutcome, String> {
    let source_stat =
        std::fs::symlink_metadata(&request.source).map_err(|_| "选择的路径不存在。".to_owned())?;
    // 形态与入参必须一致：用户选的是目录但按文件处理会读到奇怪的东西。
    match request.kind {
        ImportKind::Directory if !source_stat.is_dir() => {
            return Err("只能导入 Markdown 文件或 skill 目录。".to_owned());
        }
        ImportKind::File if !source_stat.is_file() => {
            return Err("只能导入 Markdown 文件或 skill 目录。".to_owned());
        }
        _ => {}
    }
    if request.kind == ImportKind::File && !has_markdown_extension(&request.source) {
        return Err("请选择 Markdown 文件。".to_owned());
    }

    // 文件形态下源就是 SKILL.md；目录形态下是 `<源>/SKILL.md`。
    let skill_file = match request.kind {
        ImportKind::File => request.source.clone(),
        ImportKind::Directory => request.source.join("SKILL.md"),
    };
    let metadata = read_skill_file(&skill_file)?;

    let parent = active_dir(&request.codex_home);
    let destination = parent.join(&metadata.name);
    let exists = destination.exists() || disabled_sibling(&parent, &metadata.name).exists();
    if exists && !request.overwrite {
        return Err(format!("skill 已存在：{}", metadata.name));
    }

    std::fs::create_dir_all(&parent).map_err(|error| format!("创建 skills 目录失败：{error}"))?;
    let temp = parent.join(format!(".import-{}", request.suffix));
    let backup = parent.join(format!(".backup-{}-{}", metadata.name, request.suffix));

    // 第一步：把内容拼进临时目录。目标位置此刻完全没动。
    let staged = stage(request, &temp);
    if let Err(error) = staged {
        let _ = std::fs::remove_dir_all(&temp);
        return Err(error);
    }

    // 第二步与第三步：换入。失败则回滚。
    match swap_into_place(&destination, &temp, &backup, exists) {
        Ok(()) => Ok(ImportOutcome {
            skill_name: metadata.name,
            destination_path: destination.to_string_lossy().into_owned(),
        }),
        Err(error) => {
            let _ = std::fs::remove_dir_all(&temp);
            // 旧版本已经在备份位置，必须挪回去——否则用户会看到
            //「导入失败，而且原来的 skill 也没了」。
            if exists && !destination.exists() && backup.exists() {
                let _ = std::fs::rename(&backup, &destination);
            }
            Err(error)
        }
    }
}

/// 把源内容复制到临时目录。
fn stage(request: &ImportRequest, temp: &Path) -> Result<(), String> {
    match request.kind {
        ImportKind::Directory => {
            std::fs::create_dir_all(temp).map_err(|error| format!("创建临时目录失败：{error}"))?;
            copy_directory(&request.source, temp)
        }
        ImportKind::File => {
            std::fs::create_dir_all(temp).map_err(|error| format!("创建临时目录失败：{error}"))?;
            std::fs::copy(&request.source, temp.join("SKILL.md"))
                .map(|_| ())
                .map_err(|error| format!("复制 SKILL.md 失败：{error}"))
        }
    }
}

/// 换入：备份旧版本 → 临时目录就位 → 删除备份。
fn swap_into_place(
    destination: &Path,
    temp: &Path,
    backup: &Path,
    had_previous: bool,
) -> Result<(), String> {
    if had_previous {
        // 已停用的同名 skill 也要一并让位，否则换入后会出现「一份启用一份停用」。
        let parent = destination.parent().unwrap_or(Path::new("."));
        let name = destination
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        let disabled = parent.join(format!("{name}.disabled"));
        if disabled.exists() {
            std::fs::rename(&disabled, backup)
                .map_err(|error| format!("备份已停用的 skill 失败：{error}"))?;
        } else if destination.exists() {
            std::fs::rename(destination, backup)
                .map_err(|error| format!("备份旧版本失败：{error}"))?;
        }
    }
    std::fs::rename(temp, destination).map_err(|error| format!("导入 skill 失败：{error}"))?;
    if backup.exists() {
        let _ = std::fs::remove_dir_all(backup);
    }
    Ok(())
}

/// 同名但停用的目录。
fn disabled_sibling(parent: &Path, name: &str) -> PathBuf {
    parent.join(format!("{name}.disabled"))
}

/// 递归复制目录，排除 [`EXCLUDED_DIR_NAMES`]。
fn copy_directory(source: &Path, destination: &Path) -> Result<(), String> {
    let entries = std::fs::read_dir(source).map_err(|error| format!("读取源目录失败：{error}"))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("读取源目录失败：{error}"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if EXCLUDED_DIR_NAMES.contains(&name.as_str()) {
            continue;
        }
        let from = entry.path();
        let to = destination.join(&name);
        let kind = entry
            .file_type()
            .map_err(|error| format!("读取源条目类型失败：{error}"))?;
        if kind.is_dir() {
            std::fs::create_dir_all(&to).map_err(|error| format!("创建目录失败：{error}"))?;
            copy_directory(&from, &to)?;
        } else if kind.is_file() {
            std::fs::copy(&from, &to).map_err(|error| format!("复制文件失败：{error}"))?;
        }
        // 符号链接既不跟随也不复制：跟随可能把目录外的内容拉进来。
    }
    Ok(())
}

/// 读取并校验 `SKILL.md`。
fn read_skill_file(path: &Path) -> Result<crate::skills::core::SkillMetadata, String> {
    let stat = std::fs::metadata(path).map_err(|_| "未找到 SKILL.md。".to_owned())?;
    if !stat.is_file() {
        return Err("未找到 SKILL.md。".to_owned());
    }
    if stat.len() > MAX_SKILL_MD_BYTES {
        return Err("SKILL.md 过大，拒绝导入。".to_owned());
    }
    let content =
        std::fs::read_to_string(path).map_err(|error| format!("读取 SKILL.md 失败：{error}"))?;
    read_skill_metadata(&content)
}

/// 扩展名是否是 `.md`（大小写不敏感）。
fn has_markdown_extension(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("md"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::store::{list_skills, set_enabled};

    /// 建一个源目录形式的 skill。
    fn make_source_dir(root: &Path, name: &str, description: &str) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).expect("目录");
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n"),
        )
        .expect("写入");
        dir
    }

    fn request(source: PathBuf, home: &str, kind: ImportKind, overwrite: bool) -> ImportRequest {
        ImportRequest {
            source,
            codex_home: home.to_owned(),
            kind,
            overwrite,
            suffix: "test".to_owned(),
        }
    }

    #[test]
    fn a_directory_skill_is_imported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("home");
        let source = make_source_dir(dir.path(), "alpha", "描述");

        let outcome = import_skill(&request(
            source,
            &home.to_string_lossy(),
            ImportKind::Directory,
            false,
        ))
        .expect("导入");
        assert_eq!(outcome.skill_name, "alpha");
        let skills = list_skills(&home.to_string_lossy());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].description, "描述");
    }

    #[test]
    fn a_markdown_file_becomes_a_skill_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("home");
        let file = dir.path().join("alpha.md");
        std::fs::write(&file, "---\nname: alpha\ndescription: 来自文件\n---\n").expect("写入");

        import_skill(&request(
            file,
            &home.to_string_lossy(),
            ImportKind::File,
            false,
        ))
        .expect("导入");
        let skills = list_skills(&home.to_string_lossy());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].description, "来自文件");
        // 文件形态下应当落成 SKILL.md。
        assert!(Path::new(&skills[0].path).join("SKILL.md").is_file());
    }

    #[test]
    fn a_non_markdown_file_is_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("home");
        let file = dir.path().join("alpha.txt");
        std::fs::write(&file, "x").expect("写入");
        let error = import_skill(&request(
            file,
            &home.to_string_lossy(),
            ImportKind::File,
            false,
        ))
        .expect_err("必须拒绝");
        assert_eq!(error, "请选择 Markdown 文件。");
    }

    #[test]
    fn a_missing_source_is_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("home");
        let error = import_skill(&request(
            dir.path().join("nope"),
            &home.to_string_lossy(),
            ImportKind::Directory,
            false,
        ))
        .expect_err("必须拒绝");
        assert_eq!(error, "选择的路径不存在。");
    }

    #[test]
    fn a_missing_skill_md_is_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("home");
        let source = dir.path().join("alpha");
        std::fs::create_dir_all(&source).expect("目录");
        let error = import_skill(&request(
            source,
            &home.to_string_lossy(),
            ImportKind::Directory,
            false,
        ))
        .expect_err("必须拒绝");
        assert_eq!(error, "未找到 SKILL.md。");
    }

    #[test]
    fn an_existing_skill_is_rejected_without_overwrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("home");
        let source = make_source_dir(dir.path(), "alpha", "第一版");
        import_skill(&request(
            source.clone(),
            &home.to_string_lossy(),
            ImportKind::Directory,
            false,
        ))
        .expect("首次");

        let error = import_skill(&request(
            source,
            &home.to_string_lossy(),
            ImportKind::Directory,
            false,
        ))
        .expect_err("必须拒绝");
        assert_eq!(error, "skill 已存在：alpha");
        // 原有内容必须没被动过。
        assert_eq!(
            list_skills(&home.to_string_lossy())[0].description,
            "第一版"
        );
    }

    #[test]
    fn overwriting_replaces_the_previous_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("home");
        let first = make_source_dir(dir.path(), "alpha", "第一版");
        import_skill(&request(
            first,
            &home.to_string_lossy(),
            ImportKind::Directory,
            false,
        ))
        .expect("首次");

        // 用同名但内容不同的源覆盖。
        let second_dir = dir.path().join("second");
        std::fs::create_dir_all(&second_dir).expect("目录");
        std::fs::write(
            second_dir.join("SKILL.md"),
            "---\nname: alpha\ndescription: 第二版\n---\n",
        )
        .expect("写入");
        import_skill(&request(
            second_dir,
            &home.to_string_lossy(),
            ImportKind::Directory,
            true,
        ))
        .expect("覆盖");

        let skills = list_skills(&home.to_string_lossy());
        assert_eq!(skills.len(), 1, "覆盖后不该留下两份");
        assert_eq!(skills[0].description, "第二版");
    }

    #[test]
    fn overwriting_a_disabled_skill_leaves_no_stale_copy() {
        // 停用状态下覆盖：旧的那份（.disabled）必须一并让位，
        // 否则会出现「一份启用一份停用」。
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("home");
        let source = make_source_dir(dir.path(), "alpha", "旧");
        import_skill(&request(
            source.clone(),
            &home.to_string_lossy(),
            ImportKind::Directory,
            false,
        ))
        .expect("首次");
        set_enabled(&home.to_string_lossy(), "alpha", false).expect("停用");

        import_skill(&request(
            source,
            &home.to_string_lossy(),
            ImportKind::Directory,
            true,
        ))
        .expect("覆盖");

        let skills = list_skills(&home.to_string_lossy());
        assert_eq!(skills.len(), 1, "不该同时存在启用与停用两份");
        assert!(skills[0].enabled);
    }

    #[test]
    fn an_already_enabled_skill_counts_as_existing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("home");
        let source = make_source_dir(dir.path(), "alpha", "描述");
        import_skill(&request(
            source.clone(),
            &home.to_string_lossy(),
            ImportKind::Directory,
            false,
        ))
        .expect("首次");
        set_enabled(&home.to_string_lossy(), "alpha", false).expect("停用");
        // 停用状态下不覆盖应当也被拒绝。
        let error = import_skill(&request(
            source,
            &home.to_string_lossy(),
            ImportKind::Directory,
            false,
        ))
        .expect_err("必须拒绝");
        assert_eq!(error, "skill 已存在：alpha");
    }

    #[test]
    fn the_dev_directories_are_excluded() {
        // `.git` 与 `node_modules` 体积可能远大于 skill 本身，且与运行无关。
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("home");
        let source = make_source_dir(dir.path(), "alpha", "描述");
        std::fs::create_dir_all(source.join(".git")).expect("目录");
        std::fs::write(source.join(".git").join("config"), "x").expect("写入");
        std::fs::create_dir_all(source.join("node_modules")).expect("目录");
        std::fs::write(source.join("node_modules").join("dep.js"), "x").expect("写入");
        std::fs::write(source.join("extra.txt"), "保留").expect("写入");

        import_skill(&request(
            source,
            &home.to_string_lossy(),
            ImportKind::Directory,
            false,
        ))
        .expect("导入");

        let dest = active_dir(&home.to_string_lossy()).join("alpha");
        assert!(!dest.join(".git").exists(), ".git 不该被复制");
        assert!(
            !dest.join("node_modules").exists(),
            "node_modules 不该被复制"
        );
        assert!(dest.join("extra.txt").is_file(), "其他文件应当保留");
        assert!(dest.join("SKILL.md").is_file());
    }

    #[test]
    fn nested_directories_are_copied() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("home");
        let source = make_source_dir(dir.path(), "alpha", "描述");
        let nested = source.join("scripts").join("deep");
        std::fs::create_dir_all(&nested).expect("目录");
        std::fs::write(nested.join("run.sh"), "echo hi").expect("写入");

        import_skill(&request(
            source,
            &home.to_string_lossy(),
            ImportKind::Directory,
            false,
        ))
        .expect("导入");
        assert!(
            active_dir(&home.to_string_lossy())
                .join("alpha")
                .join("scripts")
                .join("deep")
                .join("run.sh")
                .is_file()
        );
    }

    #[test]
    fn no_temporary_or_backup_directories_are_left_behind() {
        // 临时目录与备份都以 `.` 开头，任何残留都会被下一次导入当成 skill 目录看到。
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("home");
        let source = make_source_dir(dir.path(), "alpha", "描述");
        import_skill(&request(
            source.clone(),
            &home.to_string_lossy(),
            ImportKind::Directory,
            false,
        ))
        .expect("首次");
        import_skill(&request(
            source,
            &home.to_string_lossy(),
            ImportKind::Directory,
            true,
        ))
        .expect("覆盖");

        let leftovers: Vec<String> = std::fs::read_dir(active_dir(&home.to_string_lossy()))
            .expect("目录")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "有残留目录：{leftovers:?}");
    }

    #[test]
    fn an_invalid_frontmatter_is_rejected_before_touching_the_target() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("home");
        let source = dir.path().join("bad");
        std::fs::create_dir_all(&source).expect("目录");
        std::fs::write(source.join("SKILL.md"), "# 没有 frontmatter\n").expect("写入");

        let error = import_skill(&request(
            source,
            &home.to_string_lossy(),
            ImportKind::Directory,
            false,
        ))
        .expect_err("必须拒绝");
        assert_eq!(error, "SKILL.md 必须以 YAML frontmatter 开头。");
        // 目标位置不该因为一次失败而被创建。
        assert!(!active_dir(&home.to_string_lossy()).join("bad").exists());
    }

    #[test]
    fn a_path_traversal_name_is_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("home");
        let source = dir.path().join("evil");
        std::fs::create_dir_all(&source).expect("目录");
        std::fs::write(
            source.join("SKILL.md"),
            "---\nname: ../../evil\ndescription: d\n---\n",
        )
        .expect("写入");
        let error = import_skill(&request(
            source,
            &home.to_string_lossy(),
            ImportKind::Directory,
            false,
        ))
        .expect_err("必须拒绝");
        assert!(error.contains("skill name"), "{error}");
    }

    #[test]
    fn the_kind_must_match_the_actual_source() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("home");
        let source = make_source_dir(dir.path(), "alpha", "描述");
        // 目录却按文件处理。
        let error = import_skill(&request(
            source.clone(),
            &home.to_string_lossy(),
            ImportKind::File,
            false,
        ))
        .expect_err("必须拒绝");
        assert_eq!(error, "只能导入 Markdown 文件或 skill 目录。");

        let file = dir.path().join("x.md");
        std::fs::write(&file, "---\nname: a\ndescription: d\n---\n").expect("写入");
        let error = import_skill(&request(
            file,
            &home.to_string_lossy(),
            ImportKind::Directory,
            false,
        ))
        .expect_err("必须拒绝");
        assert_eq!(error, "只能导入 Markdown 文件或 skill 目录。");
    }

    #[test]
    fn the_imported_skill_can_be_listed_and_toggled() {
        // 端到端：导入 → 列表 → 停用 → 列表。
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("home");
        let source = make_source_dir(dir.path(), "alpha", "描述");
        import_skill(&request(
            source,
            &home.to_string_lossy(),
            ImportKind::Directory,
            false,
        ))
        .expect("导入");

        let home_str = home.to_string_lossy().into_owned();
        assert!(list_skills(&home_str)[0].enabled);
        set_enabled(&home_str, "alpha", false).expect("停用");
        assert!(!list_skills(&home_str)[0].enabled);
    }
}
