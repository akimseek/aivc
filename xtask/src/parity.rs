//! 验收矩阵的可校验清单。
//!
//! # 这张表解决什么问题
//!
//! `docs/rust-rewrite/03-parity-matrix-and-migration.md` 列了 185 条行为契约，但那是
//! **文档**：它不会因为有人删掉一条测试而失效。于是「矩阵说覆盖了，实际没有」这种漂移
//! 会一直存在，直到验收时才被发现——而那时已经很难定位是哪一轮弄丢的。
//!
//! 这里把「条目 → 覆盖它的测试名前缀」写成可执行的数据，由 `cargo xtask parity` 校验：
//!
//! - **已声明覆盖**的条目，必须在测试列表里找到至少一个匹配的测试；找不到就报错。
//! - **尚未纳入**的条目照样打印出来，标注为未覆盖，让缺口可见而不是被忽略。
//!
//! # 为什么断言的是「前缀」而不是完整测试名
//!
//! 完整测试名会随模块结构变动（这在本项目里发生过多次），写成前缀能容忍移动与分组，
//! 但仍然能发现「整组测试消失」。断言完整名字的收益很小、维护成本很高。

/// 一条验收条目。
#[derive(Debug, Clone, Copy)]
pub struct Item {
    /// 矩阵编号（如 `B4`）。
    pub id: &'static str,
    /// 行为描述（与矩阵文档一致的精简版）。
    pub behavior: &'static str,
    /// 覆盖它的测试名前缀；空表示尚未纳入自动校验。
    pub covers: &'static [&'static str],
}

