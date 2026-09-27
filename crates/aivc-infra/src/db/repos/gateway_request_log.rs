//! 网关请求日志仓储：写入、清理、用量报表与异常诊断。
//!
//! 本表是**用量与费用统计的唯一数据源**——现网刻意不引入第二套统计，这里保持一致。
//! 所有时间范围过滤都在 SQL 层完成，不允许把全表读到内存再过滤（日志量会持续增长）。
//!
//! # 记录时的安全边界
//!
//! 写入前必须截断与清洗：
//!
//! - `path` 截断到 [`REQUEST_PATH_MAX_LENGTH`]；
//! - `error_message` 折叠换行并截断到 [`REQUEST_ERROR_MAX_LENGTH`]；
//! - **绝不记录** API Key、请求正文、响应正文。
//!
//! # 异常诊断
//!
//! 工作台与诊断弹窗都只查 `outcome IN ('error', 'timeout')`，因此该过滤条件写死在
//! SQL 里而不是交给调用方，避免误查成功请求。
//!
//! # 相对现网的一处补齐
//!
//! 现网 `getGatewayUsageSummary` 不返回 `averageDurationMs`（虽然类型里声明为可选），
//! 而 `getGatewayUsageReport` 会返回。这里两边都计算：字段可选、当前无界面读取该路径，
//! 补齐后类型声明与实际返回一致，也不会造成可见变化。

use aivc_core::error::Result;
use aivc_core::gateway::{
    GatewayOutcome, GatewayRequestRecord, REQUEST_PATH_MAX_LENGTH, sanitize_error_message,
};
use aivc_core::gateway_usage::{
    FailureDiagnostic, FailureDiagnosticsPage, FailureOutcomeFilter, RecentFailure, UsageRow,
    UsageSummary, aggregate_usage, parse_stored_usage,
};
use aivc_core::id::VendorId;
use aivc_core::target::ProviderId;
use aivc_core::time::Timestamp;
use rusqlite::{Connection, params};

use crate::db::Db;
use crate::db::migrations::sqlite_error;

/// 工作台展示的近期异常条数（固定小窗口，避免侧栏查询变成全表扫描）。
pub const RECENT_FAILURE_LIMIT: u32 = 3;

/// 已删除供应商在报表中的占位名。
pub const DELETED_VENDOR_LABEL: &str = "已删除供应商";

/// 请求日志清理筛选条件。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LogFilter {
    /// 供应商编号。
    pub vendor_id: Option<VendorId>,
    /// 结果分类；`None` 表示不限。
    pub outcome: Option<GatewayOutcome>,
    /// 区间起点（含）。
    pub period_start: Option<Timestamp>,
    /// 区间终点（含）。
    pub period_end: Option<Timestamp>,
}

/// 清理候选项（供界面「先查询、再勾选、后删除」）。
#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    /// 请求编号。
    pub request_id: String,
    /// 记录时间。
    pub created_at: Timestamp,
    /// 供应商编号。
    pub vendor_id: String,
    /// 所属平台。
    pub provider_id: ProviderId,
    /// 结果。
    pub outcome: Option<GatewayOutcome>,
    /// 上游 HTTP 状态码。
    pub upstream_status: Option<u16>,
    /// 耗时（毫秒）。
    pub duration_ms: Option<u64>,
    /// 错误代码。
    pub error_code: Option<String>,
    /// 错误摘要。
    pub error_message: Option<String>,
    /// HTTP 方法。
    pub method: Option<String>,
    /// 请求路径。
    pub path: Option<String>,
}

/// 网关请求日志仓储。
#[derive(Debug, Clone)]
pub struct GatewayRequestLogRepo {
    db: Db,
}

impl GatewayRequestLogRepo {
    /// 由数据库句柄构造。
    #[must_use]
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// 写入一条请求记录。
    ///
    /// 返回 `false` 表示写入失败。**统计持久化失败不能影响上游响应**——现网同样
    /// 吞掉错误，只保留文本日志。
    pub async fn record(&self, entry: &GatewayRequestRecord) -> bool {
        let entry = entry.clone();
        self.db
            .write(move |connection| insert_request(connection, &entry))
            .await
            .is_ok()
    }

    /// 按筛选条件统计将被清理的条数。
    pub async fn count(&self, filter: &LogFilter) -> Result<u64> {
        let filter = filter.clone();
        self.db
            .read(move |connection| {
                let (where_clause, params) = build_filter(&filter);
                let count: i64 = connection
                    .query_row(
                        &format!("SELECT COUNT(*) FROM gateway_request_logs WHERE {where_clause}"),
                        rusqlite::params_from_iter(params.iter()),
                        |row| row.get(0),
                    )
                    .map_err(sqlite_error)?;
                Ok(u64::try_from(count).unwrap_or(0))
            })
            .await
    }

