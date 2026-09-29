//! 供应商管理命令。
//!
//! # 覆盖范围
//!
//! 只接**纯仓储**的那几条：列表、保存、删除、启停。余额查询、模型列表、配置模板读取、
//! 路由切换都需要发起 HTTP 或访问网关的内存路由表，因此留在 `PENDING` 里显式报未接线——
//! 静默返回空值会让界面显示「没有供应商」这种错误结论。
//!
//! # 启用与停用的差别
//!
//! `vendor:set-enabled` 只改启用位；`vendor:enable` 额外记录**启用时刻**。后者用于候选池
//! 排序（最近启用的优先），因此是两条命令而不是一条。

use serde_json::{Value, json};

use aivc_core::error::Result;
use aivc_core::id::VendorId;
use aivc_core::time::Timestamp;
use aivc_core::vendor::{ApiVendor, ApiVendorInput};
use aivc_provider::ProviderContext;

use crate::validate::{require_boolean, require_string};

/// 供应商命令。
pub struct VendorCommands;

impl VendorCommands {
    /// 列出供应商；`providerId` 可选，缺省列出全部平台。
    ///
    /// # Errors
    ///
    /// 平台标识非法或数据库读取失败。
    pub async fn list(ctx: &ProviderContext, args: &Value) -> Result<Vec<ApiVendor>> {
        // 必须用严格解析，不能用 `ProviderId::from_target_id`：后者是为 `targetId` 设计的，
        // 对没有前缀的字符串（如裸 `"claude"`）会**兜底成 Codex**。用在 `providerId` 上会
        // 让「查询 Claude 供应商」返回 Codex 的结果——静默出错，比报错更糟。
        let provider = match args
            .get("providerId")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        {
            Some(_) => Some(crate::validate::require_provider_id(
                args.get("providerId"),
            )?),
            None => None,
        };
        ctx.vendor().list(provider).await
    }

    /// 保存供应商（新建或更新）。
    ///
    /// # 调用方没给 `configs` 时补上平台的默认模板
    ///
    /// 「启用供应商」在后端的含义是**把它的凭据写进 CLI 的配置文件**
    /// （见 [`VendorCommands::enable`]），而那条命令要求至少一个启用的配置模板。
    /// 界面此前从不设 `configs`，于是供应商能保存、**但永远启用不了**：
    /// 报错是 `请至少启用一个配置文件。`，而用户根本没有「配置文件」这个概念。
    ///
    /// 因此这里兜底填上该平台的标准目标（[`aivc_tooling::vendor::default_vendor_configs`]）。
    /// 调用方**显式给了** `configs` 就一律用它的——包括给空数组表示
    /// 「这个供应商就是不要写配置」（此时启用会照旧被后端拒绝，那是调用方的选择）。
    ///
    /// # Errors
    ///
    /// 名称为空、Key/地址缺失、费率非法、重名或排序冲突时返回领域错误。
    pub async fn save(ctx: &ProviderContext, args: &Value) -> Result<ApiVendor> {
        let mut input: ApiVendorInput = serde_json::from_value(args.clone()).map_err(|error| {
            aivc_core::Error::Ipc(aivc_core::error::IpcError::CommandFailed {
                message: format!("参数无效：api vendor（{error}）"),
            })
        })?;
        // 只有「调用方压根没提 configs 这个字段」才兜底。显式传了（哪怕是空数组）
        // 都尊重调用方——空数组是一个有意义的表达。
        let provided_configs = args.get("configs").is_some();
        if !provided_configs {
            input.configs = aivc_tooling::vendor::default_vendor_configs(input.provider_id)
                .into_iter()
                .map(
                    |(target_path, content)| aivc_core::vendor::VendorConfigTemplate {
                        id: None,
                        provider_id: input.provider_id,
                        label: None,
                        enabled: true,
                        target_path,
                        content,
                    },
                )
                .collect();
        }
        ctx.vendor().save(&input).await
    }

    /// 删除供应商。
    ///
    /// # Errors
    ///
    /// 数据库写入失败。
    pub async fn delete(ctx: &ProviderContext, args: &Value) -> Result<bool> {
        let id = VendorId::from(require_string(args.get("id"), "id")?);
        ctx.vendor().delete(&id).await
    }

