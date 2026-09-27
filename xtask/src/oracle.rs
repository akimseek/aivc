//! 用**现网 TypeScript 实现**重新生成对拍 golden 文件。
//!
//! # 为什么把它做成 xtask 任务而不是散落的脚本
//!
//! 差分测试的价值取决于 golden 的**可信度**：如果 golden 是手工编辑的、或者生成方式
//! 只存在于某次对话里，那它就不再是「现网行为的快照」，而只是一份没人敢动的数据。
//!
//! 做成任务之后：
//!
//! - 生成方式有唯一入口，可被复述、可被审计；
//! - 场景与生成脚本的对应关系写在代码里，不依赖记忆；
//! - 缺 Node 时给出明确指引，而不是静默跳过。
//!
//! # 为什么需要 Node
//!
//! 现网实现是 TypeScript。其中几个模块（`session-parser.ts`、`terminal-status-detector.ts`、
//! `shell-args.ts`、`wsl-paths.ts`、`target-ids.ts`）只有类型导入与 `node:path`，因此
//! Node 22.7+ 的 `--experimental-transform-types` 可以就地跑它们，不需要打包步骤。
//!
//! **只有生成本任务需要 Node**；跑测试不需要——测试读的是已生成的 golden。

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

/// 一份 golden 及其生成脚本。
struct Oracle {
    /// 生成脚本相对 workspace 根的路径。
    script: &'static str,
    /// 它产出的 golden 相对 workspace 根的路径。
    golden: &'static str,
    /// 参与的现网源码。
    sources: &'static [&'static str],
}

/// 全部对拍 golden。
const ORACLES: &[Oracle] = &[
    Oracle {
        script: "crates/aivc-terminal/tests/generate-status-golden.mjs",
        golden: "crates/aivc-terminal/tests/fixtures/status-golden.json",
        sources: &["src/electron/terminal/terminal-status-detector.ts"],
    },
    Oracle {
        script: "crates/aivc-provider/tests/generate-parser-golden.mjs",
        golden: "crates/aivc-provider/tests/fixtures/parser-golden.json",
        sources: &["src/shared/session-parser.ts"],
    },
    Oracle {
        script: "crates/aivc-core/tests/generate-shell-golden.mjs",
        golden: "crates/aivc-core/tests/fixtures/shell-golden.json",
        sources: &[
            "src/shared/shell-args.ts",
            "src/shared/target-ids.ts",
            "src/shared/wsl-paths.ts",
        ],
    },
];

/// 重新生成全部 golden。
pub fn run() -> Result<()> {
    let root = workspace_root()?;
    // 现网源码不在本工程里，而在它的上一级——这里显式检查，给出可操作的提示。
    let repo_root = root
        .parent()
        .context("无法定位现网源码所在目录（workspace 的上一级）")?;
    let detector = repo_root
        .join("src")
        .join("shared")
        .join("session-parser.ts");
    if !detector.is_file() {
        bail!(
            "找不到现网源码：{}。本任务需要在包含现网 TypeScript 工程的目录下运行。",
            detector.display()
        );
    }

    if Command::new("node").arg("--version").output().is_err() {
        bail!("找不到 node。生成本对拍 golden 需要 Node 22.7+（用它转译现网的 TypeScript）。");
    }

    for oracle in ORACLES {
        let script = root.join(oracle.script);
        if !script.is_file() {
            bail!("生成脚本不存在：{}", script.display());
        }
        println!("=== {} ===", oracle.script);
        for source in oracle.sources {
            println!("  依据现网源码：{source}");
        }
        let output = Command::new("node")
            .arg("--experimental-transform-types")
            .arg(&script)
            .current_dir(&root)
            .output()
            .with_context(|| format!("运行 {} 失败", script.display()))?;
        // Node 把实验特性警告写到 stderr，因此只在失败时展示它的 stderr。
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("{} 执行失败：\n{stderr}", script.display());
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        print!("{stdout}");
        let golden = root.join(oracle.golden);
        if !golden.is_file() {
            bail!("脚本执行成功但没有产出 {}", golden.display());
        }
        println!("  已写出 {}", oracle.golden);
    }

    println!();
    println!("全部 golden 已重新生成。请运行 `cargo xtask parity` 与测试确认没有回归。");
    Ok(())
}

/// workspace 根目录：本 crate 的上一级。
fn workspace_root() -> Result<std::path::PathBuf> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .map(Path::to_path_buf)
        .context("无法定位 workspace 根目录")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_oracle_declares_its_script_golden_and_sources() {
        for oracle in ORACLES {
            assert!(!oracle.script.is_empty(), "有 oracle 缺少生成脚本");
            assert!(!oracle.golden.is_empty(), "有 oracle 缺少 golden 路径");
            assert!(
                !oracle.sources.is_empty(),
                "有 oracle 没有声明依据的现网源码"
            );
            assert!(
                oracle.script.to_ascii_lowercase().ends_with(".mjs"),
                "{} 不是 mjs 脚本",
                oracle.script
            );
            assert!(
                oracle.golden.to_ascii_lowercase().ends_with(".json"),
                "{} 不是 json",
                oracle.golden
            );
            for source in oracle.sources {
                assert!(
                    source.to_ascii_lowercase().ends_with(".ts"),
                    "{source} 不是 TypeScript 源码"
                );
            }
        }
    }

    #[test]
    fn golden_paths_are_unique() {
        let paths: BTreeSet<&str> = ORACLES.iter().map(|item| item.golden).collect();
        assert_eq!(paths.len(), ORACLES.len(), "有重复的 golden 路径");
    }

    #[test]
    fn the_declared_files_exist() {
        // 声明了却不存在是最容易发生的漂移（改了目录忘了改这里），因此直接断言。
        let root = workspace_root().expect("workspace root");
        for oracle in ORACLES {
            for relative in std::iter::once(oracle.script)
                .chain(std::iter::once(oracle.golden))
                .chain(oracle.sources.iter().copied())
                .filter(|item| item.starts_with("crates/"))
            {
                assert!(root.join(relative).is_file(), "{relative} 不存在");
            }
        }
    }
}
