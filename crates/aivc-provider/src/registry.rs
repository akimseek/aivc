//! Provider 注册表、能力子 trait 与目标分发。
//!
//! # 能力用子 trait 表达，而不是布尔矩阵
//!
//! 现网用 `capabilities: { branch: bool, duplicate: bool, … }` 加运行时判断，并靠契约
//! 测试保证「能力标记与运行时行为一致」（DEV-GUIDE 明确记录过这条约束）。这里改用
//! **能力子 trait**：
//!
//! ```text
//! QoderProvider        → 只实现 SessionProvider
//! CodexProvider        → SessionProvider + Branchable + Duplicable + Exportable
//! ```
//!
//! 不支持的能力在**编译期**就不存在该代码路径，`registry.branch(...)` 对 Qoder 会返回
//! [`Error::Unsupported`]，而调用方拿到的能力矩阵由注册表从实际实现推导，不可能漂移。
//!
//! # 为什么用 `BoxFuture` 而不是 `async-trait`
//!
//! 与 [`aivc_core::ports`] 同理：这些 trait 必须对象安全（`&dyn SessionProvider`），
//! 而原生 `async fn in trait` 目前不是 dyn 兼容的。`async-trait` 过程宏能解决，但会给
//! 本层引入一个宏依赖，且它生成的代码同样是每次调用一次装箱。这里直接写 `BoxFuture`：
//! 等价开销、无额外依赖、签名显式可见。

use std::collections::BTreeMap;
use std::sync::Arc;

use aivc_core::error::{Error, Result};
use aivc_core::id::SessionId;
use aivc_core::ports::BoxFuture;
use aivc_core::session::{
    ExportFormat, ExportResult, MessagePage, PageCursor, Session, SessionBatchResult,
    SessionDeleteResult, SessionMutationRef, SessionView,
};
use aivc_core::target::{ProviderCapabilities, ProviderId, Target};

use crate::context::ProviderContext;

/// 所有 Provider 必须实现的会话操作。
pub trait SessionProvider: Send + Sync {
    /// 平台标识。
    fn id(&self) -> ProviderId;

    /// 界面展示名。
    fn label(&self) -> &'static str;

    /// 探测全部目标（本机 + 全部 WSL 发行版）。
    fn list_targets<'a>(&'a self, ctx: &'a ProviderContext) -> BoxFuture<'a, Result<Vec<Target>>>;

    /// 只读已缓存的目标，不触发探测。
    fn list_cached_targets<'a>(
        &'a self,
        ctx: &'a ProviderContext,
    ) -> BoxFuture<'a, Result<Vec<Target>>>;

    /// 只读已缓存的会话摘要，不扫描文件系统。
    fn list_cached_sessions<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        view: SessionView,
    ) -> BoxFuture<'a, Result<Vec<Session>>>;

    /// 扫描并列出活跃会话。
    fn list_sessions<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
    ) -> BoxFuture<'a, Result<Vec<Session>>>;

    /// 扫描并列出回收站会话。
    fn list_trash_sessions<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
    ) -> BoxFuture<'a, Result<Vec<Session>>>;

    /// 搜索会话：摘要匹配优先，未命中再扫描文件内容。
    fn search_sessions<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        view: SessionView,
        query: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Session>>>;

    /// 读取完整会话（含全部可见消息）。
    fn get_session<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        id: &'a SessionId,
        file_hint: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Session>>;

    /// 分页读取会话消息。
    fn get_session_messages_page<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        id: &'a SessionId,
        cursor: PageCursor,
        limit: u32,
    ) -> BoxFuture<'a, Result<MessagePage>>;

    /// 读取会话摘要（不含正文）。
    fn get_session_summary<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        id: &'a SessionId,
    ) -> BoxFuture<'a, Result<Session>>;

    /// 反查某会话的全部分支子会话。
    fn list_sessions_by_parent<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        parent: &'a SessionId,
    ) -> BoxFuture<'a, Result<Vec<Session>>>;

    /// 会话文件所在目录（用于「打开所在文件夹」）。
    fn get_session_folder_path<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        id: &'a SessionId,
    ) -> BoxFuture<'a, Result<String>>;

    /// 删除单个会话（移入回收站）。
    fn delete_session<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        reference: &'a SessionMutationRef,
    ) -> BoxFuture<'a, Result<SessionDeleteResult>>;

    /// 批量删除会话。
    fn delete_sessions<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        references: &'a [SessionMutationRef],
    ) -> BoxFuture<'a, Result<SessionBatchResult>>;

    /// 从回收站恢复会话，返回恢复后的路径。
    fn restore_session<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        id: &'a SessionId,
    ) -> BoxFuture<'a, Result<String>>;

    /// 永久删除单个会话，返回被删除的路径。
    fn purge_session<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        reference: &'a SessionMutationRef,
    ) -> BoxFuture<'a, Result<String>>;

    /// 批量永久删除会话。
    fn purge_sessions<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        references: &'a [SessionMutationRef],
    ) -> BoxFuture<'a, Result<SessionBatchResult>>;
}

