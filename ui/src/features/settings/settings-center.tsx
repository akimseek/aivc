import { useEffect, useState } from "react";

import { DiagnosticsPanel } from "../../app/config/diagnostics-panel";
import { EnvironmentPanel } from "../../app/config/environment-panel";
import { GatewayPanel } from "../../app/config/gateway-panel";
import { PresetsPanel } from "../../app/config/presets-panel";
import { SkillsPanel } from "../../app/config/skills-panel";
import { VendorsPanel } from "../../app/config/vendors-panel";
import { GatewayFailurePanel } from "../../app/gateway-failure-panel";
import { GatewayLogPanel } from "../../app/gateway-log-panel";
import { GatewayUsagePanel } from "../../app/gateway-usage-panel";
import { supportsSessionSettings } from "../../app/provider-capabilities";
import { Icon, hasLayerAbove } from "../../components";
import type { AiProviderId, AiProviderSummary, AiTarget } from "../../types";
import { AboutPanel } from "./panels/about-panel";
import { SETTINGS_GROUPS, type SettingsTab } from "./settings-nav";

export type { SettingsTab } from "./settings-nav";

/**
 * 设置中心（模态两栏）。
 *
 * 左列是分组导航，右列是面板。与旧的扁平九标签覆盖层相比，差别只在**信息架构**：
 * 九个扁平标签收进四个组、多出「关于」页承接菜单栏迁移过来的动作（spec §11）。
 * 面板本身全部原样复用——它们的业务逻辑与测试是既有资产，重排发生在批次二。
 *
 * # 为什么自绘外壳而不套 `Dialog`
 *
 * 它比 `Dialog` 多一层两栏结构（224px 导航 + 面板区），硬套会把通用弹层撑成
 * 「什么都能装」的上帝组件。遮罩、层级、圆角仍与 `Dialog` 用同一批 token，
 * 观感不会分叉。
 */