    /// 查询清理候选记录（按时间倒序）。
    pub async fn cleanup_entries(&self, filter: &LogFilter) -> Result<Vec<LogEntry>> {
        let filter = filter.clone();
        self.db
            .read(move |connection| {
                let (where_clause, params) = build_filter(&filter);
                let mut statement = connection
                    .prepare(&format!(
                        "SELECT request_id, provider_id, vendor_id, method, path, upstream_status,
                                outcome, duration_ms, error_code, error_message, created_at
                         FROM gateway_request_logs
                         WHERE {where_clause}
                         ORDER BY created_at DESC"
                    ))
                    .map_err(sqlite_error)?;
                let rows = statement
                    .query_map(rusqlite::params_from_iter(params.iter()), row_to_log_entry)
                    .map_err(sqlite_error)?;

                let mut entries = Vec::new();
                for row in rows {
                    entries.push(row.map_err(sqlite_error)?);
                }
                Ok(entries)
            })
            .await
    }

    /// 按编号删除记录，返回删除条数。
    pub async fn delete_entries(&self, request_ids: &[String]) -> Result<u64> {
        let ids: Vec<String> = request_ids
            .iter()
            .map(|id| id.trim().to_owned())
            .filter(|id| !id.is_empty())
            .collect();
        let ids: Vec<String> = {
            let mut unique = ids;
            unique.sort();
            unique.dedup();
            unique
        };
        if ids.is_empty() {
            return Ok(0);
        }
        self.db
            .write(move |connection| {
                let transaction = connection.transaction().map_err(sqlite_error)?;
                let mut removed = 0u64;
                {
                    let mut statement = transaction
                        .prepare("DELETE FROM gateway_request_logs WHERE request_id = ?1")
                        .map_err(sqlite_error)?;
                    for id in &ids {
                        removed +=
                            u64::try_from(statement.execute(params![id]).map_err(sqlite_error)?)
                                .unwrap_or(0);
                    }
                }
                transaction.commit().map_err(sqlite_error)?;
                Ok(removed)
            })
            .await
    }

    /// 按筛选条件删除记录，返回删除条数。
    pub async fn delete_by_filter(&self, filter: &LogFilter) -> Result<u64> {
        let filter = filter.clone();
        self.db
            .write(move |connection| {
                let (where_clause, params) = build_filter(&filter);
                let removed = connection
                    .execute(
                        &format!("DELETE FROM gateway_request_logs WHERE {where_clause}"),
                        rusqlite::params_from_iter(params.iter()),
                    )
                    .map_err(sqlite_error)?;
                Ok(u64::try_from(removed).unwrap_or(0))
            })
            .await
    }

    /// 统计区间内的用量总览。
    pub async fn usage_summary(
        &self,
        period_start: &Timestamp,
        period_end: &Timestamp,
    ) -> Result<UsageSummary> {
        let start = period_start.clone();
        let end = period_end.clone();
        self.db
            .read(move |connection| {
                let row = connection
                    .query_row(
                        "SELECT COUNT(*),
                                COALESCE(SUM(CASE WHEN outcome = 'ok' THEN 1 ELSE 0 END), 0),
                                COALESCE(SUM(CASE WHEN outcome <> 'ok' THEN 1 ELSE 0 END), 0),
                                COALESCE(SUM(switched), 0),
                                COALESCE(SUM(retry_count), 0),
                                COALESCE(SUM(duration_ms), 0)
                         FROM gateway_request_logs
                         WHERE created_at >= ?1 AND created_at <= ?2",
                        params![start.as_str(), end.as_str()],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, i64>(1)?,
                                row.get::<_, i64>(2)?,
                                row.get::<_, i64>(3)?,
                                row.get::<_, i64>(4)?,
                                row.get::<_, i64>(5)?,
                            ))
                        },
                    )
                    .map_err(sqlite_error)?;

                // 逐条解析 usage_json 而不是在 SQL 里做 JSON 聚合：字段族别名太多，
                // SQL 表达会迅速失控，而统计区间内的行数是有界的。
                let mut usage_rows: Vec<UsageRow> = Vec::new();
                let mut statement = connection
                    .prepare(
                        "SELECT vendor_id, provider_id, model, outcome, duration_ms, retry_count,
                                switched, usage_json
                         FROM gateway_request_logs
                         WHERE created_at >= ?1 AND created_at <= ?2",
                    )
                    .map_err(sqlite_error)?;
                let rows = statement
                    .query_map(params![start.as_str(), end.as_str()], |row| {
                        Ok(UsageRow {
                            vendor_id: VendorId::from(row.get::<_, String>(0)?),
                            vendor_name: String::new(),
                            provider_id: provider_from_sql(&row.get::<_, String>(1)?),
                            model: row.get(2)?,
                            outcome: GatewayOutcome::from_str_lossy(&row.get::<_, String>(3)?),
                            duration_ms: u64::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
                            retry_count: u32::try_from(row.get::<_, i64>(5)?).unwrap_or(0),
                            switched: row.get::<_, i64>(6)? != 0,
                            usage_json: row.get(7)?,
                        })
                    })
                    .map_err(sqlite_error)?;
                for item in rows {
                    usage_rows.push(item.map_err(sqlite_error)?);
                }

