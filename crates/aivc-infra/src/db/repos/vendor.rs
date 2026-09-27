//! 供应商仓储。
//!
//! 供应商是本应用唯一会**写入用户 CLI 配置文件**的功能，因此这里的读写语义最需要
//! 精确对齐现网。核心不变量：
//!
//! 1. **同一平台内 `name_norm` 唯一**、**同一平台内 `sort` 唯一**；两者冲突都要给出
//!    可读错误，而不是让底层唯一索引抛错。
//! 2. **更新时留空 API Key 表示沿用旧值**——用户改地址时不必重新粘贴密钥。
//! 3. **配置模板整体替换**：先删后插，且每次插入都生成新的行编号（渲染层默认模板
//!    带固定 ID，直接复用会撞全局主键）。
//! 4. **排序在平台内自增**：新建时若未指定 `sort`，取该平台 `MAX(sort) + 1`。
//!
//! # 读时的关联字段
//!
//! `balance` / `balance_status` / `gateway_health` 不存于 `api_vendors`，而是读时从
//! 余额快照表与健康表关联。这里用「一次批量查询 + Rust 侧拼接」而不是逐条 JOIN，
//! 避免 N+1：供应商列表通常几十条，但每条都要带 3 个关联对象。

use std::collections::BTreeMap;

use aivc_core::error::{Result, VendorError};
use aivc_core::gateway::GatewayVendorHealth;
use aivc_core::id::VendorId;
use aivc_core::target::ProviderId;
use aivc_core::time::Timestamp;
use aivc_core::vendor::{
    ApiVendor, ApiVendorInput, BalanceQueryConfig, BalanceSnapshot, BalanceStatus,
    ModelQueryConfig, NormalizedVendorInput, VendorConfigTemplate, VendorPricing,
    normalize_vendor_name,
};
use rusqlite::{Connection, OptionalExtension, params};

use crate::db::Db;
use crate::db::migrations::sqlite_error;

/// 供应商仓储。
#[derive(Debug, Clone)]
pub struct VendorRepo {
    db: Db,
}

impl VendorRepo {
    /// 由数据库句柄构造。
    #[must_use]
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// 列出供应商。按 `sort ASC, created_at ASC` 排序，可选按平台过滤。
    pub async fn list(&self, provider: Option<ProviderId>) -> Result<Vec<ApiVendor>> {
        self.db
            .read(move |connection| list_in_connection(connection, provider))
            .await
    }

    /// 列出某个平台下**可参与网关候选池**的供应商。
    ///
    /// 资格条件与现网一致：同平台、已启用、API Key 非空、地址非空。
    pub async fn list_route_eligible(&self, provider: ProviderId) -> Result<Vec<ApiVendor>> {
        Ok(self
            .list(Some(provider))
            .await?
            .into_iter()
            .filter(ApiVendor::is_route_eligible)
            .collect())
    }

    /// 读取单个供应商。
    pub async fn get(&self, id: &VendorId) -> Result<Option<ApiVendor>> {
        let id = id.clone();
        self.db
            .read(move |connection| {
                let vendors = list_in_connection(connection, None)?;
                Ok(vendors.into_iter().find(|vendor| vendor.id == id))
            })
            .await
    }

    /// 保存供应商（新建或更新），返回保存后的完整记录。
    pub async fn save(&self, input: &ApiVendorInput) -> Result<ApiVendor> {
        let allow_empty_api_key = input.id.is_some();
        let normalized = input.normalize(allow_empty_api_key)?;
        self.db
            .write(move |connection| save_in_connection(connection, &normalized))
            .await
    }

    /// 删除供应商（配置模板与查询配置级联删除）。
    pub async fn delete(&self, id: &VendorId) -> Result<bool> {
        let id = id.clone();
        self.db
            .write(move |connection| {
                // 显式删除关联表，不依赖 `ON DELETE CASCADE`（PRAGMA 是按连接设置的，
                // 一旦某条连接漏设外键，级联会静默失效，留下孤儿行）。
                connection
                    .execute(
                        "DELETE FROM api_vendor_query_configs WHERE vendor_id = ?1",
                        params![id.as_str()],
                    )
                    .map_err(sqlite_error)?;
                connection
                    .execute(
                        "DELETE FROM api_vendor_configs WHERE vendor_id = ?1",
                        params![id.as_str()],
                    )
                    .map_err(sqlite_error)?;
                connection
                    .execute(
                        "DELETE FROM api_vendor_balance_snapshots WHERE vendor_id = ?1",
                        params![id.as_str()],
                    )
                    .map_err(sqlite_error)?;
                let removed = connection
                    .execute(
                        "DELETE FROM api_vendors WHERE id = ?1",
                        params![id.as_str()],
                    )
                    .map_err(sqlite_error)?;
                Ok(removed > 0)
            })
            .await
    }