/// 已纳入自动校验的条目。
///
/// 只列**当前确实有测试支撑**的条目。没写的条目会被 [`matrix`] 归入「未纳入」，
/// 这在报告里是显式可见的，不会被伪装成通过。
pub const ITEMS: &[Item] = &[
    // --- B. 路径与配置 ---
    Item {
        id: "B1",
        behavior: "运行根三形态派生",
        covers: &["config::paths::tests"],
    },
    Item {
        id: "B2",
        behavior: "data/ 与 logs/ 从运行根派生",
        covers: &["config::paths::tests"],
    },
    Item {
        id: "B4",
        behavior: "settings.json 全字段默认值与校验范围",
        covers: &["config::settings::tests"],
    },
    Item {
        id: "B6",
        behavior: "损坏配置降级 + 备份",
        covers: &["config::settings::tests"],
    },
    Item {
        id: "B7",
        behavior: "cachedTargets 按 provider 合并覆盖",
        covers: &["config::settings::tests"],
    },
    Item {
        id: "B8",
        behavior: "wslCodexHomes 覆盖优先于自动探测",
        covers: &["config::settings::tests"],
    },
    // --- C. 数据库 ---
    Item {
        id: "C1",
        behavior: "表结构、列名、主键、唯一索引",
        covers: &["db::migrations::tests", "db::facade::tests"],
    },
    Item {
        id: "C3",
        behavior: "WAL + busy_timeout + foreign_keys",
        covers: &["db::connection::tests"],
    },
    Item {
        id: "C4",
        behavior: "版本化迁移（V1 起）",
        covers: &["db::migrations::tests"],
    },
    Item {
        id: "C6",
        behavior: "写入串行且失败回滚",
        covers: &["db::facade::tests", "db::connection::tests"],
    },
    // --- D. Provider 注册与目标 ---
    Item {
        id: "D1",
        behavior: "四平台注册且能力矩阵与实现一致",
        covers: &["registry_tests"],
    },
    Item {
        id: "D2",
        behavior: "未支持能力在分发处显式拒绝",
        covers: &["registry_tests"],
    },
    // --- E. 会话读取与分页 ---
    Item {
        id: "E1",
        behavior: "会话解析（四平台）",
        covers: &[
            "codex::parser::tests",
            "claude::parser::tests",
            "gemini::parser::tests",
            "qoder::parser::tests",
        ],
    },
    Item {
        id: "E2",
        behavior: "消息分页与锚点",
        covers: &["paging::tests", "codex::provider::tests"],
    },
    Item {
        id: "E3",
        behavior: "路径围栏：拒绝 sessions 目录之外的文件",
        covers: &["codex::paths::tests", "qoder::paths::tests"],
    },
    Item {
        id: "E4",
        behavior: "列表与摘要读取",
        covers: &["codex::listing::tests"],
    },
    // --- F. 会话变更 ---
    Item {
        id: "F1",
        behavior: "删除 / 恢复 / 清除（四平台）",
        covers: &[
            "codex::provider::tests",
            "claude::provider::tests",
            "gemini::provider::tests",
            "qoder::provider::tests",
        ],
    },
    Item {
        id: "F2",
        behavior: "分支与复制",
        covers: &[
            "codex::branch::tests",
            "claude::branch::tests",
            "gemini::branch::tests",
        ],
    },
    Item {
        id: "F3",
        behavior: "导出（三格式）",
        covers: &["export::tests"],
    },
    Item {
        id: "F4",
        behavior: "Qoder 多产物原子移动与回滚",
        covers: &["qoder::provider::tests"],
    },
    // --- G. Gateway ---
    Item {
        id: "G1",
        behavior: "路由注册与常量时间令牌比较",
        covers: &["routes::tests"],
    },
    Item {
        id: "G2",
        behavior: "上游请求头剔除与 URL 拼接",
        covers: &["upstream::tests"],
    },
    Item {
        id: "G3",
        behavior: "候选选择与熔断三态",
        covers: &["failover::tests"],
    },
    Item {
        id: "G4",
        behavior: "尝试规划与失败处置",
        covers: &["proxy::tests"],
    },
    Item {
        id: "G5",
        behavior: "响应探测：识别 200 但内容是错误",
        covers: &["response::tests"],
    },
    Item {
        id: "G6",
        behavior: "请求记录：每条路径都落库",
        covers: &["record::tests", "server::tests"],
    },
    Item {
        id: "G7",
        behavior: "端到端转发与故障转移",
        covers: &["server::tests"],
    },
    Item {
        id: "G8",
        behavior: "仓储来源：供应商/规则/设置/记录",
        covers: &["source::tests"],
    },
    // --- I. 终端运行时 ---
    Item {
        id: "I1",
        behavior: "状态检测：等待确认 / 出错 / 本轮已完成",
        covers: &["status::tests"],
    },
    Item {
        id: "I2",
        behavior: "四平台命令构造与网关路由注入",
        covers: &["command::tests"],
    },
    // --- 跨阶段的编排与命令面 ---
    Item {
        id: "P7-1",
        behavior: "参数校验与命令名注册表",
        covers: &["validate::tests", "commands::tests"],
    },
    Item {
        id: "P7-2",
        behavior: "命令分发：已接线与未接线的区分",
        covers: &["router::tests"],
    },
    Item {
        id: "P7-3",
        behavior: "命令域：会话设置 / 预设 / 网关 / 供应商",
        covers: &["settings::tests", "gateway::tests", "vendor::tests"],
    },
    Item {
        id: "P7-4",
        behavior: "组装根：一次启动建好全部组件",
        covers: &["tests"],
    },
    Item {
        id: "P7-5",
        behavior: "Tauri 命令面：错误映射为消息文本",
        covers: &["tauri_app::tests"],
    },
    Item {
        id: "P7-6",
        behavior: "托盘：提示文字、菜单结构与顺序",
        covers: &["tray::tests"],
    },
    Item {
        id: "P7-7",
        behavior: "退出清理顺序（PTY → Gateway → flush），幂等且失败不中断",
        covers: &["shutdown::tests"],
    },
    Item {
        id: "P7-8",
        behavior: "窗口规格：尺寸、最小尺寸、标题、背景色（矩阵 A1）",
        covers: &["window_spec_tests"],
    },
    Item {
        id: "P7-9",
        behavior: "导航白名单与外链转交系统浏览器（矩阵 A3）",
        covers: &["harden::tests"],
    },
    // --- P8. 端到端对拍（Node 作为 oracle） ---
    Item {
        id: "P8-1",
        behavior: "终端状态检测器与现网逐事件一致",
        covers: &["the_detector_matches_the_node_oracle"],
    },
    Item {
        id: "P8-2",
        behavior: "Codex 会话解析与现网逐字段一致",
        covers: &["the_parser_matches_the_node_oracle"],
    },
    Item {
        id: "P8-3",
        behavior: "shell 引号/命令构造/参数解析/目标编号与现网逐值一致",
        covers: &[
            "quotes_match_the_node_oracle",
            "cli_argument_parsing_matches_the_node_oracle",
        ],
    },
    Item {
        id: "P8-4",
        behavior: "路径围栏与挂载路径转换与现网一致",
        covers: &[
            "path_fencing_matches_the_node_oracle",
            "mount_path_conversion_matches_the_node_oracle",
        ],
    },
    // --- J. WSL ---
    Item {
        id: "J1",
        behavior: "WSL 输出解码与路径围栏",
        covers: &["wsl::decode::tests"],
    },
];

