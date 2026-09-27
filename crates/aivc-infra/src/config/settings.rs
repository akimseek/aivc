//! `settings.json` 读写与校验。
//!
//! # 为什么用「原始值 + 归一化访问器」而不是直接反序列化到强类型
//!
//! 现网的读取路径是 `JSON.parse` 之后**逐字段**判断类型：
//!
//! ```js
//! function normalizeGatewayPort(port) {
//!   if (typeof port !== "number" || !Number.isInteger(port) || port < 0 || port > 65535) return 0;
//!   return port;
//! }
//! ```
//!
//! 也就是说，一个被手工编辑坏的 `settings.json`（例如把端口写成字符串 `"8080"`）
//! 只会让**该字段**回退默认值，其余字段照常生效。若直接反序列化到强类型结构，
//! 一个字段类型不符就会让整个文件解析失败，行为不一致。
//!
//! 因此这里保留 [`RawSettings`]（字段为 `serde_json::Value`）作为磁盘形态，
//! 由 [`AppSettings`] 提供已归一化的强类型快照。这样解析过程是**全函数**的：
//! 任何输入都不会失败，只会回退到默认值。
//!
//! # 其他必须保持的行为
//!
//! - 原子写：先写临时文件再 `rename`，避免中途崩溃留下半个文件。
//! - 损坏文件降级为空配置，并备份为 `<path>.broken-<时间戳>`。
//! - `gatewayEnabled` 的判定是 `!== false`（缺失视为开启），而
//!   `gatewayExternalApiEnabled` 是 `=== true`（缺失视为关闭）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use aivc_core::error::{ConfigError, Result};
use aivc_core::target::Target;
use aivc_core::time::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::RwLock;

/// 网关端口上限。
const GATEWAY_PORT_MAX: i64 = 65_535;
/// 同一供应商连续失败多少次后切换到下一个候选。
const FAILURE_THRESHOLD_DEFAULT: i64 = 1;
const FAILURE_THRESHOLD_MIN: i64 = 1;
const FAILURE_THRESHOLD_MAX: i64 = 10;
/// 熔断触发阈值。
const CIRCUIT_THRESHOLD_DEFAULT: i64 = 3;
const CIRCUIT_THRESHOLD_MIN: i64 = 1;
const CIRCUIT_THRESHOLD_MAX: i64 = 20;
/// 熔断持续时长（秒）。
const CIRCUIT_DURATION_DEFAULT: i64 = 60;
const CIRCUIT_DURATION_MIN: i64 = 10;
const CIRCUIT_DURATION_MAX: i64 = 86_400;

/// 已归一化的设置快照：全部字段都经过校验并带默认值。
#[derive(Debug, Clone, PartialEq)]
pub struct AppSettings {
    /// 网关监听端口；`0` 表示自动分配。
    pub gateway_port: u16,
    /// 是否开启常驻网关。缺失视为开启（`!== false`）。
    pub gateway_enabled: bool,
    /// 是否开启对外 API。缺失视为关闭（`=== true`）。
    pub gateway_external_api_enabled: bool,
    /// 对外 API 令牌的 sha256 十六进制摘要；令牌本体从不落库。
    pub gateway_external_api_token_hash: String,
    /// 同一供应商连续失败多少次后切换候选。
    pub gateway_failure_threshold: u8,
    /// 熔断触发阈值。
    pub gateway_circuit_failure_threshold: u8,
    /// 熔断持续时长（秒）。
    pub gateway_circuit_duration_seconds: u32,
    /// WSL 发行版 → Codex 配置目录的人工覆盖。
    pub wsl_codex_homes: BTreeMap<String, String>,
    /// 已缓存的目标列表（按平台合并覆盖）。
    pub cached_targets: Vec<Target>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            gateway_port: 0,
            gateway_enabled: true,
            gateway_external_api_enabled: false,
            gateway_external_api_token_hash: String::new(),
            gateway_failure_threshold: FAILURE_THRESHOLD_DEFAULT as u8,
            gateway_circuit_failure_threshold: CIRCUIT_THRESHOLD_DEFAULT as u8,
            gateway_circuit_duration_seconds: CIRCUIT_DURATION_DEFAULT as u32,
            wsl_codex_homes: BTreeMap::new(),
            cached_targets: Vec::new(),
        }
    }
}