                let (request_count, success, failure, switched, retry, duration) = row;
                let mut usage = aivc_core::gateway::GatewayUsage::default();
                for item in &usage_rows {
                    if let Some(parsed) = parse_stored_usage(item.usage_json.as_deref()) {
                        // 跨请求必须求和；这里手工累加而不是复用逐项取最大值的 merge。
                        sum_usage(&mut usage, &parsed);
                    }
                }

                Ok(UsageSummary {
                    request_count: u64::try_from(request_count).unwrap_or(0),
                    success_count: u64::try_from(success).unwrap_or(0),
                    failure_count: u64::try_from(failure).unwrap_or(0),
                    switched_count: u64::try_from(switched).unwrap_or(0),
                    retry_count: Some(u64::try_from(retry).unwrap_or(0)),
                    average_duration_ms: Some(average(
                        u64::try_from(duration).unwrap_or(0),
                        u64::try_from(request_count).unwrap_or(0),
                    )),
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    cached_input_tokens: usage.cached_input_tokens,
                    reasoning_tokens: usage.reasoning_tokens,
                    total_tokens: usage.total_tokens,
                    cost_usd: usage.cost_usd,
                    period_start: start,
                    period_end: end,
                })
            })
            .await
    }

    /// 统计区间内的用量报表（总览 + 按供应商 + 按模型）。
    pub async fn usage_report(
        &self,
        period_start: &Timestamp,
        period_end: &Timestamp,
    ) -> Result<aivc_core::gateway_usage::UsageReport> {
        let start = period_start.clone();
        let end = period_end.clone();
        self.db
            .read(move |connection| {
                let mut statement = connection
                    .prepare(
                        "SELECT l.vendor_id, COALESCE(v.name, ?3) AS vendor_name,
                                l.provider_id, l.model, l.outcome, l.duration_ms,
                                l.retry_count, l.switched, l.usage_json
                         FROM gateway_request_logs l
                         LEFT JOIN api_vendors v ON v.id = l.vendor_id
                         WHERE l.created_at >= ?1 AND l.created_at <= ?2
                         ORDER BY l.created_at DESC",
                    )
                    .map_err(sqlite_error)?;
                let rows = statement
                    .query_map(
                        params![start.as_str(), end.as_str(), DELETED_VENDOR_LABEL],
                        |row| {
                            Ok(UsageRow {
                                vendor_id: VendorId::from(row.get::<_, String>(0)?),
                                vendor_name: row.get(1)?,
                                provider_id: provider_from_sql(&row.get::<_, String>(2)?),
                                model: row.get(3)?,
                                outcome: GatewayOutcome::from_str_lossy(&row.get::<_, String>(4)?),
                                duration_ms: u64::try_from(row.get::<_, i64>(5)?).unwrap_or(0),
                                retry_count: u32::try_from(row.get::<_, i64>(6)?).unwrap_or(0),
                                switched: row.get::<_, i64>(7)? != 0,
                                usage_json: row.get(8)?,
                            })
                        },
                    )
                    .map_err(sqlite_error)?;

                let mut usage_rows = Vec::new();
                for item in rows {
                    usage_rows.push(item.map_err(sqlite_error)?);
                }
                Ok(aggregate_usage(&usage_rows, &start, &end))
            })
            .await
    }

    /// 工作台用的近期异常（固定 3 条）。
    pub async fn recent_failures(&self) -> Result<Vec<RecentFailure>> {
        self.recent_failures_limited(RECENT_FAILURE_LIMIT).await
    }

    /// 近期异常（可指定条数）。
    pub async fn recent_failures_limited(&self, limit: u32) -> Result<Vec<RecentFailure>> {
        let limit = i64::from(limit);
        self.db
            .read(move |connection| {
                let mut statement = connection
                    .prepare(
                        "SELECT vendor_id, provider_id, upstream_status, outcome, created_at
                         FROM gateway_request_logs
                         WHERE outcome IN ('error', 'timeout')
                         ORDER BY created_at DESC
                         LIMIT ?1",
                    )
                    .map_err(sqlite_error)?;
                let rows = statement
                    .query_map(params![limit], |row| {
                        Ok(RecentFailure {
                            vendor_id: VendorId::from(row.get::<_, String>(0)?),
                            provider_id: provider_from_sql(&row.get::<_, String>(1)?),
                            upstream_status: row
                                .get::<_, Option<i64>>(2)?
                                .and_then(|value| u16::try_from(value).ok()),
                            outcome: FailureOutcomeFilter::from_str_lossy(
                                &row.get::<_, String>(3)?,
                            ),
                            created_at: Timestamp::from_raw(row.get::<_, String>(4)?),
                        })
                    })
                    .map_err(sqlite_error)?;

                let mut items = Vec::new();
                for row in rows {
                    items.push(row.map_err(sqlite_error)?);
                }
                Ok(items)
            })
            .await
    }

    /// 异常诊断分页。
    ///
    /// 筛选条件（平台 / 供应商 / 结果 / 时间区间）全部在 SQL 层生效，且固定
    /// `outcome IN ('error', 'timeout')`——不能把成功请求混进诊断列表。
    #[allow(clippy::too_many_arguments)]
    pub async fn failure_diagnostics_page(
        &self,
        page: u32,
        page_size: u32,
        vendor_id: Option<VendorId>,
        outcome: FailureOutcomeFilter,
        period_start: Option<Timestamp>,
        period_end: Option<Timestamp>,
        provider_id: Option<ProviderId>,
    ) -> Result<FailureDiagnosticsPage> {
        let safe_page = page.max(1);
        let safe_size = page_size.clamp(1, 50);
        let vendor = vendor_id.map(VendorId::into_string);
        let provider = provider_id.map(|id| id.as_str().to_owned());
        let outcome = outcome.as_str().to_owned();
        let start = period_start.map(|value| value.as_str().to_owned());
        let end = period_end.map(|value| value.as_str().to_owned());

        self.db
            .read(move |connection| {
                let mut filters = vec!["outcome IN ('error', 'timeout')".to_owned()];
                let mut values: Vec<String> = Vec::new();
                if let Some(provider) = &provider {
                    filters.push(format!("provider_id = ?{}", values.len() + 1));
                    values.push(provider.clone());
                }
                if let Some(vendor) = &vendor {
                    filters.push(format!("vendor_id = ?{}", values.len() + 1));
                    values.push(vendor.clone());
                }
                if !outcome.is_empty() {
                    filters.push(format!("outcome = ?{}", values.len() + 1));
                    values.push(outcome.clone());
                }
                if let Some(start) = &start {
                    filters.push(format!("created_at >= ?{}", values.len() + 1));
                    values.push(start.clone());
                }
                if let Some(end) = &end {
                    filters.push(format!("created_at <= ?{}", values.len() + 1));
                    values.push(end.clone());
                }
                let where_clause = filters.join(" AND ");

                let total: i64 = connection
                    .query_row(
                        &format!("SELECT COUNT(*) FROM gateway_request_logs WHERE {where_clause}"),
                        rusqlite::params_from_iter(values.iter()),
                        |row| row.get(0),
                    )
                    .map_err(sqlite_error)?;

                let mut page_values = values.clone();
                page_values.push(i64::from(safe_size).to_string());
                page_values.push(i64::from((safe_page - 1) * safe_size).to_string());
                let mut statement = connection
                    .prepare(&format!(
                        "SELECT vendor_id, provider_id, upstream_status, outcome, retry_count,
                                duration_ms, error_code, error_message, created_at
                         FROM gateway_request_logs
                         WHERE {where_clause}
                         ORDER BY created_at DESC
                         LIMIT ?{} OFFSET ?{}",
                        page_values.len() - 1,
                        page_values.len()
                    ))
                    .map_err(sqlite_error)?;
                let rows = statement
                    .query_map(rusqlite::params_from_iter(page_values.iter()), |row| {
                        Ok(FailureDiagnostic {
                            vendor_id: VendorId::from(row.get::<_, String>(0)?),
                            provider_id: provider_from_sql(&row.get::<_, String>(1)?),
                            upstream_status: row
                                .get::<_, Option<i64>>(2)?
                                .and_then(|value| u16::try_from(value).ok()),
                            outcome: FailureOutcomeFilter::from_str_lossy(
                                &row.get::<_, String>(3)?,
                            ),
                            retry_count: u32::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
                            duration_ms: u64::try_from(row.get::<_, i64>(5)?).unwrap_or(0),
                            error_code: row.get(6)?,
                            error_message: row.get(7)?,
                            created_at: Timestamp::from_raw(row.get::<_, String>(8)?),
                        })
                    })
                    .map_err(sqlite_error)?;

                let mut items = Vec::new();
                for row in rows {
                    items.push(row.map_err(sqlite_error)?);
                }
                Ok(FailureDiagnosticsPage {
                    items,
                    total: u64::try_from(total).unwrap_or(0),
                    page: safe_page,
                    page_size: safe_size,
                })
            })
            .await
    }
}

