//! 构建与校验自动化。
//!
//! 目前提供 `check-layers`：基于 `cargo metadata` 校验 crate 依赖方向，
//! 因为 `cargo-deny` 无法表达「某 crate 不得依赖任何其他 workspace crate」这类
//! 反向约束，也无法限制某一层的**外部**依赖集合。
//!
//! 用法：
//!
//! ```text
//! cargo xtask check-layers
//! ```

use std::collections::BTreeSet;
use std::process::Command;

use anyhow::{Context, Result, bail};

mod oracle;
mod parity;

/// 分层定义：数值越小越靠内层。
///
/// 规则：一个 crate 只能依赖**同层或更内层**的 workspace crate。
const LAYERS: &[(&str, u32)] = &[
    ("aivc-core", 0),
    ("aivc-infra", 1),
    ("aivc-provider", 2),
    ("aivc-gateway", 2),
    ("aivc-terminal", 2),
    ("aivc-tooling", 2),
    ("aivc-ipc", 3),
    ("aivc-app", 4),
];

/// `aivc-core` 允许的外部依赖白名单。
///
/// 领域层必须保持「零 I/O、零平台依赖」。任何新增外部依赖都要在这里显式登记，
/// 从而强制作者思考它是否真的属于领域层。
const CORE_EXTERNAL_ALLOWLIST: &[&str] = &["serde", "serde_json", "thiserror", "uuid", "tracing"];

fn main() -> Result<()> {
    let task = std::env::args().nth(1).unwrap_or_default();
    match task.as_str() {
        "check-layers" => check_layers(),
        "parity" => run_parity(),
        "regen-golden" => oracle::run(),
        "" => bail!("用法：cargo xtask <check-layers|parity|regen-golden>"),
        other => bail!("未知任务：{other}（可用：check-layers, parity, regen-golden）"),
    }
}

/// 读取 workspace 成员的依赖关系并校验分层。
fn check_layers() -> Result<()> {
    let metadata = load_metadata()?;
    let mut violations = Vec::new();

    for package in &metadata.packages {
        let Some(layer) = layer_of(&package.name) else {
            // xtask 自身不参与分层。
            continue;
        };

        for dependency in &package.dependencies {
            // dev-dependencies 只在测试期编译，且「低层测试引用高层」是合法模式；
            // 这里只严格校验正常依赖与构建依赖。
            if dependency.kind.as_deref() == Some("dev") {
                continue;
            }

            match layer_of(&dependency.name) {
                Some(dependency_layer) if dependency_layer > layer => {
                    violations.push(format!(
                        "{}（第 {layer} 层）依赖了更外层的 {}（第 {dependency_layer} 层）",
                        package.name, dependency.name
                    ));
                }
                Some(_) => {}
                None => {
                    // 外部依赖：只有领域层需要白名单约束。
                    if package.name == "aivc-core"
                        && !CORE_EXTERNAL_ALLOWLIST.contains(&dependency.name.as_str())
                    {
                        violations.push(format!(
                            "aivc-core 引入了白名单外的外部依赖 {}；领域层必须保持零 I/O、零平台依赖。\
                             确有必要时请在 xtask 的 CORE_EXTERNAL_ALLOWLIST 中显式登记并说明理由。",
                            dependency.name
                        ));
                    }
                }
            }
        }
    }

    // 反向约束：aivc-core 不得依赖任何 workspace crate。
    if let Some(core) = metadata
        .packages
        .iter()
        .find(|item| item.name == "aivc-core")
    {
        for dependency in &core.dependencies {
            if dependency.kind.as_deref() == Some("dev") {
                continue;
            }
            if layer_of(&dependency.name).is_some() {
                violations.push(format!(
                    "aivc-core 依赖了 workspace crate {}；领域层必须是最内层，不允许任何内部依赖。",
                    dependency.name
                ));
            }
        }
    }

    // 完整性：LAYERS 必须覆盖全部 workspace 成员（xtask 除外），避免新增 crate 后漏登记。
    let declared: BTreeSet<&str> = LAYERS.iter().map(|(name, _)| *name).collect();
    for package in &metadata.packages {
        if package.name != "xtask" && !declared.contains(package.name.as_str()) {
            violations.push(format!(
                "workspace 成员 {} 未在 xtask 的 LAYERS 中登记分层。",
                package.name
            ));
        }
    }

    if violations.is_empty() {
        println!(
            "依赖分层校验通过：{} 个 crate，{} 层。",
            LAYERS.len(),
            max_layer() + 1
        );
        return Ok(());
    }

    eprintln!("依赖分层校验失败：");
    for violation in &violations {
        eprintln!("  - {violation}");
    }
    bail!("共 {} 处分层违规。", violations.len())
}

/// 查询 crate 所在层。
fn layer_of(name: &str) -> Option<u32> {
    LAYERS
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, layer)| *layer)
}

/// 最外层编号。
fn max_layer() -> u32 {
    LAYERS.iter().map(|(_, layer)| *layer).max().unwrap_or(0)
}

/// `cargo metadata --no-deps` 的最小反序列化视图。
#[derive(serde::Deserialize)]
struct Metadata {
    packages: Vec<Package>,
}

/// 单个 workspace 成员。
#[derive(serde::Deserialize)]
struct Package {
    name: String,
    dependencies: Vec<Dependency>,
}

/// 一条依赖声明。
#[derive(serde::Deserialize)]
struct Dependency {
    name: String,
    /// `null` 表示正常依赖，`"dev"` / `"build"` 分别为开发与构建依赖。
    kind: Option<String>,
}