/// 磁盘形态：字段保持原始 JSON 值，逐字段容错。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RawSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    wsl_codex_homes: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gateway_port: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gateway_enabled: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gateway_external_api_enabled: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gateway_external_api_token_hash: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gateway_failure_threshold: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gateway_circuit_failure_threshold: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gateway_circuit_duration_seconds: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cached_targets: Option<Value>,
    /// 旧字段：工作区预设已迁移到 SQLite，读取后忽略，避免把旧数据当成权威来源。
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_presets: Option<Value>,
}

impl RawSettings {
    /// 归一化为强类型快照。
    #[must_use]
    pub fn normalize(&self) -> AppSettings {
        AppSettings {
            gateway_port: u16::try_from(normalize_integer(
                self.gateway_port.as_ref(),
                0,
                0,
                GATEWAY_PORT_MAX,
            ))
            .unwrap_or(0),
            // 现网判定是 `!== false`：缺失或非布尔值都视为开启。
            gateway_enabled: self.gateway_enabled.as_ref().and_then(Value::as_bool) != Some(false),
            // 现网判定是 `=== true`：缺失或非布尔值都视为关闭。
            gateway_external_api_enabled: self
                .gateway_external_api_enabled
                .as_ref()
                .and_then(Value::as_bool)
                == Some(true),
            gateway_external_api_token_hash: self
                .gateway_external_api_token_hash
                .as_ref()
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            gateway_failure_threshold: u8::try_from(normalize_integer(
                self.gateway_failure_threshold.as_ref(),
                FAILURE_THRESHOLD_DEFAULT,
                FAILURE_THRESHOLD_MIN,
                FAILURE_THRESHOLD_MAX,
            ))
            .unwrap_or(FAILURE_THRESHOLD_DEFAULT as u8),
            gateway_circuit_failure_threshold: u8::try_from(normalize_integer(
                self.gateway_circuit_failure_threshold.as_ref(),
                CIRCUIT_THRESHOLD_DEFAULT,
                CIRCUIT_THRESHOLD_MIN,
                CIRCUIT_THRESHOLD_MAX,
            ))
            .unwrap_or(CIRCUIT_THRESHOLD_DEFAULT as u8),
            gateway_circuit_duration_seconds: u32::try_from(normalize_integer(
                self.gateway_circuit_duration_seconds.as_ref(),
                CIRCUIT_DURATION_DEFAULT,
                CIRCUIT_DURATION_MIN,
                CIRCUIT_DURATION_MAX,
            ))
            .unwrap_or(CIRCUIT_DURATION_DEFAULT as u32),
            wsl_codex_homes: self.wsl_codex_homes.clone().unwrap_or_default(),
            cached_targets: self
                .cached_targets
                .as_ref()
                .and_then(|value| serde_json::from_value::<Vec<Target>>(value.clone()).ok())
                .unwrap_or_default(),
        }
    }

    /// 写入网关端口（先归一化，与现网 `setGatewayPort` 一致）。
    pub fn set_gateway_port(&mut self, port: u16) {
        self.gateway_port = Some(Value::from(port));
    }

    /// 写入常驻网关开关。
    pub fn set_gateway_enabled(&mut self, enabled: bool) {
        self.gateway_enabled = Some(Value::from(enabled));
    }

    /// 写入对外 API 开关。
    pub fn set_gateway_external_api_enabled(&mut self, enabled: bool) {
        self.gateway_external_api_enabled = Some(Value::from(enabled));
    }

    /// 写入对外 API 令牌摘要。
    pub fn set_gateway_external_api_token_hash(&mut self, hash: impl Into<String>) {
        self.gateway_external_api_token_hash = Some(Value::from(hash.into()));
    }

    /// 写入失败阈值（归一化后落库）。
    pub fn set_gateway_failure_threshold(&mut self, threshold: u8) {
        self.gateway_failure_threshold = Some(Value::from(threshold));
    }

    /// 写入熔断阈值（归一化后落库）。
    pub fn set_gateway_circuit_failure_threshold(&mut self, threshold: u8) {
        self.gateway_circuit_failure_threshold = Some(Value::from(threshold));
    }

    /// 写入熔断时长（归一化后落库）。
    pub fn set_gateway_circuit_duration_seconds(&mut self, seconds: u32) {
        self.gateway_circuit_duration_seconds = Some(Value::from(seconds));
    }

