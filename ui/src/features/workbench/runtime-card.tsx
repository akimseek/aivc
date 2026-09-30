import { Badge, Card, DescriptionList, ProgressBar } from "../../components";
import { gatewayPortLabel, gatewayTone } from "../../app/gateway-address";
import { formatTokens } from "../../app/session-display";
import type { AiProviderSummary, AiSession, AiTarget, GatewayPortStatus } from "../../types";
import { progressTone, sessionLabel } from "./workbench-cards";

/**
 * 运行上下文卡：这一屏最上面的一格，回答「我现在在哪、用的是谁、跑到哪了」。
 *
 * 只做展示——供应商清单、会话摘要、网关状态都由 `WorkbenchSidebar` 取好传进来。
 *
 * # 网关为什么还留在这里
 *
 * 状态栏上也有一个网关端口，但那说的是**全局**；这一格说的是**当前这路会话**
 * 正在用的路由，两者相邻读起来才是一件事（旧工作台也是这么放的，别处没有替代）。
 */
export function RuntimeCard({
  provider,
  target,
  session,
  gateway
}: {
  provider: AiProviderSummary | undefined;
  target: AiTarget | null;
  session: AiSession | null;
  gateway: GatewayPortStatus | null;
}) {
  const usage = session?.usage;
  const percent = usage?.contextPercent;
  const contextWindow = usage?.contextWindow;
  const consumed = usage?.total?.totalTokens;

  return (
    <Card title="运行上下文" description="当前会话与路由的即时状态。">
      <div className="workbench-badges">
        <Badge tone="accent">{provider?.label ?? "未选择平台"}</Badge>
        <Badge>{target?.label ?? "未选择目标"}</Badge>
      </div>
      <DescriptionList
        items={[
          { label: "会话", value: sessionLabel(session) },
          { label: "模型", value: session?.model ?? "-" },
          {
            label: "已消耗",
            value: formatTokens(consumed ?? 0),
            tone: consumed ? "ok" : undefined
          },
          {
            label: "网关",
            // 与状态栏共用同一个判断：两边各写一遍时，一处算错只有一格显示不对。
            value: gatewayPortLabel(gateway),
            tone: gatewayTone(gateway) === "ok" ? "ok" : undefined
          }
        ]}
      />
      <div className="workbench-context">
        <div className="workbench-context-head">
          <span>上下文占用</span>
          <span className="ui-num">
            已用 {percent !== undefined ? `${percent}%` : "-"} /{" "}
            {contextWindow !== undefined ? formatTokens(contextWindow) : "-"}
          </span>
        </div>
        {/* 没有数字就不画条：一根 0% 的空条会被读成「上下文是空的」。 */}
        {percent !== undefined && (
          <ProgressBar value={percent} tone={progressTone(percent)} label="上下文占用" />
        )}
      </div>
    </Card>
  );
}