/// 写入一条请求记录（截断与清洗在此完成）。
fn insert_request(connection: &Connection, entry: &GatewayRequestRecord) -> Result<()> {
    connection
        .execute(
            "INSERT OR REPLACE INTO gateway_request_logs
               (request_id, route_id, provider_id, vendor_id, method, path, model,
                upstream_status, outcome, duration_ms, bytes_in, bytes_out, retry_count,
                switched, error_code, error_message, usage_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            params![
                entry.request_id.as_str(),
                entry.route_id,
                entry.provider_id.as_str(),
                entry.vendor_id.as_str(),
                entry.method,
                clamp(&entry.path, REQUEST_PATH_MAX_LENGTH),
                entry.model,
                entry.upstream_status,
                entry.outcome.as_str(),
                to_i64(entry.duration_ms),
                to_i64(entry.bytes_in),
                to_i64(entry.bytes_out),
                i64::from(entry.retry_count),
                i64::from(entry.switched),
                entry.error_code,
                // 清洗换行并截断；清洗后为空则视为没有错误摘要。
                entry
                    .error_message
                    .as_deref()
                    .map(sanitize_error_message)
                    .filter(|value| !value.is_empty()),
                entry
                    .usage
                    .as_ref()
                    .and_then(|usage| serde_json::to_string(usage).ok()),
                entry.created_at.as_str(),
            ],
        )
        .map_err(sqlite_error)?;
    Ok(())
}

/// 构造 WHERE 子句与参数。
fn build_filter(filter: &LogFilter) -> (String, Vec<String>) {
    let mut filters = Vec::new();
    let mut values = Vec::new();
    if let Some(vendor_id) = &filter.vendor_id {
        filters.push(format!("vendor_id = ?{}", values.len() + 1));
        values.push(vendor_id.as_str().to_owned());
    }
    if let Some(outcome) = filter.outcome {
        filters.push(format!("outcome = ?{}", values.len() + 1));
        values.push(outcome.as_str().to_owned());
    }
    if let Some(start) = &filter.period_start {
        filters.push(format!("created_at >= ?{}", values.len() + 1));
        values.push(start.as_str().to_owned());
    }
    if let Some(end) = &filter.period_end {
        filters.push(format!("created_at <= ?{}", values.len() + 1));
        values.push(end.as_str().to_owned());
    }
    let clause = if filters.is_empty() {
        "1 = 1".to_owned()
    } else {
        filters.join(" AND ")
    };
    (clause, values)
}

/// 把一行映射为清理候选项。
fn row_to_log_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<LogEntry> {
    let outcome: String = row.get(6)?;
    Ok(LogEntry {
        request_id: row.get(0)?,
        provider_id: provider_from_sql(&row.get::<_, String>(1)?),
        vendor_id: row.get(2)?,
        method: row.get(3)?,
        path: row.get(4)?,
        upstream_status: row
            .get::<_, Option<i64>>(5)?
            .and_then(|value| u16::try_from(value).ok()),
        outcome: match outcome.as_str() {
            "ok" => Some(GatewayOutcome::Ok),
            "client-aborted" => Some(GatewayOutcome::ClientAborted),
            "timeout" => Some(GatewayOutcome::Timeout),
            "error" => Some(GatewayOutcome::Error),
            _ => None,
        },
        duration_ms: row
            .get::<_, Option<i64>>(7)?
            .and_then(|value| u64::try_from(value).ok()),
        error_code: row.get(8)?,
        error_message: row.get(9)?,
        created_at: Timestamp::from_raw(row.get::<_, String>(10)?),
    })
}

/// 跨请求累加用量（与请求内的逐项取最大值不同）。
fn sum_usage(
    target: &mut aivc_core::gateway::GatewayUsage,
    next: &aivc_core::gateway::GatewayUsage,
) {
    macro_rules! add {
        ($($field:ident),* $(,)?) => {
            $( if let Some(value) = next.$field {
                target.$field = Some(target.$field.unwrap_or(0) + value);
            } )*
        };
    }
    add!(
        input_tokens,
        output_tokens,
        cached_input_tokens,
        reasoning_tokens,
        total_tokens
    );
    if let Some(cost) = next.cost_usd {
        target.cost_usd = Some(target.cost_usd.unwrap_or(0.0) + cost);
    }
}

/// 平均耗时；无请求时为 0。
fn average(total_ms: u64, count: u64) -> f64 {
    if count == 0 {
        0.0
    } else {
        total_ms as f64 / count as f64
    }
}

/// 按字符数截断。
fn clamp(value: &str, max_length: usize) -> String {
    value.chars().take(max_length).collect()
}

/// `u64` → `i64`，溢出饱和。
fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
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
    use aivc_core::gateway::{GatewayUsage, REQUEST_ERROR_MAX_LENGTH};
    use aivc_core::id::RequestId;

    async fn repo() -> (tempfile::TempDir, GatewayRequestLogRepo) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open(dir.path().join("aivc.db")).await.expect("open");
        (dir, GatewayRequestLogRepo::new(db))
    }

    fn record(id: &str, outcome: GatewayOutcome, at: &str) -> GatewayRequestRecord {
        GatewayRequestRecord {
            request_id: RequestId::from(id),
            route_id: "route-1".to_owned(),
            provider_id: ProviderId::Codex,
            vendor_id: VendorId::from("v1"),
            method: "POST".to_owned(),
            path: "/v1/responses".to_owned(),
            model: Some("gpt-5".to_owned()),
            upstream_status: Some(200),
            outcome,
            duration_ms: 100,
            bytes_in: 10,
            bytes_out: 20,
            retry_count: 0,
            switched: false,
            error_code: None,
            error_message: None,
            usage: None,
            created_at: Timestamp::from_raw(at),
        }
    }

    #[tokio::test]
    async fn empty_database_reports_zeroes() {
        let (_dir, repo) = repo().await;
        let start = Timestamp::from_raw("2026-01-01T00:00:00.000Z");
        let end = Timestamp::from_raw("2026-01-02T00:00:00.000Z");
        let summary = repo.usage_summary(&start, &end).await.expect("summary");
        assert_eq!(summary.request_count, 0);
        assert_eq!(summary.average_duration_ms, Some(0.0));
        assert!(repo.recent_failures().await.expect("recent").is_empty());
    }

    #[tokio::test]
    async fn record_then_summarize_counts_every_outcome_class() {
        let (_dir, repo) = repo().await;
        let at = "2026-06-01T00:00:00.000Z";
        for (id, outcome) in [
            ("r1", GatewayOutcome::Ok),
            ("r2", GatewayOutcome::Error),
            ("r3", GatewayOutcome::Timeout),
            ("r4", GatewayOutcome::ClientAborted),
        ] {
            assert!(repo.record(&record(id, outcome, at)).await);
        }

        let start = Timestamp::from_raw("2026-01-01T00:00:00.000Z");
        let end = Timestamp::from_raw("2026-12-31T00:00:00.000Z");
        let summary = repo.usage_summary(&start, &end).await.expect("summary");
        assert_eq!(summary.request_count, 4);
        assert_eq!(summary.success_count, 1);
        // 失败包含 error / timeout / client-aborted。
        assert_eq!(summary.failure_count, 3);
    }

    #[tokio::test]
    async fn usage_tokens_are_summed_across_requests() {
        let (_dir, repo) = repo().await;
        let at = "2026-06-01T00:00:00.000Z";
        for (id, input, output) in [("r1", 10u64, 5u64), ("r2", 20, 8)] {
            let mut entry = record(id, GatewayOutcome::Ok, at);
            entry.usage = Some(GatewayUsage {
                input_tokens: Some(input),
                output_tokens: Some(output),
                total_tokens: Some(input + output),
                ..GatewayUsage::default()
            });
            assert!(repo.record(&entry).await);
        }

        let start = Timestamp::from_raw("2026-01-01T00:00:00.000Z");
        let end = Timestamp::from_raw("2026-12-31T00:00:00.000Z");
        let summary = repo.usage_summary(&start, &end).await.expect("summary");
        assert_eq!(summary.input_tokens, Some(30));
        assert_eq!(summary.output_tokens, Some(13));
        assert_eq!(summary.total_tokens, Some(43));
    }

    #[tokio::test]
    async fn usage_report_groups_by_vendor_and_model() {
        let (_dir, repo) = repo().await;
        let at = "2026-06-01T00:00:00.000Z";
        let mut first = record("r1", GatewayOutcome::Ok, at);
        first.usage = Some(GatewayUsage {
            input_tokens: Some(10),
            ..GatewayUsage::default()
        });
        let mut second = record("r2", GatewayOutcome::Ok, at);
        second.vendor_id = VendorId::from("v2");
        second.model = Some("claude-sonnet".to_owned());
        second.usage = Some(GatewayUsage {
            input_tokens: Some(5),
            ..GatewayUsage::default()
        });
        assert!(repo.record(&first).await);
        assert!(repo.record(&second).await);

        let start = Timestamp::from_raw("2026-01-01T00:00:00.000Z");
        let end = Timestamp::from_raw("2026-12-31T00:00:00.000Z");
        let report = repo.usage_report(&start, &end).await.expect("report");

        assert_eq!(report.summary.request_count, 2);
        assert_eq!(report.summary.input_tokens, Some(15));
        assert_eq!(report.vendors.len(), 2);
        assert_eq!(report.models.len(), 2);
        // 供应商已被删除时使用占位名。
        assert!(
            report
                .vendors
                .iter()
                .all(|item| item.label == DELETED_VENDOR_LABEL),
            "无对应供应商时应使用占位名"
        );
    }

    #[tokio::test]
    async fn usage_report_uses_the_live_vendor_name_when_present() {
        let (dir, repo) = repo().await;
        let db = Db::open(dir.path().join("aivc.db")).await.expect("open");
        db.write(|connection| {
            connection
                .execute(
                    "INSERT INTO api_vendors
                       (id, provider_id, name, name_norm, api_key, api_base_url, sort, enabled, created_at, updated_at)
                     VALUES ('v1', 'codex', '主供应商', '主供应商', 'k', 'https://x', 1, 1, 't', 't')",
                    [],
                )
                .map_err(sqlite_error)?;
            Ok(())
        })
        .await
        .expect("seed vendor");

        assert!(
            repo.record(&record(
                "r1",
                GatewayOutcome::Ok,
                "2026-06-01T00:00:00.000Z"
            ))
            .await
        );

        let start = Timestamp::from_raw("2026-01-01T00:00:00.000Z");
        let end = Timestamp::from_raw("2026-12-31T00:00:00.000Z");
        let report = repo.usage_report(&start, &end).await.expect("report");
        assert_eq!(report.vendors[0].label, "主供应商");
    }

    #[tokio::test]
    async fn recent_failures_only_include_error_and_timeout() {
        let (_dir, repo) = repo().await;
        repo.record(&record(
            "ok",
            GatewayOutcome::Ok,
            "2026-06-01T00:00:00.000Z",
        ))
        .await;
        repo.record(&record(
            "err",
            GatewayOutcome::Error,
            "2026-06-02T00:00:00.000Z",
        ))
        .await;
        repo.record(&record(
            "to",
            GatewayOutcome::Timeout,
            "2026-06-03T00:00:00.000Z",
        ))
        .await;
        // 客户端断开不算「网关异常」，不应出现在诊断里。
        repo.record(&record(
            "abort",
            GatewayOutcome::ClientAborted,
            "2026-06-04T00:00:00.000Z",
        ))
        .await;

        let recent = repo.recent_failures().await.expect("recent");
        assert_eq!(recent.len(), 2);
        // 按时间倒序：timeout 在前。
        assert_eq!(recent[0].outcome, FailureOutcomeFilter::Timeout);
        assert_eq!(recent[1].outcome, FailureOutcomeFilter::Error);
    }

    #[tokio::test]
    async fn recent_failures_respect_the_limit() {
        let (_dir, repo) = repo().await;
        for index in 0..10 {
            repo.record(&record(
                &format!("e{index}"),
                GatewayOutcome::Error,
                &format!("2026-06-{:02}T00:00:00.000Z", index + 1),
            ))
            .await;
        }
        assert_eq!(repo.recent_failures().await.expect("recent").len(), 3);
        assert_eq!(
            repo.recent_failures_limited(7).await.expect("recent").len(),
            7
        );
    }

    #[tokio::test]
    async fn diagnostics_page_filters_and_paginates_in_sql() {
        let (_dir, repo) = repo().await;
        for index in 0..25 {
            let mut entry = record(
                &format!("e{index}"),
                GatewayOutcome::Error,
                &format!("2026-06-{:02}T00:00:00.000Z", index % 28 + 1),
            );
            if index % 2 == 0 {
                entry.vendor_id = VendorId::from("v2");
            }
            if index % 5 == 0 {
                entry.provider_id = ProviderId::Claude;
            }
            repo.record(&entry).await;
        }
        repo.record(&record(
            "ok",
            GatewayOutcome::Ok,
            "2026-06-01T00:00:00.000Z",
        ))
        .await;

        let first = repo
            .failure_diagnostics_page(1, 10, None, FailureOutcomeFilter::Any, None, None, None)
            .await
            .expect("page 1");
        assert_eq!(first.total, 25, "成功请求不得计入诊断总数");
        assert_eq!(first.items.len(), 10);
        assert_eq!(first.page, 1);
        assert_eq!(first.page_size, 10);

        let last = repo
            .failure_diagnostics_page(3, 10, None, FailureOutcomeFilter::Any, None, None, None)
            .await
            .expect("page 3");
        assert_eq!(last.items.len(), 5);

        // 按供应商筛选在 SQL 层生效。
        let by_vendor = repo
            .failure_diagnostics_page(
                1,
                50,
                Some(VendorId::from("v2")),
                FailureOutcomeFilter::Any,
                None,
                None,
                None,
            )
            .await
            .expect("by vendor");
        assert_eq!(by_vendor.total, 13);

        // 按平台筛选。
        let by_provider = repo
            .failure_diagnostics_page(
                1,
                50,
                None,
                FailureOutcomeFilter::Any,
                None,
                None,
                Some(ProviderId::Claude),
            )
            .await
            .expect("by provider");
        assert_eq!(by_provider.total, 5);
    }

    #[tokio::test]
    async fn diagnostics_page_clamps_pagination_inputs() {
        let (_dir, repo) = repo().await;
        // 页码 0 归一化为 1，每页条数被限制在 1..=50。
        let page = repo
            .failure_diagnostics_page(0, 999, None, FailureOutcomeFilter::Any, None, None, None)
            .await
            .expect("page");
        assert_eq!(page.page, 1);
        assert_eq!(page.page_size, 50);

        let page = repo
            .failure_diagnostics_page(1, 0, None, FailureOutcomeFilter::Any, None, None, None)
            .await
            .expect("page");
        assert_eq!(page.page_size, 1);
    }

    #[tokio::test]
    async fn diagnostics_time_range_filter_is_applied() {
        let (_dir, repo) = repo().await;
        repo.record(&record(
            "old",
            GatewayOutcome::Error,
            "2026-01-01T00:00:00.000Z",
        ))
        .await;
        repo.record(&record(
            "new",
            GatewayOutcome::Error,
            "2026-06-01T00:00:00.000Z",
        ))
        .await;

        let page = repo
            .failure_diagnostics_page(
                1,
                10,
                None,
                FailureOutcomeFilter::Any,
                Some(Timestamp::from_raw("2026-05-01T00:00:00.000Z")),
                Some(Timestamp::from_raw("2026-07-01T00:00:00.000Z")),
                None,
            )
            .await
            .expect("page");
        assert_eq!(page.total, 1);
    }

    #[tokio::test]
    async fn cleanup_entries_respect_filters_and_can_be_deleted_by_id() {
        let (_dir, repo) = repo().await;
        for index in 0..6 {
            let mut entry = record(
                &format!("r{index}"),
                GatewayOutcome::Ok,
                &format!("2026-06-{:02}T00:00:00.000Z", index + 1),
            );
            if index >= 4 {
                entry.outcome = GatewayOutcome::Error;
            }
            repo.record(&entry).await;
        }

        let all = repo
            .cleanup_entries(&LogFilter::default())
            .await
            .expect("all");
        assert_eq!(all.len(), 6);
        assert_eq!(repo.count(&LogFilter::default()).await.expect("count"), 6);

        let errors = repo
            .cleanup_entries(&LogFilter {
                outcome: Some(GatewayOutcome::Error),
                ..LogFilter::default()
            })
            .await
            .expect("errors");
        assert_eq!(errors.len(), 2);

        // 只删除勾选的编号。
        let removed = repo
            .delete_entries(&[errors[0].request_id.clone()])
            .await
            .expect("delete");
        assert_eq!(removed, 1);
        assert_eq!(repo.count(&LogFilter::default()).await.expect("count"), 5);
    }

    #[tokio::test]
    async fn delete_entries_deduplicates_and_ignores_blank_ids() {
        let (_dir, repo) = repo().await;
        repo.record(&record(
            "r1",
            GatewayOutcome::Ok,
            "2026-06-01T00:00:00.000Z",
        ))
        .await;

        let removed = repo
            .delete_entries(&[
                "r1".to_owned(),
                "r1".to_owned(),
                "  ".to_owned(),
                String::new(),
            ])
            .await
            .expect("delete");
        assert_eq!(removed, 1);
        assert_eq!(repo.count(&LogFilter::default()).await.expect("count"), 0);
        // 空输入是 no-op。
        assert_eq!(repo.delete_entries(&[]).await.expect("empty"), 0);
    }

    #[tokio::test]
    async fn delete_by_filter_removes_only_matching_rows() {
        let (_dir, repo) = repo().await;
        repo.record(&record(
            "old",
            GatewayOutcome::Ok,
            "2026-01-01T00:00:00.000Z",
        ))
        .await;
        repo.record(&record(
            "new",
            GatewayOutcome::Ok,
            "2026-06-01T00:00:00.000Z",
        ))
        .await;

        let removed = repo
            .delete_by_filter(&LogFilter {
                period_start: Some(Timestamp::from_raw("2026-05-01T00:00:00.000Z")),
                ..LogFilter::default()
            })
            .await
            .expect("delete");
        assert_eq!(removed, 1);
        assert_eq!(repo.count(&LogFilter::default()).await.expect("count"), 1);
    }

    #[tokio::test]
    async fn long_paths_and_error_messages_are_truncated_and_cleaned() {
        let (_dir, repo) = repo().await;
        let mut entry = record("r1", GatewayOutcome::Error, "2026-06-01T00:00:00.000Z");
        entry.path = format!("/v1/{}", "p".repeat(1_000));
        entry.error_message = Some(format!("line1\nline2\t{}", "e".repeat(1_000)));
        assert!(repo.record(&entry).await);

        let listed = repo
            .cleanup_entries(&LogFilter::default())
            .await
            .expect("list");
        let item = &listed[0];
        assert_eq!(
            item.path.as_ref().map(|p| p.chars().count()),
            Some(REQUEST_PATH_MAX_LENGTH)
        );
        let message = item.error_message.as_deref().expect("message");
        assert!(!message.contains('\n'), "错误摘要不得包含换行");
        assert!(!message.contains('\t'));
        assert_eq!(message.chars().count(), REQUEST_ERROR_MAX_LENGTH - 1 + 3);
    }

    #[tokio::test]
    async fn record_is_idempotent_for_the_same_request_id() {
        let (_dir, repo) = repo().await;
        let mut entry = record("r1", GatewayOutcome::Ok, "2026-06-01T00:00:00.000Z");
        assert!(repo.record(&entry).await);
        entry.outcome = GatewayOutcome::Error;
        assert!(repo.record(&entry).await);

        assert_eq!(repo.count(&LogFilter::default()).await.expect("count"), 1);
        let listed = repo
            .cleanup_entries(&LogFilter::default())
            .await
            .expect("list");
        assert_eq!(
            listed[0].outcome,
            Some(GatewayOutcome::Error),
            "同编号应覆盖"
        );
    }

    #[tokio::test]
    async fn corrupt_usage_json_does_not_break_the_report() {
        let (dir, repo) = repo().await;
        repo.record(&record(
            "r1",
            GatewayOutcome::Ok,
            "2026-06-01T00:00:00.000Z",
        ))
        .await;

        let db = Db::open(dir.path().join("aivc.db")).await.expect("open");
        db.write(|connection| {
            connection
                .execute(
                    "UPDATE gateway_request_logs SET usage_json = 'not json' WHERE request_id = 'r1'",
                    [],
                )
                .map_err(sqlite_error)?;
            Ok(())
        })
        .await
        .expect("corrupt");

        let start = Timestamp::from_raw("2026-01-01T00:00:00.000Z");
        let end = Timestamp::from_raw("2026-12-31T00:00:00.000Z");
        let summary = repo.usage_summary(&start, &end).await.expect("summary");
        // 请求仍被计数，但不得产生虚假 token。
        assert_eq!(summary.request_count, 1);
        assert_eq!(summary.input_tokens, None);
    }
}