    /// 设置某发行版的 Codex 目录覆盖（值会 `trim`，与现网一致）。
    pub fn set_wsl_codex_home(&mut self, distro: &str, codex_home: &str) {
        self.wsl_codex_homes
            .get_or_insert_with(BTreeMap::new)
            .insert(distro.to_owned(), codex_home.trim().to_owned());
    }

    /// 清除某发行版的 Codex 目录覆盖。
    pub fn clear_wsl_codex_home(&mut self, distro: &str) {
        if let Some(homes) = self.wsl_codex_homes.as_mut() {
            homes.remove(distro);
        }
    }

    /// 按平台合并覆盖目标缓存。
    ///
    /// 现网语义：`next` 为空时**保持原值不变**；否则移除 `next` 覆盖到的全部平台，
    /// 再追加 `next`。这样单平台刷新不会清掉其他平台的目标。
    pub fn merge_cached_targets(&mut self, next: &[Target]) {
        if next.is_empty() {
            return;
        }
        let incoming_providers: std::collections::BTreeSet<_> =
            next.iter().map(|target| target.provider).collect();
        let current = self
            .cached_targets
            .as_ref()
            .and_then(|value| serde_json::from_value::<Vec<Target>>(value.clone()).ok())
            .unwrap_or_default();
        let kept: Vec<Target> = current
            .into_iter()
            .filter(|target| !incoming_providers.contains(&target.provider))
            .collect();
        let mut merged = kept;
        merged.extend_from_slice(next);
        self.cached_targets = serde_json::to_value(merged).ok();
    }
}

/// 判断 JSON 值是否为「整数」（对齐 JS `Number.isInteger`）。
///
/// JS 里 `8080` 与 `8080.0` 是同一个值，因此小数部分为 0 的浮点也必须接受。
fn normalize_integer(value: Option<&Value>, default: i64, min: i64, max: i64) -> i64 {
    let Some(raw) = value else {
        return default;
    };
    let Some(number) = integral_f64(raw) else {
        return default;
    };
    let (min_f, max_f) = (min as f64, max as f64);
    if number < min_f || number > max_f {
        return default;
    }
    number as i64
}

/// 把 JSON 值读成整数型 `f64`；非数字或带小数部分返回 [`None`]。
fn integral_f64(value: &Value) -> Option<f64> {
    let number = match value {
        Value::Number(number) => number.as_f64()?,
        _ => return None,
    };
    if number.is_finite() && number.fract() == 0.0 {
        Some(number)
    } else {
        None
    }
}

/// 去掉文件开头的 UTF-8 BOM。
///
/// # 为什么要专门处理这一个字节
///
/// `serde_json` **不接受**开头的 BOM：`{` 前面多三个字节 `EF BB BF`，它会直接报
/// 「expected value at line 1 column 1」。而不少编辑器（部分 Windows 版本的记事本、
/// 各种「UTF-8 with BOM」选项）保存 JSON 时**默认加 BOM**。
///
/// 后果不是「文件读不了」这么轻：解析失败会走「损坏文件」分支——**把原文件改名备份、
/// 并把设置换成默认值**。用户手改一下设置文件再回来，目标缓存、网关端口、窗口偏好
/// 全没了，只有一行 ERROR 日志能解释。
///
/// 实测踩到过：手工写设置文件启用网关，启动后日志报解析失败、文件被改名成
/// `*.broken-*`，应用的设置全部回到默认值。
#[must_use]
fn strip_bom(content: &str) -> &str {
    content.strip_prefix('\u{feff}').unwrap_or(content)
}

/// 设置存储：内存缓存 + 串行化原子写。
///
/// 读走读锁（网关热路径会频繁读取端口与阈值），写走写锁并在锁内完成
/// 「读当前值 → 应用变更 → 原子落盘 → 更新缓存」，因此并发写入不会互相覆盖。
///
/// 内部锁用 `Arc` 共享，因此 [`SettingsStore`] 可克隆：多个模块各自持有一份句柄，
/// 但共享同一份缓存与同一条写序列。
#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: PathBuf,
    state: Arc<RwLock<Option<RawSettings>>>,
}

