import { useState } from "react";

import { Button, ChipGroup, ToolbarSpacer } from "../components";
import { DiagnosticsPanel } from "./config/diagnostics-panel";
import { EnvironmentPanel } from "./config/environment-panel";
import { GatewayPanel } from "./config/gateway-panel";
import { PresetsPanel } from "./config/presets-panel";
import { SkillsPanel } from "./config/skills-panel";
import { VendorsPanel } from "./config/vendors-panel";
import { GatewayFailurePanel } from "./gateway-failure-panel";
import { GatewayLogPanel } from "./gateway-log-panel";
import { GatewayUsagePanel } from "./gateway-usage-panel";
import type { AiProviderId, AiProviderSummary, AiTarget } from "../types";

/**
 * 配置覆盖层。
 *
 * # 为什么是覆盖层而不是侧栏的一个区域
 *
 * 这四类东西（供应商 / 技能 / 网关 / 环境）**都不是「我正在看的会话」**，把它们放进
 * 侧栏会和会话列表抢同一块空间，而且每切一次都要重新布局。旧平台也是这个形状。
 *
 * 覆盖层还带来一个好处：它是**临时**的——关掉就回到原来的视图，不需要「返回」。
 */

/**
 * 配置面的分页。导出给命令面板用——它要能直接打开某一页。
 *
 * 分成「网关」与「用量 / 异常 / 日志」三页而不是全塞进「网关」一页：
 *
 * - 「网关」是**设置**（开关、端口、故障规则、外部接入），改的是行为；
 * - 另外三页是**观测**（花在哪、为什么失败、有哪些日志），只看不改。
 *
 * 混在一页里的后果是实测过的：`config-overlay.tsx` 一度到 1380 行，
 * 打开「网关」要同时渲染设置表单与三张只读表格，滚动条拉到一半就找不到开关了。
 */
export type ConfigTab =
  | "vendors"
  | "skills"
  | "gateway"
  | "usage"
  | "failures"
  | "logs"
  | "environment"
  | "presets"
  | "diagnostics";


export function ConfigOverlay({
  providerId,
  providers,
  targetId,
  targets,
  initialTab = "vendors",
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
  initialTab?: ConfigTab;
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
  const [tab, setTab] = useState<ConfigTab>(initialTab);

  return (
    <div className="overlay" role="dialog" aria-modal="true" aria-label="配置">
      <div className="overlay-panel">
        <header className="overlay-head">
          <h2>配置</h2>
          <ToolbarSpacer />
          <Button small onClick={onClose}>
            关闭
          </Button>
        </header>

        <div className="overlay-tabs">
          <ChipGroup
            value={tab}
            onChange={(value) => setTab(value as ConfigTab)}
            options={[
              { value: "vendors", label: "供应商" },
              { value: "skills", label: "技能" },
              { value: "gateway", label: "网关" },
              { value: "usage", label: "用量" },
              { value: "failures", label: "异常" },
              { value: "logs", label: "日志" },
              { value: "environment", label: "环境" },
              { value: "presets", label: "工作区预设" },
              { value: "diagnostics", label: "诊断" }
            ]}
          />
        </div>

        <div className="overlay-body">
          {tab === "vendors" && (
            <VendorsPanel
              providerId={providerId}
              providers={providers}
              onError={onError}
              onNotify={onNotify}
            />
          )}
          {tab === "skills" && (
            <SkillsPanel targetId={targetId} onError={onError} onNotify={onNotify} />
          )}
          {tab === "gateway" && <GatewayPanel onError={onError} onNotify={onNotify} />}
          {/* 观测三页：看，不改。设置仍然在「网关」那一页。 */}
          {tab === "usage" && <GatewayUsagePanel onError={onError} />}
          {tab === "failures" && (
            <GatewayFailurePanel providerId={providerId} onError={onError} />
          )}
          {tab === "logs" && <GatewayLogPanel onError={onError} />}
          {tab === "environment" && (
            <EnvironmentPanel
              providerId={providerId}
              targetId={targetId}
              onError={onError}
              onNotify={onNotify}
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
        </div>
      </div>
    </div>
  );
}