/// 支持「从此处分支」。
pub trait Branchable: SessionProvider {
    /// 从指定原始 JSONL 行创建分支。
    ///
    /// `message_line` 是**原始 JSONL 的绝对行号**，不是可见消息序号——两者在过滤掉
    /// 合成消息后并不相等。
    fn branch_session<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        id: &'a SessionId,
        message_line: u64,
    ) -> BoxFuture<'a, Result<Session>>;
}

/// 支持复制会话。
pub trait Duplicable: SessionProvider {
    /// 复制会话；`title` 为空时沿用原标题。
    fn duplicate_session<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        id: &'a SessionId,
        title: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Session>>;
}

/// 支持导出会话。
pub trait Exportable: SessionProvider {
    /// 导出会话到指定格式。
    ///
    /// `destination` 是**宿主侧**的目标文件路径：它由保存对话框决定，因此始终写在
    /// 本机，而不是目标（可能位于 WSL）的存储里。调用方负责在对话框取消时不调用本方法。
    fn export_session<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        id: &'a SessionId,
        format: ExportFormat,
        destination: &'a str,
    ) -> BoxFuture<'a, Result<ExportResult>>;
}

/// 一个已注册的平台：基础实现 + 可选能力。
pub struct ProviderEntry {
    provider: Arc<dyn SessionProvider>,
    branchable: Option<Arc<dyn Branchable>>,
    duplicable: Option<Arc<dyn Duplicable>>,
    exportable: Option<Arc<dyn Exportable>>,
}

impl std::fmt::Debug for ProviderEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderEntry")
            .field("id", &self.provider.id())
            .field("branchable", &self.branchable.is_some())
            .field("duplicable", &self.duplicable.is_some())
            .field("exportable", &self.exportable.is_some())
            .finish()
    }
}

impl ProviderEntry {
    /// 注册一个只具备基础能力的平台（例如 Qoder）。
    #[must_use]
    pub fn basic(provider: Arc<dyn SessionProvider>) -> Self {
        Self {
            provider,
            branchable: None,
            duplicable: None,
            exportable: None,
        }
    }

    /// 追加「可分支」能力。
    #[must_use]
    pub fn with_branch(mut self, provider: Arc<dyn Branchable>) -> Self {
        self.branchable = Some(provider);
        self
    }

    /// 追加「可复制」能力。
    #[must_use]
    pub fn with_duplicate(mut self, provider: Arc<dyn Duplicable>) -> Self {
        self.duplicable = Some(provider);
        self
    }

    /// 追加「可导出」能力。
    #[must_use]
    pub fn with_export(mut self, provider: Arc<dyn Exportable>) -> Self {
        self.exportable = Some(provider);
        self
    }

    /// 基础实现。
    #[must_use]
    pub fn provider(&self) -> &dyn SessionProvider {
        self.provider.as_ref()
    }

    /// 该平台实际具备的能力矩阵。
    ///
    /// 由**实现**推导而不是手写常量，因此不可能与运行时行为漂移。
    #[must_use]
    pub fn capabilities(&self) -> ProviderCapabilities {
        let id = self.provider.id();
        ProviderCapabilities {
            // Skill 管理只对 Codex 开放（现网能力矩阵）。
            skills: id == ProviderId::Codex,
            branch: self.branchable.is_some(),
            usage: true,
            trash: true,
            batch_actions: true,
            custom_cwd: true,
            export: self.exportable.is_some(),
            session_settings: id == ProviderId::Codex,
            duplicate: self.duplicable.is_some(),
            vendor_management: id != ProviderId::Qoder,
        }
    }

    /// 能力矩阵是否与 [`ProviderId::capabilities`] 的静态声明一致。
    ///
    /// 这是**契约自检**：静态表供渲染层早期渲染使用，实现决定真实行为，两者必须一致。
    #[must_use]
    pub fn capabilities_match_declaration(&self) -> bool {
        self.capabilities() == self.provider.id().capabilities()
    }
}

/// Provider 注册表。
#[derive(Debug, Default)]
pub struct ProviderRegistry {
    entries: BTreeMap<ProviderId, ProviderEntry>,
}