export function SettingsCenter({
  providerId,
  providers,
  targetId,
  targets,
  selectedTarget,
  initialTab = "vendors",
  version,
  onOpenSessionSettings,
  onQuit,
  onOpenLogDirectory,
  onClose,
  onError,
  onNotify
}: {
  providerId: AiProviderId | "";
  /**
   * 全部平台。
   *
   * 供应商页要它——那里管的是**所有厂商**的供应商，厂商的下拉与筛选都从这里取，
   * 因此显示名与侧栏逐字一致。缺了它那一页只能拿编号当名字。
   */
  providers: AiProviderSummary[];
  targetId: string;
  /** 全部目标：诊断页要把每个目标的状态列出来。 */
  targets: AiTarget[];
  /** 当前选中的目标。环境面板的「WSL 会话目录」卡只在 WSL 且平台支持会话设置时显示。 */
  selectedTarget: AiTarget | null;
  initialTab?: SettingsTab;
  /**
   * 应用版本。
   *
   * 由 `app.tsx` **取一次**后传进来（`app:get-version`）：版本是进程级的常量，
   * 每个面板各取一次只是多几次 IPC，还会让「读不到」的兜底分散在几处。
   */
  version: string;
  /**
   * 打开现有的「设置会话」对话框。
   *
   * 对话框本体（`session-settings-dialog.tsx`）与它的状态仍在 `app.tsx` 那一层：
   * 这里只把入口从菜单栏挪到环境面板，不重新实现一遍。
   * `undefined` = 当前平台不支持（上层不构造这个动作），面板按「缺入口不渲染」处理。
   */
  onOpenSessionSettings?: () => void;
  /** 退出控制台（走窗口关闭 → 后端的退出确认流程）。 */
  onQuit: () => void;
  /** 在文件管理器里打开日志目录。 */
  onOpenLogDirectory: () => void;
  onClose: () => void;
  onError: (message: string) => void;
  /**
   * 一句话成功反馈。
   *
   * 与 `onError` 成对：旧平台**每个动作都有结果回应**，而失败与成功走的是两条路
   * （失败常驻在状态栏、成功几秒后自动消失）。面板只管说「做完了什么」，
   * 通道由上层给——这样「反馈长什么样」只有一处实现。
   */
  onNotify?: (message: string) => void;
}) {
  const [tab, setTab] = useState<SettingsTab>(initialTab);

  /**
   * Escape 关闭。
   *
   * 键盘用户此前**没有出口**：这个模态只有右上角那个 ✕，而它比 `Dialog` 自绘
   * （`Dialog` 家族一直有 Escape）。
   *
   * 有更内层的东西开着时让给它：面板里能再开对话框（供应商编辑等），退出确认框
   * 还会直接盖在整个设置中心之上——不判断的话按一次 Escape 会**同时**关掉内层
   * 对话框与整个设置中心，用户丢的是刚填了一半的东西。
   */
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (hasLayerAbove("modal")) return;
      onClose();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  return (
    <div className="settings-overlay" role="dialog" aria-modal="true" aria-label="系统配置与服务中心">
      <div className="settings-panel">
        <header className="settings-head">
          <span className="settings-head-icon">
            <Icon name="settings" size={16} />
          </span>
          <div className="settings-head-text">
            <h2>系统配置与服务中心</h2>
            <p>统一管理供应商、网关路由、观测审计、运行时扩展及系统信息</p>
          </div>
          <button type="button" className="icon-btn" title="关闭" aria-label="关闭" onClick={onClose}>
            <Icon name="close" size={16} />
          </button>
        </header>

        <div className="settings-body">
          <nav className="settings-nav" aria-label="设置分类">
            {SETTINGS_GROUPS.map((group) => (
              <div className="settings-group" key={group.id}>
                <div className="settings-group-label">
                  <Icon name={group.icon} size={13} />
                  <span>{group.label}</span>
                </div>
                {group.tabs.map((item) => (
                  <button
                    key={item.id}
                    type="button"
                    className={`settings-nav-item${tab === item.id ? " on" : ""}`}
                    // 当前页对读屏要说得出来：`.on` 只是视觉上的高亮。
                    // 非当前项**不写**这个属性（写 `aria-current="false"` 等于给每一项
                    // 都加一句话），这也是 ARIA 的推荐写法。
                    aria-current={tab === item.id ? "page" : undefined}
                    onClick={() => setTab(item.id)}
                  >
                    {item.label}
                  </button>
                ))}
              </div>
            ))}
          </nav>

          <div className="settings-content">
            {tab === "vendors" && (
              <VendorsPanel
                providerId={providerId}
                providers={providers}
                onError={onError}
                onNotify={onNotify}
              />
            )}
            {tab === "gateway" && (
              <GatewayPanel
                // 平台名以**后端 label** 为准（规则表与侧栏要逐字一致）。
                providers={providers}
                onError={onError}
                onNotify={onNotify}
              />
            )}
            {tab === "usage" && <GatewayUsagePanel onError={onError} />}
            {tab === "failures" && <GatewayFailurePanel providerId={providerId} onError={onError} />}
            {tab === "logs" && <GatewayLogPanel onError={onError} onNotify={onNotify} />}
            {tab === "skills" && (
              <SkillsPanel targetId={targetId} onError={onError} onNotify={onNotify} />
            )}
            {tab === "environment" && (
              <EnvironmentPanel
                providerId={providerId}
                targetId={targetId}
                selectedTarget={selectedTarget}
                // 能力从平台摘要**现算**：这一层是 `providers` 与 `providerId` 的最近
                // 持有者，判定（含缺键 = 不支持）只由 `provider-capabilities.ts` 解释。
                supportsSessionSettings={supportsSessionSettings(providers, providerId)}
                // 版本由 `app.tsx` 取一次后传到这里，再交给需要它的两页（关于 / 环境）——
                // 面板自己再取一次只是多一次 IPC，读到的还是同一个值。
                version={version}
                onError={onError}
                onNotify={onNotify}
                onOpenSessionSettings={onOpenSessionSettings}
              />
            )}
            {tab === "presets" && <PresetsPanel onError={onError} />}
            {tab === "diagnostics" && (
              <DiagnosticsPanel
                targets={targets}
                selectedTargetId={targetId}
                // 能力矩阵从平台摘要取，因此这里也要传进去。
                providers={providers}
                selectedProviderId={providerId}
              />
            )}
            {tab === "about" && (
              <AboutPanel
                version={version}
                onOpenLogDirectory={onOpenLogDirectory}
                onQuit={onQuit}
              />
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