    /// 只切换启用位。
    ///
    /// # Errors
    ///
    /// 供应商不存在或数据库写入失败。
    pub async fn set_enabled(ctx: &ProviderContext, args: &Value) -> Result<ApiVendor> {
        let id = VendorId::from(require_string(args.get("id"), "id")?);
        let enabled = require_boolean(args.get("enabled"), "enabled")?;
        ctx.vendor().set_enabled(&id, enabled).await
    }

    /// 启用并记录启用时刻。
    ///
    /// 启用时刻用于候选池排序（最近启用的优先），因此不能在 `set_enabled` 里顺手写——
    /// 停用再启用会刷新它，而单纯改启用位不应该影响排序。
    ///
    /// # Errors
    ///
    /// 供应商不存在或数据库写入失败。
    pub async fn enable(ctx: &ProviderContext, args: &Value) -> Result<Value> {
        let id = VendorId::from(require_string(args.get("id"), "id")?);
        let at = args
            .get("at")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map_or_else(Timestamp::now, Timestamp::from_raw);

        // 先找供应商：启用要写它声明的配置文件，没有配置就没有可写的东西。
        let vendor = ctx
            .vendor()
            .list(None)
            .await?
            .into_iter()
            .find(|vendor| vendor.id == id)
            .ok_or_else(|| failed("供应商不存在。".to_owned()))?;

        let active: Vec<&aivc_core::vendor::VendorConfigTemplate> = vendor
            .configs
            .iter()
            .filter(|config| config.enabled)
            .collect();
        if active.is_empty() {
            return Err(failed("请至少启用一个配置文件。".to_owned()));
        }

        // 每次启用用一个新的时间戳目录：反复启用时，**每一次的历史内容都值得保留**。
        //
        // `backup_root` 接收的是**父目录**（它自己会拼一层 `vendor-backups`），
        // 因此这里传 `data/` 而不是 `ctx.vendor_backup_root()`——后者本身已经
        // 是 `data/vendor-backups`，再传进去会得到 `data/vendor-backups/vendor-backups/...`。
        // 实测就是这样：备份真的落在那个重复了一层的目录里。
        let backup_root =
            aivc_tooling::vendor::backup_root(&ctx.paths().data_dir(), &vendor.name, at.as_str());

        // 先写完全部文件，再改数据库。
        //
        // 顺序不能颠倒：反过来的话，写文件失败会留下「数据库说已启用、但配置没写进去」
        // 的状态——界面显示已启用而 CLI 实际用的是旧配置，这是最难排查的一类不一致。
        let mut written: Vec<String> = Vec::new();
        for config in active {
            let content = aivc_tooling::vendor::normalize_config_content(
                config.provider_id,
                &config.target_path,
                &config.content,
            );
            let rendered = aivc_tooling::vendor::render_template(
                &content,
                &vendor.api_key,
                &vendor.api_base_url,
                &vendor.name,
            );
            // 空内容会把用户配置**清空**——比写入失败严重得多，因为它看起来是成功的。
            if rendered.trim().is_empty() {
                return Err(failed(format!(
                    "配置文件内容为空，已中止写入：{}",
                    config.target_path
                )));
            }
            aivc_tooling::vendor::write_config_file(
                &config.target_path,
                &rendered,
                &backup_root,
                "local",
                // 主目录**由上下文给**：生产是真实 HOME，测试是临时目录。
                // 此前这里在展开函数里读 HOME，于是测试能改掉用户的真实配置。
                &ctx.config_home(),
            )
            .map_err(|error| {
                // 中途失败时**如实报告已经写了哪些**：用户需要知道当前处于什么状态，
                // 以及从哪里恢复。已写入的文件都有备份。
                if written.is_empty() {
                    failed(error)
                } else {
                    failed(format!("{error}（此前已写入：{}）", written.join("、")))
                }
            })?;
            written.push(config.target_path.clone());
        }

        let vendor = ctx.vendor().set_enabled(&id, true).await?;
        ctx.vendor().mark_enabled_at(&id, &at).await?;

        Ok(json!({
            "vendorId": vendor.id.as_str(),
            "written": written,
            "backupRoot": backup_root.to_string_lossy(),
            "lastEnabledAt": vendor.last_enabled_at.as_ref().map(|value| value.as_str().to_owned()),
        }))
    }
}

