//! 强类型标识。
//!
//! 现网用裸 `string` 表示会话编号、目标、供应商、路由、终端等标识，跨模块传参时
//! 极易混淆（例如把 `targetId` 当成 `sessionId`）。这里用零成本 newtype 把每类
//! 标识区分开，编译期即可拦住这类错误，序列化后仍是普通字符串，与现网 JSON 契约
//! 完全一致。

use std::fmt;

/// 声明一个字符串 newtype 标识。
macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// 由任意字符串构造。调用方负责保证内容合法（非空、格式正确）。
            #[must_use]
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// 以字符串切片借用内部值。
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// 取出内部字符串。
            #[must_use]
            pub fn into_string(self) -> String {
                self.0
            }

            /// 内容是否为空。
            #[must_use]
            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

define_id! {
    /// 会话编号。
    ///
    /// Codex 为 JSONL 文件名中的 UUID；Claude/Gemini/Qoder 为各自会话文件的编号。
    SessionId
}

define_id! {
    /// 目标标识。
    ///
    /// 本机为 `local`；WSL 目标 Codex 为 `wsl:<distro>`，其余平台为
    /// `<provider>:wsl:<distro>`。解析规则见 `aivc_core::target`。
    TargetId
}

define_id! {
    /// 供应商编号（UUID v4）。
    VendorId
}

define_id! {
    /// 网关路由编号（UUID v4）。
    RouteId
}

define_id! {
    /// 终端会话编号（UUID v4）。
    TerminalId
}

define_id! {
    /// 单次网关请求编号（UUID v4）。
    RequestId
}

define_id! {
    /// Skill 目录名。
    SkillName
}

define_id! {
    /// 工作区预设编号（UUID v4）。
    PresetId
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_serializes_as_plain_string_for_wire_compatibility() {
        let id = SessionId::new("0191f2a3-0000-7000-8000-000000000000");
        let json = serde_json::to_string(&id).expect("serialize");
        assert_eq!(json, "\"0191f2a3-0000-7000-8000-000000000000\"");

        let back: SessionId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, id);
    }

    #[test]
    fn distinct_id_types_are_not_interchangeable() {
        // 类型不同即无法比较，这是本模块存在的全部意义；此处只验证各自的转换正确。
        let target = TargetId::from("wsl:Ubuntu");
        assert_eq!(target.as_str(), "wsl:Ubuntu");
        assert_eq!(target.to_string(), "wsl:Ubuntu");
        assert!(!target.is_empty());
        assert_eq!(target.clone().into_string(), "wsl:Ubuntu");
        assert_eq!(target.as_ref(), "wsl:Ubuntu");
    }

    #[test]
    fn empty_id_is_reported_as_empty() {
        assert!(VendorId::from("").is_empty());
    }
}