    /// 设置「是否参与候选池」。
    pub async fn set_enabled(&self, id: &VendorId, enabled: bool) -> Result<ApiVendor> {
        let id = id.clone();
        self.db
            .write(move |connection| {
                let updated = connection
                    .execute(
                        "UPDATE api_vendors SET enabled = ?1, updated_at = ?2 WHERE id = ?3",
                        params![i64::from(enabled), Timestamp::now().as_str(), id.as_str()],
                    )
                    .map_err(sqlite_error)?;
                if updated == 0 {
                    return Err(VendorError::NotFound.into());
                }
                let vendors = list_in_connection(connection, None)?;
                vendors
                    .into_iter()
                    .find(|vendor| vendor.id == id)
                    .ok_or_else(|| VendorError::NotFound.into())
            })
            .await
    }

    /// 记录「最近一次启用时间」。
    pub async fn mark_enabled_at(&self, id: &VendorId, at: &Timestamp) -> Result<()> {
        let id = id.clone();
        let at = at.clone();
        self.db
            .write(move |connection| {
                connection
                    .execute(
                        "UPDATE api_vendors SET last_enabled_at = ?1, updated_at = ?2 WHERE id = ?3",
                        params![at.as_str(), Timestamp::now().as_str(), id.as_str()],
                    )
                    .map_err(sqlite_error)?;
                Ok(())
            })
            .await
    }

    /// 供应商总数（诊断与测试用）。
    pub async fn count(&self) -> Result<i64> {
        self.db
            .read(|connection| {
                connection
                    .query_row("SELECT COUNT(*) FROM api_vendors", [], |row| row.get(0))
                    .map_err(sqlite_error)
            })
            .await
    }
}

/// 在给定连接上读取供应商列表（含关联字段）。
fn list_in_connection(
    connection: &Connection,
    provider: Option<ProviderId>,
) -> Result<Vec<ApiVendor>> {
    let mut statement = connection
        .prepare(
            "SELECT id, provider_id, name, api_key, api_base_url,
                    input_price_usd, output_price_usd, sort, enabled,
                    created_at, updated_at, last_enabled_at
             FROM api_vendors
             WHERE (?1 IS NULL OR provider_id = ?1)
             ORDER BY sort ASC, created_at ASC",
        )
        .map_err(sqlite_error)?;

    let rows = statement
        .query_map(params![provider.map(ProviderId::as_str)], |row| {
            let provider_id: String = row.get(1)?;
            let input_price: Option<f64> = row.get(5)?;
            let output_price: Option<f64> = row.get(6)?;
            let pricing = if input_price.is_none() && output_price.is_none() {
                None
            } else {
                Some(VendorPricing {
                    input_per_million_usd: input_price,
                    output_per_million_usd: output_price,
                })
            };
            Ok(ApiVendor {
                id: VendorId::from(row.get::<_, String>(0)?),
                provider_id: provider_from_sql(&provider_id),
                name: row.get(2)?,
                api_key: row.get(3)?,
                api_base_url: row.get(4)?,
                sort: row.get(7)?,
                pricing,
                model_query: None,
                balance_query: None,
                configs: Vec::new(),
                enabled: row.get::<_, i64>(8)? != 0,
                created_at: Timestamp::from_raw(row.get::<_, String>(9)?),
                updated_at: Timestamp::from_raw(row.get::<_, String>(10)?),
                last_enabled_at: row.get::<_, Option<String>>(11)?.map(Timestamp::from_raw),
                balance: None,
                balance_status: None,
                balance_error: None,
                balance_queried_at: None,
                gateway_health: None,
            })
        })
        .map_err(sqlite_error)?;

    let mut vendors = Vec::new();
    for row in rows {
        vendors.push(row.map_err(sqlite_error)?);
    }
    if vendors.is_empty() {
        return Ok(vendors);
    }

    let configs = load_configs(connection)?;
    let query_configs = load_query_configs(connection)?;
    let balances = load_balances(connection)?;
    let health = load_health(connection)?;

    for vendor in &mut vendors {
        let key = vendor.id.as_str().to_owned();
        vendor.configs = configs.get(&key).cloned().unwrap_or_default();
        if let Some((model_query, balance_query)) = query_configs.get(&key) {
            vendor.model_query.clone_from(model_query);
            vendor.balance_query.clone_from(balance_query);
        }
        if let Some((snapshot, status, error, queried_at)) = balances.get(&key) {
            vendor.balance.clone_from(snapshot);
            vendor.balance_status = Some(*status);
            vendor.balance_error.clone_from(error);
            vendor.balance_queried_at.clone_from(queried_at);
        }
        vendor.gateway_health = health.get(&key).cloned();
    }
    Ok(vendors)
}