/// 查询类命令：模型列表与余额。
///
/// 与增删改分开：这些方法要发网络请求，而增删改只碰数据库。
impl VendorCommands {
    /// 按 `vendorId` 找供应商。
    async fn require_vendor(ctx: &ProviderContext, args: &Value) -> Result<ApiVendor> {
        let id = require_string(args.get("vendorId"), "vendorId")?;
        let vendors = ctx.vendor().list(None).await?;
        vendors
            .into_iter()
            .find(|vendor| vendor.id.as_str() == id.trim())
            .ok_or_else(|| failed("供应商不存在。".to_owned()))
    }

    /// 列出某个供应商的可用模型。
    ///
    /// # Errors
    ///
    /// 供应商不存在、地址无效或查询失败时返回错误。
    pub async fn list_models(ctx: &ProviderContext, args: &Value) -> Result<Value> {
        let vendor = Self::require_vendor(ctx, args).await?;
        let fetcher = crate::http_fetcher::ReqwestFetcher::new();
        let config = vendor.model_query.as_ref();
        let models = aivc_tooling::vendor_query::query_vendor_models(
            &fetcher,
            &vendor.api_base_url,
            vendor.provider_id,
            &vendor.api_key,
            config.and_then(|value| value.endpoint.as_deref()),
            config.and_then(|value| value.auth_mode).map(map_auth_mode),
        )
        .await
        .map_err(failed)?;
        Ok(Value::Array(
            models.iter().map(model_to_json).collect::<Vec<_>>(),
        ))
    }

    /// 读回配置文件内容。
    ///
    /// # 读取失败不算错误
    ///
    /// 某个文件不存在或读不了时返回**空内容**：配置预览是辅助信息，
    /// 不该阻断供应商表单——用户仍然应该能编辑并保存。
    ///
    /// # Errors
    ///
    /// 路径不在白名单内时返回错误。
    pub fn read_configs(args: &Value, home: &std::path::Path) -> Result<Value> {
        let paths = args
            .get("paths")
            .and_then(Value::as_array)
            .ok_or_else(|| failed("参数无效：paths".to_owned()))?;
        let mut files = Vec::with_capacity(paths.len());
        for path in paths {
            let path = path
                .as_str()
                .ok_or_else(|| failed("参数无效：paths".to_owned()))?;
            // 路径白名单在这里同样强制：不加限制的话，这条命令就成了
            // 「读取任意文件」的入口——配置文件里通常有 API Key。
            let content = aivc_tooling::vendor::read_config_file(path, home).map_err(failed)?;
            files.push(json!({ "path": path, "content": content }));
        }
        Ok(json!({ "files": files }))
    }
    /// 刷新单个供应商的余额。
    ///
    /// # Errors
    ///
    /// 供应商不存在、地址无效或查询失败时返回错误。
    pub async fn refresh_balance(ctx: &ProviderContext, args: &Value) -> Result<Value> {
        let vendor = Self::require_vendor(ctx, args).await?;
        let snapshot = query_balance(&vendor).await.map_err(failed)?;
        Ok(json!({
            "vendorId": vendor.id.as_str(),
            "balance": snapshot_to_json(&snapshot),
        }))
    }

    /// 刷新全部供应商的余额。
    ///
    /// # 并发上限
    ///
    /// 一次刷新十几个供应商时，全并发会给中转站造成瞬时压力，容易被判成滥用。
    /// 因此按 [`BALANCE_BATCH_CONCURRENCY`] 分批。
    ///
    /// # Errors
    ///
    /// 数据库读取失败时返回错误。**单个供应商查询失败不算整体失败**——
    /// 结果里带上各自的错误信息，否则一个失效的供应商会让整批刷新失败。
    pub async fn refresh_balances(ctx: &ProviderContext) -> Result<Value> {
        let vendors = ctx.vendor().list(None).await?;
        let mut results = Vec::with_capacity(vendors.len());
        for chunk in vendors.chunks(aivc_tooling::vendor_balance::BALANCE_BATCH_CONCURRENCY) {
            let mut futures = Vec::with_capacity(chunk.len());
            for vendor in chunk {
                futures.push(query_balance(vendor));
            }
            for (vendor, outcome) in chunk.iter().zip(futures) {
                results.push(match outcome.await {
                    Ok(snapshot) => json!({
                        "vendorId": vendor.id.as_str(),
                        "balance": snapshot_to_json(&snapshot),
                    }),
                    Err(error) => json!({
                        "vendorId": vendor.id.as_str(),
                        "error": error,
                    }),
                });
            }
        }
        Ok(Value::Array(results))
    }
}