/// 未纳入自动校验的分区（矩阵中存在但当前没有可断言覆盖）。
///
/// 显式列出来，让报告里能区分「已覆盖」「未覆盖」与「还不知道」。
///
/// # 为什么把「做不到」也写进来
///
/// 「未纳入」有两种性质完全不同的原因：**还没做**与**做不了**。只写「未纳入」会让
/// 后来者反复尝试同一件不可能的事。因此这里把原因写清楚。
pub const UNCOVERED_SECTIONS: &[(&str, &str)] = &[
    (
        "A",
        "桌面外壳与生命周期：**窗口、托盘、退出清理顺序已完成**；关闭三选一（A4，需渲染层弹窗）、\
         系统通知（A7）、CSP（A2）未做",
    ),
    (
        "H",
        "供应商：**CRUD、启用编排、白名单、备份、模型查询、余额查询、认证模式已完成**；\
         余额查询的四种响应模板与 DeepSeek 预设未做（后者在渲染层，前端复用即具备）",
    ),
    (
        "I3",
        "PTY 会话运行时：**已能运行**（63 项测试，泄漏与 DSR 阻塞均已修复）。\
         未覆盖的是**真实终端交互**——需要在一个能点界面的人手里跑一遍",
    ),
    (
        "K",
        "Skill 与 CLI 环境：**已实现**（Skill 根/停用/导入原子换入、环境探测、安装）。\
         未覆盖的是 `skill:import` 需要的原生文件对话框（离线缺依赖）",
    ),
    (
        "L",
        "渲染层：**已被真跑多次，并借此发现 7 个自动化测试测不出的缺陷**（终端泄漏、\
         事件权限、桥参数丢弃 ×2、测试 launcher 抢占生产、WSL worker 挂死、bash 抢占）。\
         但 L1–L20 那 20 条**交互行为本身仍未逐条核对**",
    ),
    (
        "P8-命令面",
        "17 条未接线命令：其中 4 条**离线做不了**（剪贴板 2、目录选择、系统通知，均需新依赖），\
         其余是薄封装或依赖未实现特性（终端路由切换、其余平台终端构造器）",
    ),
    (
        "P8-打包",
        "性能基准与三平台打包**未开始**，且都需要真实多平台环境",
    ),
    (
        "P8-claude-gemini-qoder",
        "三平台解析对拍：其解析函数是模块私有，导出的只有整套 provider API（依赖 node:fs、\
         数据库、设置）。要直接对拍必须用 Node loader 打桩 8 个模块（含一个假数据库），\
         成本与脆弱度都高。**共享的底层原语（safeJsonParse/stringField/clampText/shellQuote/\
         isInsidePosixDir/getWslDistroFromProviderTarget）已单独对拍**，未覆盖的是各平台\
         自己的消息形状判定。",
    ),
];

/// 全部条目。
#[must_use]
pub fn matrix() -> &'static [Item] {
    ITEMS
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn item_ids_are_unique() {
        let ids: BTreeSet<&str> = ITEMS.iter().map(|item| item.id).collect();
        assert_eq!(ids.len(), ITEMS.len(), "有重复的条目编号");
    }

    #[test]
    fn every_item_has_a_description_and_coverage() {
        for item in ITEMS {
            assert!(!item.id.is_empty(), "条目缺少编号");
            assert!(!item.behavior.is_empty(), "{} 缺少描述", item.id);
            assert!(
                !item.covers.is_empty(),
                "{} 列在已校验表里却没有声明覆盖测试",
                item.id
            );
            for prefix in item.covers {
                assert!(!prefix.is_empty(), "{} 有空的前缀", item.id);
            }
        }
    }

    #[test]
    fn ids_follow_the_documented_shape() {
        // 编号要么是矩阵里的 `字母+数字`，要么是跨阶段的 `P7-x`。
        for item in ITEMS {
            let looks_like_matrix = item
                .id
                .chars()
                .next()
                .is_some_and(|first| first.is_ascii_uppercase())
                && item.id[1..].chars().all(|ch| ch.is_ascii_digit());
            let looks_like_phase = item.id.starts_with('P') && item.id.contains('-');
            assert!(
                looks_like_matrix || looks_like_phase,
                "{} 的编号形状不符合约定",
                item.id
            );
        }
    }
}