/// 一次性读取全部配置模板，按供应商编号分组。
fn load_configs(connection: &Connection) -> Result<BTreeMap<String, Vec<VendorConfigTemplate>>> {
    let mut statement = connection
        .prepare(
            "SELECT vendor_id, id, provider_id, label, enabled, target_path, content
             FROM api_vendor_configs
             ORDER BY vendor_id, sort_order ASC",
        )
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map([], |row| {
            let provider_id: String = row.get(2)?;
            Ok((
                row.get::<_, String>(0)?,
                VendorConfigTemplate {
                    id: Some(row.get::<_, String>(1)?),
                    provider_id: provider_from_sql(&provider_id),
                    label: row.get(3)?,
                    enabled: row.get::<_, i64>(4)? != 0,
                    target_path: row.get(5)?,
                    content: row.get(6)?,
                },
            ))
        })
        .map_err(sqlite_error)?;

    let mut grouped: BTreeMap<String, Vec<VendorConfigTemplate>> = BTreeMap::new();
    for row in rows {
        let (vendor_id, config) = row.map_err(sqlite_error)?;
        grouped.entry(vendor_id).or_default().push(config);
    }
    Ok(grouped)
}

/// 一次性读取全部查询配置。
type QueryConfigPair = (Option<ModelQueryConfig>, Option<BalanceQueryConfig>);

fn load_query_configs(connection: &Connection) -> Result<BTreeMap<String, QueryConfigPair>> {
    let mut statement = connection
        .prepare(
            "SELECT vendor_id, model_query_json, balance_query_json
             FROM api_vendor_query_configs",
        )
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(sqlite_error)?;

    let mut map = BTreeMap::new();
    for row in rows {
        let (vendor_id, model_json, balance_json) = row.map_err(sqlite_error)?;
        map.insert(
            vendor_id,
            (
                parse_json_lossy::<ModelQueryConfig>(model_json.as_deref()),
                parse_json_lossy::<BalanceQueryConfig>(balance_json.as_deref()),
            ),
        );
    }
    Ok(map)
}

/// 一次性读取全部余额快照。
type BalanceEntry = (
    Option<BalanceSnapshot>,
    BalanceStatus,
    Option<String>,
    Option<Timestamp>,
);

fn load_balances(connection: &Connection) -> Result<BTreeMap<String, BalanceEntry>> {
    let mut statement = connection
        .prepare(
            "SELECT vendor_id, remaining, total, used, unit, plan_name, is_valid,
                    status, error_message, queried_at
             FROM api_vendor_balance_snapshots",
        )
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                BalanceSnapshot {
                    remaining: row.get(1)?,
                    total: row.get(2)?,
                    used: row.get(3)?,
                    unit: row.get(4)?,
                    plan_name: row.get(5)?,
                    is_valid: row.get::<_, i64>(6)? != 0,
                },
                row.get::<_, String>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
            ))
        })
        .map_err(sqlite_error)?;

    let mut map = BTreeMap::new();
    for row in rows {
        let (vendor_id, snapshot, status, error, queried_at) = row.map_err(sqlite_error)?;
        map.insert(
            vendor_id,
            (
                Some(snapshot),
                BalanceStatus::from_str_lossy(&status),
                error,
                queried_at.map(Timestamp::from_raw),
            ),
        );
    }
    Ok(map)
}

/// 一次性读取全部健康状态。
fn load_health(connection: &Connection) -> Result<BTreeMap<String, GatewayVendorHealth>> {
    let mut statement = connection
        .prepare(
            "SELECT vendor_id, provider_id, status, failure_count, success_count, failure_rate,
                    last_failure_at, last_success_at, circuit_until, last_failure_reason
             FROM gateway_vendor_health",
        )
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map([], |row| {
            let provider_id: String = row.get(1)?;
            let status: String = row.get(2)?;
            let circuit_until: Option<String> = row.get(8)?;
            Ok((
                row.get::<_, String>(0)?,
                GatewayVendorHealth {
                    vendor_id: VendorId::from(row.get::<_, String>(0)?),
                    provider_id: provider_from_sql(&provider_id),
                    // 与健康仓储保持同一归一化：过期熔断视为 half-open。
                    status: crate::db::repos::gateway_health::normalize_status(
                        &status,
                        circuit_until.as_deref(),
                    ),
                    failure_count: u64::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
                    success_count: u64::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
                    failure_rate: row.get(5)?,
                    last_failure_at: row.get::<_, Option<String>>(6)?.map(Timestamp::from_raw),
                    last_success_at: row.get::<_, Option<String>>(7)?.map(Timestamp::from_raw),
                    circuit_until: circuit_until.map(Timestamp::from_raw),
                    last_failure_reason: row.get(9)?,
                },
            ))
        })
        .map_err(sqlite_error)?;

    let mut map = BTreeMap::new();
    for row in rows {
        let (vendor_id, health) = row.map_err(sqlite_error)?;
        map.insert(vendor_id, health);
    }
    Ok(map)
}