/// 查询单个供应商的余额。
async fn query_balance(
    vendor: &ApiVendor,
) -> std::result::Result<aivc_tooling::vendor_balance::BalanceSnapshot, String> {
    let fetcher = crate::http_fetcher::ReqwestFetcher::new();
    let config = vendor.balance_query.as_ref();
    let query = aivc_tooling::vendor_query::BalanceQueryConfig {
        template: config.and_then(|value| value.template).map_or(
            aivc_tooling::vendor_query::BalanceTemplate::Generic,
            |value| match value {
                aivc_core::vendor::BalanceQueryTemplate::NewApi => {
                    aivc_tooling::vendor_query::BalanceTemplate::NewApi
                }
                aivc_core::vendor::BalanceQueryTemplate::Custom => {
                    aivc_tooling::vendor_query::BalanceTemplate::Custom
                }
                // `Auto` 与 `Generic` 都走通用模板：自动识别就是按候选路径找字段。
                aivc_core::vendor::BalanceQueryTemplate::Auto
                | aivc_core::vendor::BalanceQueryTemplate::Generic => {
                    aivc_tooling::vendor_query::BalanceTemplate::Generic
                }
            },
        ),
        endpoint: config.and_then(|value| value.endpoint.clone()),
        auth_mode: config.and_then(|value| value.auth_mode).map(map_auth_mode),
        multiplier: config.and_then(|value| value.multiplier),
        paths: Vec::new(),
    };
    aivc_tooling::vendor_query::query_vendor_balance(
        &fetcher,
        &vendor.api_base_url,
        vendor.provider_id,
        &vendor.api_key,
        &query,
    )
    .await
}

/// 命令面的认证方式转查询层。
fn map_auth_mode(mode: aivc_core::vendor::QueryAuthMode) -> aivc_tooling::vendor_models::AuthMode {
    use aivc_core::vendor::QueryAuthMode as From;
    use aivc_tooling::vendor_models::AuthMode as To;
    match mode {
        From::Bearer => To::Bearer,
        From::XApiKey => To::XApiKey,
        From::XGoogApiKey => To::XGoogApiKey,
        From::ApiKey => To::ApiKey,
        From::Query => To::Query,
        From::None => To::None,
    }
}

/// 模型转命令面的 JSON（字段名与现网 `VendorModel` 一致）。
fn model_to_json(model: &aivc_tooling::vendor_models::VendorModel) -> Value {
    json!({
        "id": model.id,
        "object": model.object,
        "ownedBy": model.owned_by,
        "created": model.created,
        "description": model.description,
    })
}

/// 余额快照转命令面的 JSON。
fn snapshot_to_json(snapshot: &aivc_tooling::vendor_balance::BalanceSnapshot) -> Value {
    json!({
        "remaining": snapshot.remaining,
        "total": snapshot.total,
        "used": snapshot.used,
        "unit": snapshot.unit,
        "planName": snapshot.plan_name,
        "isValid": snapshot.is_valid,
    })
}

/// 把领域层的字符串错误包成命令面的错误。
fn failed(message: String) -> aivc_core::Error {
    aivc_core::Error::Ipc(aivc_core::error::IpcError::CommandFailed { message })
}
#[cfg(test)]
mod tests {
    use super::*;
    use aivc_core::target::Platform;
    use aivc_infra::config::{AppPaths, SettingsStore};
    use aivc_infra::db::Db;
    use aivc_infra::wsl::{LocalBashLauncher, WslAdapter, WslConfig};
    use aivc_provider::ProviderRepositories;
    use serde_json::json;
    use std::sync::Arc;