impl ProviderRegistry {
    /// 空注册表。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册一个平台。同一平台重复注册会覆盖。
    pub fn register(&mut self, entry: ProviderEntry) {
        self.entries.insert(entry.provider.id(), entry);
    }

    /// 按平台取基础实现。
    #[must_use]
    pub fn get(&self, id: ProviderId) -> Option<&dyn SessionProvider> {
        self.entries.get(&id).map(ProviderEntry::provider)
    }

    /// 列出全部已注册平台（顺序与 [`ProviderId::ALL`] 一致）。
    #[must_use]
    pub fn list(&self) -> Vec<&dyn SessionProvider> {
        ProviderId::ALL
            .into_iter()
            .filter_map(|id| self.get(id))
            .collect()
    }

    /// 导出全部平台的能力矩阵。
    #[must_use]
    pub fn capabilities(&self) -> BTreeMap<ProviderId, ProviderCapabilities> {
        self.entries
            .iter()
            .map(|(id, entry)| (*id, entry.capabilities()))
            .collect()
    }

    /// 校验全部平台的能力矩阵与静态声明一致。
    pub fn verify_capabilities(&self) -> Result<()> {
        for (id, entry) in &self.entries {
            if !entry.capabilities_match_declaration() {
                return Err(Error::Unsupported {
                    provider: id.display_name().to_owned(),
                    capability: format!(
                        "能力矩阵与静态声明不一致（实现 {:?} / 声明 {:?}）",
                        entry.capabilities(),
                        id.capabilities()
                    ),
                });
            }
        }
        Ok(())
    }