/// 保存供应商（在写连接内完成全部校验与写入）。
fn save_in_connection(
    connection: &Connection,
    normalized: &NormalizedVendorInput,
) -> Result<ApiVendor> {
    let transaction = connection.unchecked_transaction().map_err(sqlite_error)?;
    let existing_id = normalized.id.as_ref().map(|id| id.as_str().to_owned());

    // 已有记录（用于沿用 API Key、排序与创建时间）。
    let existing: Option<(String, String, i64, String, String)> = match existing_id.as_deref() {
        Some(id) => transaction
            .query_row(
                "SELECT id, provider_id, sort, api_key, created_at FROM api_vendors WHERE id = ?1",
                params![id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()
            .map_err(sqlite_error)?,
        None => None,
    };

    let name_norm = normalize_vendor_name(&normalized.name);
    let exclude_id = existing.as_ref().map_or("", |(id, ..)| id.as_str());

    // 同平台重名检查：给出可读错误，而不是让唯一索引抛底层错误。
    let duplicate_name: Option<String> = transaction
        .query_row(
            "SELECT id FROM api_vendors
             WHERE provider_id = ?1 AND name_norm = ?2 AND id <> ?3",
            params![normalized.provider_id.as_str(), name_norm, exclude_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(sqlite_error)?;
    if duplicate_name.is_some() {
        return Err(VendorError::DuplicateName {
            name: normalized.name.clone(),
        }
        .into());
    }

    // 排序：沿用旧排序（同平台时）或指定值，否则取该平台最大值 + 1。
    let sort = match (&existing, normalized.sort) {
        (Some((_, provider, sort, ..)), None) if provider == normalized.provider_id.as_str() => {
            *sort
        }
        (_, Some(sort)) => sort,
        _ => next_vendor_sort(&transaction, normalized.provider_id)?,
    };

    let duplicate_sort: Option<String> = transaction
        .query_row(
            "SELECT id FROM api_vendors WHERE provider_id = ?1 AND sort = ?2 AND id <> ?3",
            params![normalized.provider_id.as_str(), sort, exclude_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(sqlite_error)?;
    if duplicate_sort.is_some() {
        return Err(VendorError::DuplicateSort { sort }.into());
    }

    let now = Timestamp::now();
    let vendor = ApiVendor {
        id: existing.as_ref().map_or_else(
            || VendorId::from(uuid::Uuid::new_v4().to_string()),
            |(id, ..)| VendorId::from(id.clone()),
        ),
        provider_id: normalized.provider_id,
        name: normalized.name.clone(),
        // 留空表示沿用旧值。
        api_key: if normalized.api_key.is_empty() {
            existing
                .as_ref()
                .map_or_else(String::new, |(_, _, _, key, _)| key.clone())
        } else {
            normalized.api_key.clone()
        },
        api_base_url: normalized.api_base_url.clone(),
        sort,
        pricing: normalized.pricing,
        model_query: normalized.model_query.clone(),
        balance_query: normalized.balance_query.clone(),
        configs: normalized.configs.clone(),
        enabled: normalized.enabled,
        created_at: existing.as_ref().map_or_else(
            || now.clone(),
            |(_, _, _, _, created)| Timestamp::from_raw(created.clone()),
        ),
        updated_at: now,
        last_enabled_at: None,
        balance: None,
        balance_status: None,
        balance_error: None,
        balance_queried_at: None,
        gateway_health: None,
    };

    upsert_vendor(&transaction, &vendor, &name_norm)?;
    replace_configs(&transaction, &vendor)?;
    upsert_query_configs(&transaction, &vendor)?;

    transaction.commit().map_err(sqlite_error)?;
    Ok(vendor)
}

/// 整体替换配置模板（先删后插）。
///
/// 每次插入都使用归一化阶段生成的新行编号：渲染层默认模板带固定 ID，
/// 直接复用会撞 `api_vendor_configs` 的全局主键。
fn replace_configs(connection: &Connection, vendor: &ApiVendor) -> Result<()> {
    connection
        .execute(
            "DELETE FROM api_vendor_configs WHERE vendor_id = ?1",
            params![vendor.id.as_str()],
        )
        .map_err(sqlite_error)?;

    let mut statement = connection
        .prepare(
            "INSERT INTO api_vendor_configs
               (id, vendor_id, provider_id, label, enabled, target_path, content, sort_order)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .map_err(sqlite_error)?;
    for (index, config) in vendor.configs.iter().enumerate() {
        statement
            .execute(params![
                config
                    .id
                    .clone()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                vendor.id.as_str(),
                config.provider_id.as_str(),
                config.label,
                i64::from(config.enabled),
                config.target_path,
                config.content,
                i64::try_from(index).unwrap_or(i64::MAX),
            ])
            .map_err(sqlite_error)?;
    }
    Ok(())
}

/// 写入模型 / 余额查询配置。
fn upsert_query_configs(connection: &Connection, vendor: &ApiVendor) -> Result<()> {
    connection
        .execute(
            "INSERT INTO api_vendor_query_configs (vendor_id, model_query_json, balance_query_json)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(vendor_id) DO UPDATE SET
               model_query_json = excluded.model_query_json,
               balance_query_json = excluded.balance_query_json",
            params![
                vendor.id.as_str(),
                optional_json(vendor.model_query.as_ref()),
                optional_json(vendor.balance_query.as_ref()),
            ],
        )
        .map_err(sqlite_error)?;
    Ok(())
}

/// 写入供应商主行。
fn upsert_vendor(connection: &Connection, vendor: &ApiVendor, name_norm: &str) -> Result<()> {
    connection
        .execute(
            "INSERT INTO api_vendors
               (id, provider_id, name, name_norm, api_key, api_base_url,
                input_price_usd, output_price_usd, sort, enabled,
                created_at, updated_at, last_enabled_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(id) DO UPDATE SET
               provider_id = excluded.provider_id,
               name = excluded.name,
               name_norm = excluded.name_norm,
               api_key = excluded.api_key,
               api_base_url = excluded.api_base_url,
               input_price_usd = excluded.input_price_usd,
               output_price_usd = excluded.output_price_usd,
               sort = excluded.sort,
               enabled = excluded.enabled,
               updated_at = excluded.updated_at,
               last_enabled_at = excluded.last_enabled_at",
            params![
                vendor.id.as_str(),
                vendor.provider_id.as_str(),
                vendor.name,
                name_norm,
                vendor.api_key,
                vendor.api_base_url,
                vendor.pricing.and_then(|p| p.input_per_million_usd),
                vendor.pricing.and_then(|p| p.output_per_million_usd),
                vendor.sort,
                i64::from(vendor.enabled),
                vendor.created_at.as_str(),
                vendor.updated_at.as_str(),
                vendor.last_enabled_at.as_ref().map(Timestamp::as_str),
            ],
        )
        .map_err(sqlite_error)?;
    Ok(())
}

/// 该平台下一个可用排序值。
fn next_vendor_sort(connection: &Connection, provider: ProviderId) -> Result<i64> {
    let max_sort: Option<i64> = connection
        .query_row(
            "SELECT MAX(sort) FROM api_vendors WHERE provider_id = ?1",
            params![provider.as_str()],
            |row| row.get(0),
        )
        .map_err(sqlite_error)?;
    Ok(max_sort.unwrap_or(0) + 1)
}

/// 可选值序列化为 JSON 文本。
fn optional_json<T: serde::Serialize>(value: Option<&T>) -> Option<String> {
    value.and_then(|item| serde_json::to_string(item).ok())
}

/// 宽松解析 JSON；损坏时返回 [`None`] 而不是让整行读取失败。
fn parse_json_lossy<T: serde::de::DeserializeOwned>(value: Option<&str>) -> Option<T> {
    serde_json::from_str::<T>(value?).ok()
}

/// SQL 值 → 平台标识。
fn provider_from_sql(value: &str) -> ProviderId {
    ProviderId::ALL
        .into_iter()
        .find(|item| item.as_str() == value)
        .unwrap_or(ProviderId::Codex)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivc_core::gateway::GatewayVendorHealthStatus;
    use aivc_core::vendor::{HttpMethod, QueryAuthMode};

    async fn repo() -> (tempfile::TempDir, VendorRepo) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open(dir.path().join("aivc.db")).await.expect("open");
        (dir, VendorRepo::new(db))
    }

    fn input(name: &str, provider: ProviderId) -> ApiVendorInput {
        ApiVendorInput {
            provider_id: provider,
            name: name.to_owned(),
            api_key: "sk-test".to_owned(),
            api_base_url: "https://api.example.com".to_owned(),
            ..ApiVendorInput::default()
        }
    }

    #[tokio::test]
    async fn empty_database_lists_nothing() {
        let (_dir, repo) = repo().await;
        assert!(repo.list(None).await.expect("list").is_empty());
        assert_eq!(repo.count().await.expect("count"), 0);
    }

    #[tokio::test]
    async fn save_then_list_round_trips_all_fields() {
        let (_dir, repo) = repo().await;
        let mut raw = input("主供应商", ProviderId::Claude);
        raw.pricing = Some(VendorPricing {
            input_per_million_usd: Some(3.0),
            output_per_million_usd: Some(15.0),
        });
        raw.model_query = Some(ModelQueryConfig {
            endpoint: Some("/v1/models".to_owned()),
            auth_mode: Some(QueryAuthMode::XApiKey),
            ..ModelQueryConfig::default()
        });
        raw.balance_query = Some(BalanceQueryConfig {
            endpoint: Some("/api/user/self".to_owned()),
            method: Some(HttpMethod::Get),
            ..BalanceQueryConfig::default()
        });
        raw.configs = vec![VendorConfigTemplate {
            id: None,
            provider_id: ProviderId::Claude,
            label: Some("主配置".to_owned()),
            enabled: true,
            target_path: "~/.claude/settings.json".to_owned(),
            content: "{}".to_owned(),
        }];

        let saved = repo.save(&raw).await.expect("save");
        assert!(!saved.id.is_empty());
        assert_eq!(saved.sort, 1, "首个供应商排序应为 1");
        assert_eq!(
            saved.pricing.expect("pricing").input_per_million_usd,
            Some(3.0)
        );
        assert_eq!(saved.configs.len(), 1);

        let listed = repo.list(None).await.expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "主供应商");
        assert_eq!(
            listed[0]
                .model_query
                .as_ref()
                .and_then(|c| c.endpoint.as_deref()),
            Some("/v1/models")
        );
        assert_eq!(
            listed[0].balance_query.as_ref().and_then(|c| c.method),
            Some(HttpMethod::Get)
        );
        assert_eq!(listed[0].configs[0].label.as_deref(), Some("主配置"));
    }

    #[tokio::test]
    async fn sort_auto_increments_per_provider() {
        let (_dir, repo) = repo().await;
        let first = repo.save(&input("a", ProviderId::Codex)).await.expect("a");
        let second = repo.save(&input("b", ProviderId::Codex)).await.expect("b");
        // 不同平台的排序互不干扰，都从 1 开始。
        let other = repo.save(&input("c", ProviderId::Claude)).await.expect("c");

        assert_eq!(first.sort, 1);
        assert_eq!(second.sort, 2);
        assert_eq!(other.sort, 1, "不同平台的排序应独立自增");
    }

    #[tokio::test]
    async fn duplicate_name_within_a_provider_is_rejected() {
        let (_dir, repo) = repo().await;
        repo.save(&input("同名", ProviderId::Codex))
            .await
            .expect("first");

        let error = repo
            .save(&input("  同名  ", ProviderId::Codex))
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "供应商名称已存在：同名");

        // 大小写不同也视为重名（name_norm 归一化）。
        let error = repo
            .save(&input("同名", ProviderId::Codex))
            .await
            .expect_err("必须拒绝");
        assert!(error.to_string().contains("供应商名称已存在"));
    }

    #[tokio::test]
    async fn the_same_name_is_allowed_across_providers() {
        let (_dir, repo) = repo().await;
        repo.save(&input("同名", ProviderId::Codex))
            .await
            .expect("codex");
        // 不同平台是不同命名空间。
        repo.save(&input("同名", ProviderId::Claude))
            .await
            .expect("claude");
        assert_eq!(repo.count().await.expect("count"), 2);
    }

    #[tokio::test]
    async fn duplicate_sort_within_a_provider_is_rejected() {
        let (_dir, repo) = repo().await;
        repo.save(&input("a", ProviderId::Codex)).await.expect("a");

        let mut raw = input("b", ProviderId::Codex);
        raw.sort = Some(1);
        let error = repo.save(&raw).await.expect_err("必须拒绝");
        assert_eq!(error.to_string(), "排序值 1 已被占用。");
    }

    #[tokio::test]
    async fn explicit_sort_is_honoured() {
        let (_dir, repo) = repo().await;
        let mut raw = input("a", ProviderId::Codex);
        raw.sort = Some(50);
        let saved = repo.save(&raw).await.expect("save");
        assert_eq!(saved.sort, 50);

        // 下一个自动排序必须接在最大值之后。
        let next = repo.save(&input("b", ProviderId::Codex)).await.expect("b");
        assert_eq!(next.sort, 51);
    }

    #[tokio::test]
    async fn empty_api_key_on_update_keeps_the_stored_key() {
        let (_dir, repo) = repo().await;
        let saved = repo
            .save(&input("a", ProviderId::Codex))
            .await
            .expect("save");

        let mut raw = input("a", ProviderId::Codex);
        raw.id = Some(saved.id.clone());
        raw.api_key = "   ".to_owned();
        raw.api_base_url = "https://changed.example.com".to_owned();
        let updated = repo.save(&raw).await.expect("update");

        assert_eq!(updated.id, saved.id);
        assert_eq!(updated.api_key, "sk-test", "留空应沿用旧 Key");
        assert_eq!(updated.api_base_url, "https://changed.example.com");
        assert_eq!(updated.created_at, saved.created_at, "创建时间必须保留");
        assert_eq!(updated.sort, saved.sort, "未指定排序应沿用旧值");
    }

    #[tokio::test]
    async fn update_replaces_config_templates_wholesale() {
        let (_dir, repo) = repo().await;
        let mut raw = input("a", ProviderId::Codex);
        raw.configs = vec![
            VendorConfigTemplate {
                id: None,
                provider_id: ProviderId::Codex,
                label: Some("旧一".to_owned()),
                enabled: true,
                target_path: "~/.codex/config.toml".to_owned(),
                content: "1".to_owned(),
            },
            VendorConfigTemplate {
                id: None,
                provider_id: ProviderId::Codex,
                label: Some("旧二".to_owned()),
                enabled: true,
                target_path: "~/.codex/other.toml".to_owned(),
                content: "2".to_owned(),
            },
        ];
        let saved = repo.save(&raw).await.expect("save");
        assert_eq!(saved.configs.len(), 2);

        let mut update = input("a", ProviderId::Codex);
        update.id = Some(saved.id.clone());
        update.configs = vec![VendorConfigTemplate {
            id: None,
            provider_id: ProviderId::Codex,
            label: Some("新".to_owned()),
            enabled: false,
            target_path: "~/.codex/config.toml".to_owned(),
            content: "3".to_owned(),
        }];
        let updated = repo.save(&update).await.expect("update");

        assert_eq!(updated.configs.len(), 1, "配置模板是整体替换而不是追加");
        assert_eq!(updated.configs[0].label.as_deref(), Some("新"));
        assert!(!updated.configs[0].enabled);
        // 重新读取也必须只有一条。
        let listed = repo.list(None).await.expect("list");
        assert_eq!(listed[0].configs.len(), 1);
    }

    #[tokio::test]
    async fn query_configs_can_be_cleared_by_saving_without_them() {
        let (_dir, repo) = repo().await;
        let mut raw = input("a", ProviderId::Codex);
        raw.model_query = Some(ModelQueryConfig {
            endpoint: Some("/models".to_owned()),
            ..ModelQueryConfig::default()
        });
        let saved = repo.save(&raw).await.expect("save");
        assert!(saved.model_query.is_some());

        let mut update = input("a", ProviderId::Codex);
        update.id = Some(saved.id.clone());
        let cleared = repo.save(&update).await.expect("update");
        assert!(cleared.model_query.is_none(), "不传查询配置应清空旧值");
    }

    #[tokio::test]
    async fn list_can_filter_by_provider() {
        let (_dir, repo) = repo().await;
        repo.save(&input("codex-1", ProviderId::Codex))
            .await
            .expect("save");
        repo.save(&input("claude-1", ProviderId::Claude))
            .await
            .expect("save");

        assert_eq!(repo.list(None).await.expect("all").len(), 2);
        let codex = repo.list(Some(ProviderId::Codex)).await.expect("codex");
        assert_eq!(codex.len(), 1);
        assert_eq!(codex[0].name, "codex-1");
    }

    #[tokio::test]
    async fn list_is_ordered_by_sort() {
        let (_dir, repo) = repo().await;
        let mut late = input("late", ProviderId::Codex);
        late.sort = Some(9);
        repo.save(&late).await.expect("late");
        let mut early = input("early", ProviderId::Codex);
        early.sort = Some(2);
        repo.save(&early).await.expect("early");

        let names: Vec<String> = repo
            .list(Some(ProviderId::Codex))
            .await
            .expect("list")
            .into_iter()
            .map(|vendor| vendor.name)
            .collect();
        assert_eq!(names, vec!["early", "late"]);
    }

    #[tokio::test]
    async fn route_eligible_filters_incomplete_vendors() {
        let (_dir, repo) = repo().await;
        repo.save(&input("完整", ProviderId::Codex))
            .await
            .expect("ok");

        // 已禁用。
        let mut disabled = input("禁用", ProviderId::Codex);
        disabled.enabled = Some(false);
        repo.save(&disabled).await.expect("disabled");

        let eligible = repo
            .list_route_eligible(ProviderId::Codex)
            .await
            .expect("eligible");
        assert_eq!(eligible.len(), 1);
        assert_eq!(eligible[0].name, "完整");
    }

    #[tokio::test]
    async fn set_enabled_toggles_and_reports_missing_vendors() {
        let (_dir, repo) = repo().await;
        let saved = repo
            .save(&input("a", ProviderId::Codex))
            .await
            .expect("save");

        let disabled = repo.set_enabled(&saved.id, false).await.expect("disable");
        assert!(!disabled.enabled);
        assert!(
            !repo
                .get(&saved.id)
                .await
                .expect("get")
                .expect("present")
                .enabled
        );

        let enabled = repo.set_enabled(&saved.id, true).await.expect("enable");
        assert!(enabled.enabled);

        let error = repo
            .set_enabled(&VendorId::from("missing"), true)
            .await
            .expect_err("必须报错");
        assert_eq!(error.to_string(), "供应商不存在。");
    }

    #[tokio::test]
    async fn delete_removes_the_vendor_and_all_related_rows() {
        let (_dir, repo) = repo().await;
        let mut raw = input("a", ProviderId::Codex);
        raw.configs = vec![VendorConfigTemplate {
            id: None,
            provider_id: ProviderId::Codex,
            label: None,
            enabled: true,
            target_path: "~/.codex/config.toml".to_owned(),
            content: "x".to_owned(),
        }];
        raw.model_query = Some(ModelQueryConfig {
            endpoint: Some("/models".to_owned()),
            ..ModelQueryConfig::default()
        });
        let saved = repo.save(&raw).await.expect("save");

        assert!(repo.delete(&saved.id).await.expect("delete"));
        assert_eq!(repo.count().await.expect("count"), 0);
        assert!(repo.get(&saved.id).await.expect("get").is_none());
        assert!(!repo.delete(&saved.id).await.expect("delete again"));
    }

    #[tokio::test]
    async fn mark_enabled_at_is_persisted() {
        let (_dir, repo) = repo().await;
        let saved = repo
            .save(&input("a", ProviderId::Codex))
            .await
            .expect("save");
        assert!(saved.last_enabled_at.is_none());

        let at = Timestamp::from_raw("2026-09-16T12:00:00.000Z");
        repo.mark_enabled_at(&saved.id, &at).await.expect("mark");

        let reloaded = repo.get(&saved.id).await.expect("get").expect("present");
        assert_eq!(
            reloaded.last_enabled_at.as_ref().map(Timestamp::as_str),
            Some("2026-09-16T12:00:00.000Z")
        );
    }

    #[tokio::test]
    async fn corrupt_query_config_json_degrades_to_none() {
        let (dir, repo) = repo().await;
        let mut raw = input("a", ProviderId::Codex);
        raw.model_query = Some(ModelQueryConfig {
            endpoint: Some("/models".to_owned()),
            ..ModelQueryConfig::default()
        });
        let saved = repo.save(&raw).await.expect("save");

        let db = Db::open(dir.path().join("aivc.db")).await.expect("open");
        let id = saved.id.clone();
        db.write(move |connection| {
            connection
                .execute(
                    "UPDATE api_vendor_query_configs SET model_query_json = 'not json' WHERE vendor_id = ?1",
                    params![id.as_str()],
                )
                .map_err(sqlite_error)?;
            Ok(())
        })
        .await
        .expect("corrupt");

        // 损坏的查询配置不影响供应商主行。
        let vendor = repo.get(&saved.id).await.expect("get").expect("present");
        assert!(vendor.model_query.is_none());
        assert_eq!(vendor.name, "a");
    }

    #[tokio::test]
    async fn balance_snapshot_and_health_are_stitched_into_the_vendor() {
        let (dir, repo) = repo().await;
        let saved = repo
            .save(&input("a", ProviderId::Codex))
            .await
            .expect("save");

        let db = Db::open(dir.path().join("aivc.db")).await.expect("open");
        let id = saved.id.clone();
        db.write(move |connection| {
            connection
                .execute(
                    "INSERT INTO api_vendor_balance_snapshots
                       (vendor_id, remaining, total, used, unit, plan_name, is_valid, status, error_message, queried_at)
                     VALUES (?1, 12.5, 100.0, 87.5, 'USD', 'Pro', 1, 'success', NULL, '2026-09-16T12:00:00.000Z')",
                    params![id.as_str()],
                )
                .map_err(sqlite_error)?;
            connection
                .execute(
                    "INSERT INTO gateway_vendor_health
                       (vendor_id, provider_id, status, failure_count, success_count, failure_rate,
                        last_failure_at, last_success_at, circuit_until, last_failure_reason)
                     VALUES (?1, 'codex', 'degraded', 2, 8, 0.2, NULL, NULL, NULL, 'HTTP 503')",
                    params![id.as_str()],
                )
                .map_err(sqlite_error)?;
            Ok(())
        })
        .await
        .expect("seed");

        let vendor = repo.get(&saved.id).await.expect("get").expect("present");
        let balance = vendor.balance.expect("balance");
        assert_eq!(balance.remaining, Some(12.5));
        assert_eq!(balance.plan_name.as_deref(), Some("Pro"));
        assert_eq!(vendor.balance_status, Some(BalanceStatus::Success));
        assert_eq!(
            vendor.balance_queried_at.as_ref().map(Timestamp::as_str),
            Some("2026-09-16T12:00:00.000Z")
        );
        let health = vendor.gateway_health.expect("health");
        assert_eq!(health.status, GatewayVendorHealthStatus::Degraded);
        assert_eq!(health.failure_count, 2);
    }

    #[tokio::test]
    async fn concurrent_saves_of_different_names_all_succeed_with_distinct_sorts() {
        let (_dir, repo) = repo().await;
        let repo = std::sync::Arc::new(repo);

        let mut handles = Vec::new();
        for index in 0..8 {
            let repo = std::sync::Arc::clone(&repo);
            handles.push(tokio::spawn(async move {
                repo.save(&input(&format!("v{index}"), ProviderId::Codex))
                    .await
            }));
        }
        let mut sorts = Vec::new();
        for handle in handles {
            sorts.push(handle.await.expect("join").expect("save").sort);
        }
        sorts.sort_unstable();
        // 排序分配在写事务内完成，因此并发保存不会撞车或重复。
        assert_eq!(sorts, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }
}