impl SettingsStore {
    /// 由设置文件路径构造（不触碰磁盘）。
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            state: Arc::new(RwLock::new(None)),
        }
    }

    /// 设置文件路径。
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 读取已归一化的设置快照。
    pub async fn snapshot(&self) -> AppSettings {
        self.load_raw().await.normalize()
    }

    /// 读取原始设置（保持字段的原始 JSON 值）。
    pub async fn load_raw(&self) -> RawSettings {
        // 快路径：已缓存时只取读锁。
        if let Some(cached) = self.state.read().await.as_ref() {
            return cached.clone();
        }
        // 慢路径：取写锁后二次检查，避免并发首次加载重复读盘与重复备份。
        let mut guard = self.state.write().await;
        if let Some(cached) = guard.as_ref() {
            return cached.clone();
        }
        let loaded = self.read_from_disk().await;
        *guard = Some(loaded.clone());
        loaded
    }

    /// 在写锁内执行「读 → 改 → 原子写 → 更新缓存」。
    ///
    /// 返回变更后的归一化快照。
    pub async fn update<F>(&self, mutate: F) -> Result<AppSettings>
    where
        F: FnOnce(&mut RawSettings),
    {
        let mut guard = self.state.write().await;
        let mut current = match guard.as_ref() {
            Some(cached) => cached.clone(),
            None => self.read_from_disk().await,
        };
        mutate(&mut current);
        self.write_to_disk(&current).await?;
        let snapshot = current.normalize();
        *guard = Some(current);
        Ok(snapshot)
    }

    /// 丢弃内存缓存，下次读取重新读盘。
    pub async fn invalidate(&self) {
        *self.state.write().await = None;
    }

    /// 读盘。文件缺失或损坏时降级为空配置。
    async fn read_from_disk(&self) -> RawSettings {
        match tokio::fs::read_to_string(&self.path).await {
            Ok(content) => match serde_json::from_str::<RawSettings>(strip_bom(&content)) {
                Ok(settings) => settings,
                Err(error) => {
                    tracing::error!(path = %self.path.display(), %error, "设置文件解析失败，已降级为默认配置");
                    self.backup_broken().await;
                    RawSettings::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => RawSettings::default(),
            Err(error) => {
                tracing::error!(path = %self.path.display(), %error, "设置文件读取失败，已降级为默认配置");
                self.backup_broken().await;
                RawSettings::default()
            }
        }
    }

    /// 原子写：临时文件 + `rename`。
    async fn write_to_disk(&self, settings: &RawSettings) -> Result<()> {
        let Some(parent) = self.path.parent() else {
            return Err(ConfigError::RuntimeRootUnset.into());
        };
        tokio::fs::create_dir_all(parent).await?;

        let content = serde_json::to_string_pretty(settings).map_err(|error| {
            ConfigError::ConfigPathNotAllowed {
                path: format!("{} ({error})", self.path.display()),
            }
        })?;

        let temp_path = self.temp_path();
        tokio::fs::write(&temp_path, content).await?;
        // rename 在同目录内是原子替换；失败时清理临时文件，避免留下垃圾。
        if let Err(error) = tokio::fs::rename(&temp_path, &self.path).await {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(error.into());
        }
        Ok(())
    }

    /// 临时文件路径：带进程号与毫秒时间戳，避免并发写冲突。
    fn temp_path(&self) -> PathBuf {
        let millis = Timestamp::now().to_unix_millis().unwrap_or(0);
        let name = format!(
            "{}.{}.{}.tmp",
            self.path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("settings.json"),
            std::process::id(),
            millis
        );
        self.path.with_file_name(name)
    }

    /// 备份损坏的设置文件。
    ///
    /// 文件名中的 `:` 与 `.` 会被替换为 `-`（与现网一致），否则在 Windows 上非法。
    async fn backup_broken(&self) {
        if !self.path.exists() {
            return;
        }
        let stamp = Timestamp::now().as_str().replace([':', '.'], "-");
        let name = format!(
            "{}.broken-{stamp}",
            self.path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("settings.json")
        );
        let backup = self.path.with_file_name(name);
        if let Err(error) = tokio::fs::copy(&self.path, &backup).await {
            // 备份失败不应阻止应用以默认配置启动。
            tracing::warn!(%error, "备份损坏的设置文件失败");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivc_core::id::TargetId;
    use aivc_core::target::{ProviderId, TargetKind};

    fn store_in(dir: &tempfile::TempDir) -> SettingsStore {
        SettingsStore::new(dir.path().join("settings.json"))
    }

    fn target(provider: ProviderId, id: &str) -> Target {
        Target {
            id: TargetId::from(id),
            provider,
            label: format!("{provider} target"),
            kind: TargetKind::Local,
            distro: None,
            codex_home: None,
            available: true,
            detail: None,
        }
    }

    #[tokio::test]
    async fn a_bom_does_not_wipe_the_users_settings() {
        // **这是一条数据丢失的回归测试。**
        //
        // `serde_json` 不接受开头的 BOM。没有这一步时，用「UTF-8 with BOM」保存过的
        // 设置文件会被判成「损坏」：原文件被改名备份、设置换成默认值。用户手改一下
        // 设置再回来，目标缓存与网关端口全没了，只有一行 ERROR 日志能解释。
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(&dir);
        tokio::fs::write(store.path(), "\u{feff}{\"gatewayPort\": 18080}\n")
            .await
            .expect("写文件");

        let settings = store.snapshot().await;

        assert_eq!(settings.gateway_port, 18_080, "BOM 之后的设置必须被读到");
        // 更关键的是**没有把它当成损坏文件处理**：一旦走了那条路，用户的东西就没了。
        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .expect("目录")
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".broken-"))
            .collect();
        assert!(backups.is_empty(), "不该产生损坏备份：{backups:?}");
    }

    #[test]
    fn only_a_leading_bom_is_stripped() {
        assert_eq!(strip_bom("\u{feff}{}"), "{}");
        // 中间的 BOM 是用户数据的一部分，不该动它。
        assert_eq!(strip_bom("{\u{feff}}"), "{\u{feff}}");
        assert_eq!(strip_bom("{}"), "{}");
    }

    #[test]
    fn defaults_match_existing_behaviour() {
        let settings = RawSettings::default().normalize();
        assert_eq!(settings.gateway_port, 0);
        // gatewayEnabled 缺失视为开启。
        assert!(settings.gateway_enabled);
        // gatewayExternalApiEnabled 缺失视为关闭。
        assert!(!settings.gateway_external_api_enabled);
        assert_eq!(settings.gateway_failure_threshold, 1);
        assert_eq!(settings.gateway_circuit_failure_threshold, 3);
        assert_eq!(settings.gateway_circuit_duration_seconds, 60);
        assert!(settings.wsl_codex_homes.is_empty());
        assert!(settings.cached_targets.is_empty());
        assert!(settings.gateway_external_api_token_hash.is_empty());
    }

    #[test]
    fn gateway_enabled_uses_not_false_semantics() {
        // 现网是 `settings.gatewayEnabled !== false`：只有显式 false 才关闭。
        for raw in ["true", "\"yes\"", "0", "null", "\"false\""] {
            let parsed: RawSettings =
                serde_json::from_str(&format!("{{\"gatewayEnabled\":{raw}}}")).expect("parse");
            assert!(
                parsed.normalize().gateway_enabled,
                "gatewayEnabled={raw} 应视为开启"
            );
        }
        let off: RawSettings = serde_json::from_str("{\"gatewayEnabled\":false}").expect("parse");
        assert!(!off.normalize().gateway_enabled);
    }

    #[test]
    fn external_api_enabled_uses_strict_true_semantics() {
        // 现网是 `=== true`：只有显式 true 才开启。
        for raw in ["\"true\"", "1", "null", "\"yes\""] {
            let parsed: RawSettings =
                serde_json::from_str(&format!("{{\"gatewayExternalApiEnabled\":{raw}}}"))
                    .expect("parse");
            assert!(
                !parsed.normalize().gateway_external_api_enabled,
                "gatewayExternalApiEnabled={raw} 应视为关闭"
            );
        }
        let on: RawSettings =
            serde_json::from_str("{\"gatewayExternalApiEnabled\":true}").expect("parse");
        assert!(on.normalize().gateway_external_api_enabled);
    }

    #[test]
    fn out_of_range_and_wrong_typed_numbers_fall_back_per_field() {
        // 端口越界 → 0；阈值越界 → 默认；类型不符 → 默认；其余字段不受影响。
        let parsed: RawSettings = serde_json::from_str(
            r#"{
                "gatewayPort": 70000,
                "gatewayFailureThreshold": "3",
                "gatewayCircuitFailureThreshold": 99,
                "gatewayCircuitDurationSeconds": 5,
                "gatewayExternalApiTokenHash": "abc"
            }"#,
        )
        .expect("parse");
        let settings = parsed.normalize();
        assert_eq!(settings.gateway_port, 0);
        assert_eq!(settings.gateway_failure_threshold, 1);
        assert_eq!(settings.gateway_circuit_failure_threshold, 3);
        assert_eq!(settings.gateway_circuit_duration_seconds, 60);
        // 同文件中的合法字段仍然生效——这是逐字段容错的关键。
        assert_eq!(settings.gateway_external_api_token_hash, "abc");
    }

    #[test]
    fn integral_floats_are_accepted_like_javascript_number_is_integer() {
        // JS 里 8080 与 8080.0 是同一个值；非整数必须被拒绝。
        let accepted: RawSettings =
            serde_json::from_str("{\"gatewayPort\":8080.0}").expect("parse");
        assert_eq!(accepted.normalize().gateway_port, 8080);

        let rejected: RawSettings =
            serde_json::from_str("{\"gatewayPort\":8080.5}").expect("parse");
        assert_eq!(rejected.normalize().gateway_port, 0);
    }

    #[test]
    fn boundary_values_are_accepted_inclusively() {
        for (port, expected) in [(0, 0), (1, 1), (65_535, 65_535)] {
            let parsed: RawSettings =
                serde_json::from_str(&format!("{{\"gatewayPort\":{port}}}")).expect("parse");
            assert_eq!(parsed.normalize().gateway_port, expected);
        }
        for (value, expected) in [(1u8, 1u8), (10, 10)] {
            let parsed: RawSettings =
                serde_json::from_str(&format!("{{\"gatewayFailureThreshold\":{value}}}"))
                    .expect("parse");
            assert_eq!(parsed.normalize().gateway_failure_threshold, expected);
        }
    }

    #[test]
    fn legacy_workspace_presets_field_is_ignored_on_read() {
        // 旧字段已迁移到 SQLite，不得再作为权威来源。
        let parsed: RawSettings = serde_json::from_str(
            r#"{"workspacePresets":[{"id":"x","name":"n","cwd":"/w","updatedAt":"t"}]}"#,
        )
        .expect("parse");
        let settings = parsed.normalize();
        assert!(settings.cached_targets.is_empty());
        assert_eq!(settings.gateway_port, 0);
    }

    #[test]
    fn merge_cached_targets_keeps_other_providers() {
        let mut raw = RawSettings::default();
        raw.merge_cached_targets(&[
            target(ProviderId::Codex, "local"),
            target(ProviderId::Gemini, "gemini:local"),
        ]);
        // 只刷新 Codex 时，Gemini 的目标必须保留。
        raw.merge_cached_targets(&[target(ProviderId::Codex, "wsl:Ubuntu")]);

        let settings = raw.normalize();
        let ids: Vec<&str> = settings
            .cached_targets
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        assert!(ids.contains(&"gemini:local"));
        assert!(ids.contains(&"wsl:Ubuntu"));
        // 被覆盖平台的旧目标已移除。
        assert!(!ids.contains(&"local"));
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn merge_cached_targets_with_empty_input_is_a_no_op() {
        let mut raw = RawSettings::default();
        raw.merge_cached_targets(&[target(ProviderId::Codex, "local")]);
        raw.merge_cached_targets(&[]);
        assert_eq!(raw.normalize().cached_targets.len(), 1);
    }

    #[test]
    fn wsl_codex_home_override_is_trimmed_and_removable() {
        let mut raw = RawSettings::default();
        raw.set_wsl_codex_home("Ubuntu", "  /home/akim/.codex  ");
        assert_eq!(
            raw.normalize()
                .wsl_codex_homes
                .get("Ubuntu")
                .map(String::as_str),
            Some("/home/akim/.codex")
        );

        raw.clear_wsl_codex_home("Ubuntu");
        assert!(raw.normalize().wsl_codex_homes.is_empty());
        // 清除不存在的键不应报错。
        raw.clear_wsl_codex_home("Missing");
    }

    #[tokio::test]
    async fn missing_file_yields_defaults_without_creating_anything() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(&dir);
        let settings = store.snapshot().await;
        assert_eq!(settings, AppSettings::default());
        assert!(!store.path().exists());
    }

    #[tokio::test]
    async fn update_persists_and_is_visible_to_subsequent_reads() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(&dir);

        let updated = store
            .update(|raw| {
                raw.set_gateway_port(18_080);
                raw.set_gateway_enabled(false);
                raw.set_gateway_failure_threshold(5);
            })
            .await
            .expect("update");

        assert_eq!(updated.gateway_port, 18_080);
        assert!(!updated.gateway_enabled);
        assert_eq!(updated.gateway_failure_threshold, 5);

        // 新实例从磁盘读回同样的值，证明确实落盘。
        let reloaded = store_in(&dir).snapshot().await;
        assert_eq!(reloaded.gateway_port, 18_080);
        assert!(!reloaded.gateway_enabled);
        assert_eq!(reloaded.gateway_failure_threshold, 5);
    }

    #[tokio::test]
    async fn update_does_not_leave_temp_files_behind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(&dir);
        store
            .update(|raw| raw.set_gateway_port(1234))
            .await
            .expect("update");

        let leftovers: Vec<String> = std::fs::read_dir(dir.path())
            .expect("read_dir")
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.to_ascii_lowercase().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "残留临时文件：{leftovers:?}");
    }

    #[tokio::test]
    async fn concurrent_updates_are_serialized_and_do_not_lose_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(store_in(&dir));

        // 两个并发写入改的是不同字段：若读-改-写没有串行化，后写者会用陈旧快照
        // 覆盖先写者的结果。
        let first = {
            let store = Arc::clone(&store);
            tokio::spawn(async move {
                store
                    .update(|raw| raw.set_gateway_port(9999))
                    .await
                    .expect("update port");
            })
        };
        let second = {
            let store = Arc::clone(&store);
            tokio::spawn(async move {
                store
                    .update(|raw| raw.set_gateway_failure_threshold(7))
                    .await
                    .expect("update threshold");
            })
        };
        first.await.expect("join first");
        second.await.expect("join second");

        let settings = store.snapshot().await;
        assert_eq!(settings.gateway_port, 9999);
        assert_eq!(settings.gateway_failure_threshold, 7);
    }

    #[tokio::test]
    async fn corrupted_file_degrades_to_defaults_and_is_backed_up() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(&dir);
        std::fs::write(store.path(), "{ this is not json").expect("write corrupt");

        let settings = store.snapshot().await;
        assert_eq!(settings, AppSettings::default());

        let backups: Vec<String> = std::fs::read_dir(dir.path())
            .expect("read_dir")
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".broken-"))
            .collect();
        assert_eq!(backups.len(), 1, "应恰好产生一份备份：{backups:?}");
        // 备份名不得含 Windows 非法字符。
        assert!(!backups[0].contains(':'));
    }

    #[tokio::test]
    async fn valid_json_with_wrong_field_types_still_loads() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(&dir);
        std::fs::write(
            store.path(),
            r#"{"gatewayPort":"8080","gatewayEnabled":"true","cachedTargets":"nope"}"#,
        )
        .expect("write");

        let settings = store.snapshot().await;
        assert_eq!(settings.gateway_port, 0);
        assert!(settings.gateway_enabled);
        assert!(settings.cached_targets.is_empty());
        // 类型不符不属于「损坏」，不应产生备份文件。
        let backups = std::fs::read_dir(dir.path())
            .expect("read_dir")
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".broken-"))
            .count();
        assert_eq!(backups, 0);
    }

    #[tokio::test]
    async fn invalidate_forces_a_fresh_disk_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(&dir);
        store
            .update(|raw| raw.set_gateway_port(1111))
            .await
            .expect("update");
        assert_eq!(store.snapshot().await.gateway_port, 1111);

        // 外部修改文件后，缓存必须能被丢弃。
        std::fs::write(store.path(), r#"{"gatewayPort":2222}"#).expect("write");
        assert_eq!(store.snapshot().await.gateway_port, 1111, "缓存应仍然生效");
        store.invalidate().await;
        assert_eq!(store.snapshot().await.gateway_port, 2222);
    }
}
