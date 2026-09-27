//! Provider 层：AI CLI 的会话资产实现。
//!
//! # 结构
//!
//! - [`context`]：依赖注入（仓储、存储、时钟、平台）。Provider 不自己创建连接。
//! - [`registry`]：`SessionProvider` trait、能力子 trait、注册表与目标分发。
//! - [`common`]：目标探测骨架、排序、搜索匹配、有限并发。
//! - `codex` / `claude` / `gemini` / `qoder`：各平台实现。
//!
//! # 能力用子 trait 表达
//!
//! 不支持的能力在编译期就不存在代码路径（见 [`registry`] 的模块文档）。注册表把
//! 「实现具备什么」推导成能力矩阵交给渲染层，并由 [`registry::ProviderRegistry::verify_capabilities`]
//! 与静态声明做自检，因此不存在「标记说支持、运行时却不支持」的漂移。
//!
//! # 与现网的关系
//!
//! 现网刻意**不**为 Provider 做统一基类：Codex 因官方 `app-server` 分支、32 MB 读取
//! 上限与独立缓存体系而无法与其余三家共享控制流（DEV-GUIDE 已记录该结论）。这里保留
//! 同样的取舍：`common` 只放真正共享的纯逻辑，存储布局与文件格式仍各自实现。

#![forbid(unsafe_code)]

pub mod claude;
pub mod codex;
pub mod common;
pub mod context;
pub mod export;
pub mod gemini;
pub mod paging;
pub mod qoder;
pub mod registry;

#[cfg(test)]
pub mod testing;

pub use context::{ProviderContext, ProviderRepositories, ProviderStore};
pub use registry::{
    Branchable, Duplicable, Exportable, ProviderEntry, ProviderRegistry, SessionProvider,
};

/// 构建包含全部**已实现**平台的注册表。
///
/// 每接入一个平台就在这里登记一次，并显式声明它实现了哪些能力子 trait。
/// [`ProviderRegistry::verify_capabilities`] 会把由此推导出的能力矩阵与静态声明比对，
/// 因此「登记时漏写一个 `with_*`」会立刻被测试发现，而不是等到用户点击才发现功能缺失。
///
/// 四个平台现已全部登记。**登记即声明**：`ProviderEntry` 上追加了哪些能力子 trait，
/// 能力矩阵就推导出哪些能力；`verify_capabilities` 再与静态声明比对，因此
/// 「登记时漏写一个 with_*」会立刻被测试发现。
#[must_use]
pub fn build_registry() -> ProviderRegistry {
    use std::sync::Arc;

    let mut registry = ProviderRegistry::new();
    let codex: Arc<codex::provider::CodexProvider> =
        Arc::new(codex::provider::CodexProvider::new());
    registry.register(
        ProviderEntry::basic(Arc::clone(&codex) as Arc<dyn SessionProvider>)
            .with_branch(Arc::clone(&codex) as Arc<dyn Branchable>)
            .with_duplicate(Arc::clone(&codex) as Arc<dyn Duplicable>)
            .with_export(Arc::clone(&codex) as Arc<dyn Exportable>),
    );

    let claude: Arc<claude::provider::ClaudeProvider> =
        Arc::new(claude::provider::ClaudeProvider::new());
    registry.register(
        ProviderEntry::basic(Arc::clone(&claude) as Arc<dyn SessionProvider>)
            .with_branch(Arc::clone(&claude) as Arc<dyn Branchable>)
            .with_duplicate(Arc::clone(&claude) as Arc<dyn Duplicable>)
            .with_export(Arc::clone(&claude) as Arc<dyn Exportable>),
    );

    let gemini: Arc<gemini::provider::GeminiProvider> =
        Arc::new(gemini::provider::GeminiProvider::new());
    registry.register(
        ProviderEntry::basic(Arc::clone(&gemini) as Arc<dyn SessionProvider>)
            .with_branch(Arc::clone(&gemini) as Arc<dyn Branchable>)
            .with_duplicate(Arc::clone(&gemini) as Arc<dyn Duplicable>)
            .with_export(Arc::clone(&gemini) as Arc<dyn Exportable>),
    );

    // Qoder 只登记基础能力 + 导出：分支与复制必须在它自己的 CLI 内完成。
    let qoder: Arc<qoder::provider::QoderProvider> =
        Arc::new(qoder::provider::QoderProvider::new());
    registry.register(
        ProviderEntry::basic(Arc::clone(&qoder) as Arc<dyn SessionProvider>)
            .with_export(Arc::clone(&qoder) as Arc<dyn Exportable>),
    );

    registry
}

#[cfg(test)]
mod registry_tests {
    use super::*;
    use aivc_core::target::ProviderId;

    #[test]
    fn the_registry_declares_capabilities_consistent_with_the_static_matrix() {
        // 这是本设计的核心自检：能力矩阵由**实现**推导，并与静态声明比对。
        // 漏写一个 `with_*` 或实现漂移都会在这里失败。
        build_registry()
            .verify_capabilities()
            .expect("已登记平台的能力矩阵必须与静态声明一致");
    }

    #[test]
    fn codex_is_registered_with_all_three_capabilities() {
        let registry = build_registry();
        let capabilities = registry.capabilities();
        let codex = capabilities.get(&ProviderId::Codex).expect("Codex 已登记");
        assert!(codex.branch, "Codex 必须支持分支");
        assert!(codex.duplicate, "Codex 必须支持复制");
        assert!(codex.export, "Codex 必须支持导出");
        assert!(codex.skills, "Codex 是唯一支持 Skill 管理的平台");
    }

    #[test]
    fn claude_is_registered_without_skills() {
        let registry = build_registry();
        let capabilities = registry.capabilities();
        let claude = capabilities
            .get(&ProviderId::Claude)
            .expect("Claude 已登记");
        assert!(claude.branch && claude.duplicate && claude.export);
        assert!(!claude.skills, "Claude 不支持 Skill 管理");
        assert!(!claude.session_settings);
    }

    #[test]
    fn qoder_is_registered_with_only_export() {
        let registry = build_registry();
        let capabilities = registry.capabilities();
        let qoder = capabilities.get(&ProviderId::Qoder).expect("Qoder 已登记");
        assert!(qoder.export, "Qoder 支持导出");
        assert!(!qoder.branch, "Qoder 不支持分支");
        assert!(!qoder.duplicate, "Qoder 不支持复制");
        assert!(!qoder.vendor_management, "Qoder 不参与厂商管理");
        // 回收站与批量操作仍然可用。
        assert!(qoder.trash && qoder.batch_actions);
    }

    #[test]
    fn every_declared_platform_is_registered() {
        // 四个平台全部登记；少一个说明接线漏了。
        let registry = build_registry();
        for id in ProviderId::ALL {
            assert!(registry.get(id).is_some(), "{id:?} 未登记");
        }
    }

    #[test]
    fn unsupported_capabilities_are_rejected_at_dispatch() {
        // Qoder 未登记 Branchable：必须给出与现网一致的显式拒绝。
        let registry = build_registry();
        let target = aivc_core::target::Target {
            id: aivc_core::id::TargetId::from("qoder:local"),
            provider: ProviderId::Qoder,
            label: "Qoder".to_owned(),
            kind: aivc_core::target::TargetKind::Local,
            distro: None,
            codex_home: None,
            available: true,
            detail: None,
        };
        let error = registry.branchable(&target).err().expect("必须拒绝");
        assert_eq!(error.to_string(), "Qoder CN 不支持分支。");
        assert!(registry.exportable(&target).is_ok(), "导出应当可用");
    }
}