    async fn context() -> (ProviderContext, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = AppPaths::new(dir.path().join("runtime"));
        for required in paths.required_directories() {
            std::fs::create_dir_all(&required).expect("dirs");
        }
        let db = Db::open(paths.database_path()).await.expect("db");
        let wsl = WslAdapter::new(
            Arc::new(LocalBashLauncher::new("definitely-not-a-real-bash")),
            WslConfig {
                use_worker: false,
                ..WslConfig::default()
            },
        );
        let ctx = ProviderContext::new(
            paths,
            SettingsStore::new(dir.path().join("runtime/settings.json")),
            ProviderRepositories::from_db(db),
            wsl,
            None,
            Platform::current(),
        )
        /*
          **把写 CLI 配置的主目录钉在临时目录里。**
          不设这一行的话，`config_home()` 会返回真实 HOME，于是任何走到
          「启用供应商」成功路径的测试都会去改用户的 `~/.codex/...`。
          实测发生过一次：用户的 `auth.json` 与 `config.toml` 被覆盖成
          `vendor_input` 里的假值，备份随 `TempDir` 一起被删，无法恢复。
          设了之后这是**结构上的保证**：这个上下文里的代码写到临时目录之外都不可能。
        */
        .with_config_home(dir.path().join("home"));
        (ctx, dir)
    }