    /// 按目标分发到对应平台。
    pub fn dispatch<'a>(&'a self, target: &Target) -> Result<&'a dyn SessionProvider> {
        self.get(target.provider).ok_or_else(|| {
            Error::Target(aivc_core::error::TargetError::UnknownProvider {
                id: target.provider.as_str().to_owned(),
            })
        })
    }

    /// 按目标分发到「可分支」能力。
    ///
    /// 平台未注册该能力时返回 [`Error::Unsupported`]，与现网的显式拒绝语义一致。
    pub fn branchable<'a>(&'a self, target: &Target) -> Result<&'a dyn Branchable> {
        let entry = self.entry(target)?;
        entry
            .branchable
            .as_deref()
            .ok_or_else(|| Error::Unsupported {
                provider: entry.provider.id().display_name().to_owned(),
                capability: "分支".to_owned(),
            })
    }

    /// 按目标分发到「可复制」能力。
    pub fn duplicable<'a>(&'a self, target: &Target) -> Result<&'a dyn Duplicable> {
        let entry = self.entry(target)?;
        entry
            .duplicable
            .as_deref()
            .ok_or_else(|| Error::Unsupported {
                provider: entry.provider.id().display_name().to_owned(),
                capability: "复制".to_owned(),
            })
    }

    /// 按目标分发到「可导出」能力。
    pub fn exportable<'a>(&'a self, target: &Target) -> Result<&'a dyn Exportable> {
        let entry = self.entry(target)?;
        entry
            .exportable
            .as_deref()
            .ok_or_else(|| Error::Unsupported {
                provider: entry.provider.id().display_name().to_owned(),
                capability: "导出".to_owned(),
            })
    }

    fn entry<'a>(&'a self, target: &Target) -> Result<&'a ProviderEntry> {
        self.entries.get(&target.provider).ok_or_else(|| {
            Error::Target(aivc_core::error::TargetError::UnknownProvider {
                id: target.provider.as_str().to_owned(),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivc_core::id::TargetId;
    use aivc_core::target::TargetKind;

    /// 一个只实现基础能力的最小 Provider，用于验证注册表与能力推导。
    struct StubProvider {
        id: ProviderId,
    }

    impl SessionProvider for StubProvider {
        fn id(&self) -> ProviderId {
            self.id
        }

        fn label(&self) -> &'static str {
            self.id.display_name()
        }

        fn list_targets<'a>(
            &'a self,
            _ctx: &'a ProviderContext,
        ) -> BoxFuture<'a, Result<Vec<Target>>> {
            Box::pin(async move { Ok(Vec::new()) })
        }

        fn list_cached_targets<'a>(
            &'a self,
            _ctx: &'a ProviderContext,
        ) -> BoxFuture<'a, Result<Vec<Target>>> {
            Box::pin(async move { Ok(Vec::new()) })
        }

        fn list_cached_sessions<'a>(
            &'a self,
            _ctx: &'a ProviderContext,
            _target: &'a Target,
            _view: SessionView,
        ) -> BoxFuture<'a, Result<Vec<Session>>> {
            Box::pin(async move { Ok(Vec::new()) })
        }

        fn list_sessions<'a>(
            &'a self,
            _ctx: &'a ProviderContext,
            _target: &'a Target,
        ) -> BoxFuture<'a, Result<Vec<Session>>> {
            Box::pin(async move { Ok(Vec::new()) })
        }

        fn list_trash_sessions<'a>(
            &'a self,
            _ctx: &'a ProviderContext,
            _target: &'a Target,
        ) -> BoxFuture<'a, Result<Vec<Session>>> {
            Box::pin(async move { Ok(Vec::new()) })
        }

        fn search_sessions<'a>(
            &'a self,
            _ctx: &'a ProviderContext,
            _target: &'a Target,
            _view: SessionView,
            _query: &'a str,
        ) -> BoxFuture<'a, Result<Vec<Session>>> {
            Box::pin(async move { Ok(Vec::new()) })
        }

        fn get_session<'a>(
            &'a self,
            _ctx: &'a ProviderContext,
            _target: &'a Target,
            _id: &'a SessionId,
            _file_hint: Option<&'a str>,
        ) -> BoxFuture<'a, Result<Session>> {
            Box::pin(async move {
                Err(Error::Unsupported {
                    provider: self.id.display_name().to_owned(),
                    capability: "get_session".to_owned(),
                })
            })
        }

        fn get_session_messages_page<'a>(
            &'a self,
            _ctx: &'a ProviderContext,
            _target: &'a Target,
            _id: &'a SessionId,
            _cursor: PageCursor,
            _limit: u32,
        ) -> BoxFuture<'a, Result<MessagePage>> {
            Box::pin(async move {
                Err(Error::Unsupported {
                    provider: self.id.display_name().to_owned(),
                    capability: "get_session_messages_page".to_owned(),
                })
            })
        }

        fn get_session_summary<'a>(
            &'a self,
            _ctx: &'a ProviderContext,
            _target: &'a Target,
            _id: &'a SessionId,
        ) -> BoxFuture<'a, Result<Session>> {
            Box::pin(async move {
                Err(Error::Unsupported {
                    provider: self.id.display_name().to_owned(),
                    capability: "get_session_summary".to_owned(),
                })
            })
        }

        fn list_sessions_by_parent<'a>(
            &'a self,
            _ctx: &'a ProviderContext,
            _target: &'a Target,
            _parent: &'a SessionId,
        ) -> BoxFuture<'a, Result<Vec<Session>>> {
            Box::pin(async move { Ok(Vec::new()) })
        }

        fn get_session_folder_path<'a>(
            &'a self,
            _ctx: &'a ProviderContext,
            _target: &'a Target,
            _id: &'a SessionId,
        ) -> BoxFuture<'a, Result<String>> {
            Box::pin(async move {
                Err(Error::Unsupported {
                    provider: self.id.display_name().to_owned(),
                    capability: "get_session_folder_path".to_owned(),
                })
            })
        }

        fn delete_session<'a>(
            &'a self,
            _ctx: &'a ProviderContext,
            _target: &'a Target,
            _reference: &'a SessionMutationRef,
        ) -> BoxFuture<'a, Result<SessionDeleteResult>> {
            Box::pin(async move { Ok(SessionDeleteResult::default()) })
        }

        fn delete_sessions<'a>(
            &'a self,
            _ctx: &'a ProviderContext,
            _target: &'a Target,
            _references: &'a [SessionMutationRef],
        ) -> BoxFuture<'a, Result<SessionBatchResult>> {
            Box::pin(async move { Ok(SessionBatchResult::default()) })
        }

        fn restore_session<'a>(
            &'a self,
            _ctx: &'a ProviderContext,
            _target: &'a Target,
            _id: &'a SessionId,
        ) -> BoxFuture<'a, Result<String>> {
            Box::pin(async move {
                Err(Error::Unsupported {
                    provider: self.id.display_name().to_owned(),
                    capability: "restore_session".to_owned(),
                })
            })
        }

        fn purge_session<'a>(
            &'a self,
            _ctx: &'a ProviderContext,
            _target: &'a Target,
            _reference: &'a SessionMutationRef,
        ) -> BoxFuture<'a, Result<String>> {
            Box::pin(async move {
                Err(Error::Unsupported {
                    provider: self.id.display_name().to_owned(),
                    capability: "purge_session".to_owned(),
                })
            })
        }

        fn purge_sessions<'a>(
            &'a self,
            _ctx: &'a ProviderContext,
            _target: &'a Target,
            _references: &'a [SessionMutationRef],
        ) -> BoxFuture<'a, Result<SessionBatchResult>> {
            Box::pin(async move { Ok(SessionBatchResult::default()) })
        }
    }

    fn target(provider: ProviderId) -> Target {
        Target {
            id: TargetId::from("t"),
            provider,
            label: "t".to_owned(),
            kind: TargetKind::Local,
            distro: None,
            codex_home: None,
            available: true,
            detail: None,
        }
    }

    fn registry_with_all_basic() -> ProviderRegistry {
        let mut registry = ProviderRegistry::new();
        for id in ProviderId::ALL {
            registry.register(ProviderEntry::basic(Arc::new(StubProvider { id })));
        }
        registry
    }

    #[test]
    fn registry_lists_providers_in_declaration_order() {
        let registry = registry_with_all_basic();
        let ids: Vec<ProviderId> = registry
            .list()
            .into_iter()
            .map(SessionProvider::id)
            .collect();
        assert_eq!(ids, ProviderId::ALL.to_vec());
    }

    #[test]
    fn dispatch_resolves_by_target_provider() {
        let registry = registry_with_all_basic();
        assert_eq!(
            registry
                .dispatch(&target(ProviderId::Claude))
                .expect("claude")
                .id(),
            ProviderId::Claude
        );
        assert_eq!(
            registry
                .dispatch(&target(ProviderId::Qoder))
                .expect("qoder")
                .id(),
            ProviderId::Qoder
        );
    }

    #[test]
    fn dispatch_reports_unknown_providers() {
        let registry = ProviderRegistry::new();
        let error = registry
            .dispatch(&target(ProviderId::Codex))
            .err()
            .expect("必须失败");
        assert!(error.to_string().contains("未知 AI 平台"));
    }

    #[test]
    fn capabilities_are_derived_from_implemented_traits() {
        // 基础实现 + 显式追加的能力 → 能力矩阵由实现推导，不可能漂移。
        let mut registry = ProviderRegistry::new();
        let codex: Arc<StubProvider> = Arc::new(StubProvider {
            id: ProviderId::Codex,
        });
        // 这里只用基础注册验证「未实现即不支持」的推导方向。
        registry.register(ProviderEntry::basic(
            Arc::clone(&codex) as Arc<dyn SessionProvider>
        ));

        let caps = registry.capabilities();
        let codex_caps = caps.get(&ProviderId::Codex).expect("codex");
        assert!(!codex_caps.branch, "未追加 Branchable 时不得声称支持分支");
        assert!(!codex_caps.duplicate);
        assert!(!codex_caps.export);
        // 静态声明里 Codex 是支持分支的，因此此时两者应当不一致。
        assert!(
            !codex_caps.eq(&ProviderId::Codex.capabilities()),
            "能力矩阵必须由实现推导"
        );
    }

    #[test]
    fn verify_capabilities_detects_declaration_drift() {
        // 只注册基础能力时，与静态声明不一致 → 自检必须报错。
        let registry = registry_with_all_basic();
        let error = registry
            .verify_capabilities()
            .expect_err("基础注册与静态声明不一致，必须被检出");
        assert!(error.to_string().contains("能力矩阵与静态声明不一致"));
    }

    #[test]
    fn unsupported_capability_returns_explicit_rejection() {
        let registry = registry_with_all_basic();
        // Qoder 没有注册 Branchable：必须给出与现网一致的显式拒绝。
        let error = registry
            .branchable(&target(ProviderId::Qoder))
            .err()
            .expect("必须拒绝");
        assert_eq!(error.to_string(), "Qoder CN 不支持分支。");

        let error = registry
            .duplicable(&target(ProviderId::Qoder))
            .err()
            .expect("必须拒绝");
        assert_eq!(error.to_string(), "Qoder CN 不支持复制。");

        let error = registry
            .exportable(&target(ProviderId::Qoder))
            .err()
            .expect("必须拒绝");
        assert_eq!(error.to_string(), "Qoder CN 不支持导出。");
    }

    #[test]
    fn unsupported_capability_for_an_unregistered_provider_reports_unknown_platform() {
        let registry = ProviderRegistry::new();
        let error = registry
            .branchable(&target(ProviderId::Gemini))
            .err()
            .expect("必须失败");
        assert!(error.to_string().contains("未知 AI 平台"));
    }

    #[test]
    fn qoder_declares_the_narrowest_capability_set() {
        let caps = ProviderId::Qoder.capabilities();
        assert!(!caps.branch && !caps.duplicate && !caps.vendor_management);
        // 但回收站、批量操作与导出仍然可用。
        assert!(caps.trash && caps.batch_actions && caps.export);
    }
}