/// 调用 `cargo metadata`。
///
/// 用 `CARGO` 环境变量而非硬编码 `cargo`，以便在自定义工具链下也能运行。
fn load_metadata() -> Result<Metadata> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(workspace_root()?)
        .output()
        .context("无法执行 cargo metadata")?;

    if !output.status.success() {
        bail!(
            "cargo metadata 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    serde_json::from_slice(&output.stdout).context("无法解析 cargo metadata 输出")
}

/// 定位 workspace 根目录（本文件位于 `<root>/xtask/src/main.rs`）。
fn workspace_root() -> Result<std::path::PathBuf> {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .map(std::path::Path::to_path_buf)
        .context("无法定位 workspace 根目录")
}

/// 校验验收矩阵里声明的测试覆盖是否真的存在。
///
/// 见 `parity` 模块：矩阵是文档，不会因为有人删掉一条测试而失效；这个任务把
/// 「条目 → 测试名前缀」的对应关系变成可执行断言。
fn run_parity() -> Result<()> {
    println!("正在枚举测试名（cargo test --workspace -- --list）…");
    let output = Command::new(env!("CARGO"))
        .args(["test", "--workspace", "--offline", "--", "--list"])
        .output()
        .context("无法运行 cargo test 枚举测试")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let tests: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.strip_suffix(": test"))
        .collect();
    if tests.is_empty() {
        bail!("没有枚举到任何测试；cargo test 可能失败了");
    }
    println!("枚举到 {} 个测试。", tests.len());

    let mut missing = Vec::new();
    let mut covered = 0usize;
    println!();
    println!("{:<8} {:<44} 覆盖测试数", "条目", "行为");
    println!("{}", "-".repeat(76));
    for item in parity::matrix() {
        let matched: Vec<&str> = item
            .covers
            .iter()
            .filter(|prefix| tests.iter().any(|name| name.contains(**prefix)))
            .copied()
            .collect();
        if matched.len() < item.covers.len() {
            let lost: Vec<&str> = item
                .covers
                .iter()
                .filter(|prefix| !matched.contains(prefix))
                .copied()
                .collect();
            missing.push((item.id, lost));
        }
        covered += 1;
        println!(
            "{:<8} {:<44} {}/{}",
            item.id,
            truncate(item.behavior, 42),
            matched.len(),
            item.covers.len()
        );
    }

    println!();
    println!("已纳入自动校验：{covered} 条");
    for (section, description) in parity::UNCOVERED_SECTIONS {
        println!("未纳入自动校验：{section} —— {description}");
    }

    if !missing.is_empty() {
        println!();
        for (id, lost) in &missing {
            eprintln!("条目 {id} 声明的覆盖测试不存在：{}", lost.join(", "));
        }
        bail!("有 {} 条验收条目声明的测试覆盖已丢失", missing.len());
    }
    println!();
    println!("全部已声明的覆盖测试都存在。");
    Ok(())
}

/// 按字符截断，避免把多字节字符切成半个。
fn truncate(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_owned();
    }
    let mut text: String = value.chars().take(limit.saturating_sub(1)).collect();
    text.push('…');
    text
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn layers_are_contiguous_from_zero() {
        let declared: BTreeSet<u32> = LAYERS.iter().map(|(_, layer)| *layer).collect();
        let expected: BTreeSet<u32> = (0..=max_layer()).collect();
        assert_eq!(declared, expected, "层号必须从 0 连续编号");
    }

    #[test]
    fn core_is_the_only_innermost_crate() {
        let innermost: Vec<&str> = LAYERS
            .iter()
            .filter(|(_, layer)| *layer == 0)
            .map(|(name, _)| *name)
            .collect();
        assert_eq!(innermost, vec!["aivc-core"]);
    }

    #[test]
    fn layer_lookup_matches_declaration() {
        assert_eq!(layer_of("aivc-core"), Some(0));
        assert_eq!(layer_of("aivc-gateway"), Some(2));
        assert_eq!(layer_of("aivc-app"), Some(4));
        assert_eq!(layer_of("serde"), None);
        assert_eq!(layer_of("xtask"), None);
    }

    #[test]
    fn layer_table_has_no_duplicate_crate_names() {
        let names: Vec<&str> = LAYERS.iter().map(|(name, _)| *name).collect();
        let unique: BTreeSet<&str> = names.iter().copied().collect();
        assert_eq!(names.len(), unique.len(), "LAYERS 存在重复登记");
    }

    #[test]
    fn every_layer_is_reachable_and_named_consistently() {
        // 防止出现空层或命名不一致（例如把 aivc-infra-* 写成 infra-*）。
        let grouped: BTreeMap<u32, Vec<&str>> =
            LAYERS
                .iter()
                .fold(BTreeMap::new(), |mut acc, (name, layer)| {
                    acc.entry(*layer).or_default().push(name);
                    acc
                });
        for (layer, names) in grouped {
            assert!(!names.is_empty(), "第 {layer} 层为空");
            for name in names {
                assert!(name.starts_with("aivc-"), "{name} 命名不符合 aivc-* 约定");
            }
        }
    }

    #[test]
    fn core_allowlist_stays_minimal() {
        // 领域层的依赖集合必须保持极小；这条测试用于在评审中触发讨论。
        assert!(
            CORE_EXTERNAL_ALLOWLIST.len() <= 6,
            "领域层依赖已膨胀，请复核"
        );
        assert!(CORE_EXTERNAL_ALLOWLIST.contains(&"serde"));
        assert!(CORE_EXTERNAL_ALLOWLIST.contains(&"thiserror"));
        // 明确禁止把 I/O 或平台库引入领域层。
        for forbidden in ["tokio", "rusqlite", "reqwest", "axum", "portable-pty"] {
            assert!(
                !CORE_EXTERNAL_ALLOWLIST.contains(&forbidden),
                "{forbidden} 不得出现在领域层"
            );
        }
    }
}