    fn vendor_input(id: Option<&str>, name: &str) -> Value {
        let mut value = json!({
            "providerId": "codex",
            "name": name,
            "apiKey": "sk-test",
            "apiBaseUrl": "https://api.example.com",
        });
        if let Some(id) = id {
            value["id"] = json!(id);
        }
        value
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn saving_without_configs_fills_the_platform_defaults() {
        // **这是「启用」能成功的前提。**
        //
        // `enable` 要求至少一个启用的配置模板（`请至少启用一个配置文件。`），
        // 而界面此前从不设 `configs`——于是供应商能保存、**永远启用不了**，
        // 报错还指向一个用户没有的概念。
        let (ctx, _dir) = context().await;
        let saved = VendorCommands::save(&ctx, &vendor_input(None, "默认模板"))
            .await
            .expect("save");
        let paths: Vec<&str> = saved
            .configs
            .iter()
            .map(|config| config.target_path.as_str())
            .collect();
        assert_eq!(paths, vec!["~/.codex/auth.json", "~/.codex/config.toml"]);
        assert!(saved.configs.iter().all(|config| config.enabled));
        // 模板内容要带上占位符：没有它写进去的就是一份没凭据的配置。
        assert!(
            saved
                .configs
                .iter()
                .any(|config| config.content.contains("{{API_KEY}}"))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_explicit_empty_configs_is_respected() {
        // 显式传空数组表示「这个供应商就是不要写配置」，不能被兜底覆盖。
        let (ctx, _dir) = context().await;
        let mut input = vendor_input(None, "不要模板");
        input["configs"] = json!([]);
        let saved = VendorCommands::save(&ctx, &input).await.expect("save");
        assert!(saved.configs.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vendors_round_trip_through_the_command_surface() {
        let (ctx, _dir) = context().await;
        assert!(
            VendorCommands::list(&ctx, &json!({}))
                .await
                .expect("list")
                .is_empty()
        );

        let saved = VendorCommands::save(&ctx, &vendor_input(None, "主供应商"))
            .await
            .expect("save");
        assert_eq!(saved.name, "主供应商");

        let items = VendorCommands::list(&ctx, &json!({})).await.expect("list");
        assert_eq!(items.len(), 1);

        // 按平台筛选。
        let codex = VendorCommands::list(&ctx, &json!({ "providerId": "codex" }))
            .await
            .expect("list");
        assert_eq!(codex.len(), 1);
        let claude = VendorCommands::list(&ctx, &json!({ "providerId": "claude" }))
            .await
            .expect("list");
        assert!(claude.is_empty());

        assert!(
            VendorCommands::delete(&ctx, &json!({ "id": saved.id.as_str() }))
                .await
                .expect("delete")
        );
        assert!(
            VendorCommands::list(&ctx, &json!({}))
                .await
                .expect("list")
                .is_empty()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn saving_an_existing_vendor_updates_it() {
        let (ctx, _dir) = context().await;
        let saved = VendorCommands::save(&ctx, &vendor_input(None, "旧名"))
            .await
            .expect("create");
        let updated = VendorCommands::save(&ctx, &vendor_input(Some(saved.id.as_str()), "新名"))
            .await
            .expect("update");
        assert_eq!(updated.id.as_str(), saved.id.as_str());
        assert_eq!(updated.name, "新名");
        assert_eq!(
            VendorCommands::list(&ctx, &json!({}))
                .await
                .expect("list")
                .len(),
            1,
            "更新不应新增一条"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_empty_api_key_is_rejected() {
        // 校验在领域层，命令面负责把参数搬过去。
        let (ctx, _dir) = context().await;
        let mut value = vendor_input(None, "无 Key");
        value["apiKey"] = json!("");
        let error = VendorCommands::save(&ctx, &value)
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "API Key 不能为空。");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn set_enabled_only_toggles_the_flag() {
        let (ctx, _dir) = context().await;
        let saved = VendorCommands::save(&ctx, &vendor_input(None, "V"))
            .await
            .expect("save");
        let disabled = VendorCommands::set_enabled(
            &ctx,
            &json!({ "id": saved.id.as_str(), "enabled": false }),
        )
        .await
        .expect("disable");
        assert!(!disabled.enabled);
        // 只改启用位时不该写入启用时刻。
        assert!(disabled.last_enabled_at.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn enabling_without_an_enabled_config_is_rejected() {
        // 启用要写它声明的配置文件；没有配置就没有可写的东西。
        //
        // # 这条测试差点改掉用户的真实 CLI 配置
        //
        // `enable` 的成功路径会调 `write_config_file`，而它把 `~/` 展开成**真实的
        // HOME**——运行根是临时目录也拦不住它。原来这条测试靠「`save` 不填 `configs`」
        // 走到拒绝分支，因此是安全的。
        //
        // 后来 `save` 改成**兜底填平台的默认模板**（否则界面点「启用」必然失败），
        // 这条测试于是走进了成功路径：实测把用户的 `~/.codex/auth.json` 与
        // `config.toml` 覆盖成了测试里那两个假值（`sk-test` /
        // `https://api.example.com`），而备份写进了随 `TempDir` 一起删掉的目录里。
        //
        // 现在显式传**空** `configs`，让这条测试重新停在拒绝分支上。
        // **任何测试都不许走到 `enable` 的成功路径。**
        let (ctx, _dir) = context().await;
        let mut input = vendor_input(None, "V");
        input["configs"] = json!([]);
        let saved = VendorCommands::save(&ctx, &input).await.expect("save");
        let error = VendorCommands::enable(&ctx, &json!({ "id": saved.id.as_str() }))
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "请至少启用一个配置文件。");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn enabling_a_missing_vendor_is_rejected() {
        let (ctx, _dir) = context().await;
        let error = VendorCommands::enable(&ctx, &json!({ "id": "nope" }))
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "供应商不存在。");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_config_outside_the_whitelist_is_rejected_before_any_write() {
        // 路径白名单在这个编排里也要生效。用一个**绝不允许写入**的路径，
        // 因此这条测试在任何机器上都不会改到真实文件。
        let (ctx, _dir) = context().await;
        let mut input = vendor_input(None, "V");
        // `vendor_input` 返回命令面的 JSON，因此配置也按 JSON 形状追加。
        input["configs"] = json!([{
            "providerId": "codex",
            "enabled": true,
            "targetPath": "/etc/passwd",
            "content": "OPENAI_API_KEY={{API_KEY}}",
        }]);
        let saved = VendorCommands::save(&ctx, &input).await.expect("save");
        let error = VendorCommands::enable(&ctx, &json!({ "id": saved.id.as_str() }))
            .await
            .expect_err("必须拒绝");
        // 报错必须发生在写入之前——已写入列表为空。
        assert!(
            !error.to_string().contains("此前已写入"),
            "不该有任何文件被写入：{error}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn missing_ids_are_rejected_before_touching_the_database() {
        let (ctx, _dir) = context().await;
        for args in [json!({}), json!({ "id": "  " })] {
            let error = VendorCommands::delete(&ctx, &args)
                .await
                .expect_err("必须拒绝");
            assert_eq!(error.to_string(), "参数无效：id");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn deleting_an_unknown_vendor_reports_false() {
        let (ctx, _dir) = context().await;
        assert!(
            !VendorCommands::delete(&ctx, &json!({ "id": "missing" }))
                .await
                .expect("delete")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_duplicate_name_in_the_same_provider_is_rejected() {
        let (ctx, _dir) = context().await;
        VendorCommands::save(&ctx, &vendor_input(None, "同名"))
            .await
            .expect("first");
        let error = VendorCommands::save(&ctx, &vendor_input(None, "同名"))
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "供应商名称已存在：同名");
    }
}
