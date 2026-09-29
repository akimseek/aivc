import { useCallback, useState } from "react";

import { Button, Card, DescriptionList, EmptyState, LoadingBlock, Toolbar, ToolbarSpacer } from "../components";
import * as api from "../ipc/commands";
import type { AiProviderSummary, AiSession, AiTarget, ApiVendor, GatewayPortStatus } from "../types";
import { formatTokens, formatWhen } from "./session-display";
import { formatCost, formatDuration, formatTokenCount, rangeBounds } from "./gateway-range";
import { gatewayPortLabel, gatewayTone } from "./gateway-address";
import { useAsyncData } from "../components/use-async-data";

/**
 * 侧栏工作台：**当前会话与 Gateway 的运行概览**。
 *
 * # 它与旧平台那个的差别（必须说清）
 *
 * 旧平台是一个**独立的一级视图**（`SidebarWorkbench`），点「工作台」把整个侧栏
 * 换成它。这里用的是**同一块侧栏内容**，但在信息上对齐旧平台的四块：
 *
 * | 旧平台 | 这里 | 数据来源 |
 * | --- | --- | --- |
 * | 运行上下文（目标 / 供应商 / 会话 / 用量） | 同左 | 已有的选择状态 + 会话自带用量 |
 * | 运行监控：最近路由变更 | ✅ | 终端的供应商路由（`useTerminals` 已经拿到） |
 * | 运行监控：速率限制 | ✅ | `session.usage.rateLimits`（会话摘要自带，不额外请求） |
 * | 运行监控：今日 Gateway 用量 | ✅ | `gateway:get-usage-summary` |
 * | 运行监控：最近 Gateway 异常 | ✅ | `gateway:get-failure-diagnostics`（取第 1 页） |
 *
 * # 为什么不额外轮询
 *
 * 旧平台的注释写得很明确：「工作台不为获取限额额外请求 CLI 或 Gateway」。
 * 这里的速率限制同样**只读会话摘要里已有的字段**，用量与异常则在
 * 「后端推送 `gateway:request-recorded` 时」刷新（由外层订阅，见 `app.tsx`）。
 */
export function SidebarWorkbench({
  provider,
  target,
  session,
  gateway,
  vendors,
  lastSwitch,
  refreshToken
}: {
  provider: AiProviderSummary | undefined;
  target: AiTarget | null;
  session: AiSession | null;
  /** 网关状态。用**真实类型**而不是手抄的子集，理由见 `status-bar.tsx` 同一处注释。 */
  gateway: GatewayPortStatus | null;
  vendors: ApiVendor[];
  /** 最近一次供应商路由变更（来自终端状态）。 */
  lastSwitch?: { vendorId: string; reason: string; at: string };
  /**
   * 变化时重新取数。
   *
   * 用一个**计数器**而不是把「刷新函数」传下来：外层只要在收到后端事件时
   * `setRefreshToken((n) => n + 1)` 即可，不需要知道工作台内部取了哪些接口。
   */
  refreshToken: number;
}) {
  const [manual, setManual] = useState(0);
  /**
   * 取数键。
   *
   * `refreshToken`（外部事件驱动）与 `manual`（用户点刷新）拼在一起作为
   * `useAsyncData` 的依赖——两者走的是**同一条**取数路径，不存在
   * 「事件刷新和手动刷新行为不一样」这种偏差。
   */
  const loadKey = `${refreshToken}:${manual}`;

  const load = useCallback(async () => {
    const { start, end } = rangeBounds("today");
    const [usage, failures] = await Promise.all([
      api.getUsageSummary(start, end).catch(() => null),
      api
        .getFailureDiagnostics({ page: 1, pageSize: 5 })
        .then((value) => value.items)
        .catch(() => [])
    ]);
    return { usage, failures };
    // `loadKey` 只用来让这个回调在键变化时换一个身份——它本身不参与取数。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loadKey]);

  const data = useAsyncData(load, { errorMessage: "工作台读取失败" });
  const usage = data.data?.usage ?? null;
  const failures = data.data?.failures ?? [];

  const rateWindows = getRateLimitWindows(session);
  const activeVendor = lastSwitch
    ? (vendors.find((item) => item.id === lastSwitch.vendorId)?.name ?? lastSwitch.vendorId)
    : "";

  return (
    <>
      <Card
        title="运行上下文"
        description="当前会话与路由的即时状态。"
        actions={
          <Button small onClick={() => setManual((n) => n + 1)}>
            刷新
          </Button>
        }
      >
        <DescriptionList
          items={[
            { label: "平台", value: provider?.label ?? "未选择" },
            { label: "目标", value: target?.label ?? "未选择" },
            {
              label: "会话",
              value: sessionLabel(session)
            },
            { label: "模型", value: session?.model ?? "-" },
            {
              label: "累计 token",
              value: formatTokens(session?.usage?.total?.totalTokens ?? 0),
              tone: session?.usage?.total?.totalTokens ? "ok" : undefined
            },
            {
              label: "上下文占用",
              value:
                session?.usage?.contextPercent !== undefined
                  ? `${session.usage.contextPercent}%`
                  : "-"
            },
            {
              label: "网关",
              // 与状态栏共用同一个判断：两边各写一遍时，一处算错只有一格显示不对。
              value: gatewayPortLabel(gateway),
              tone: gatewayTone(gateway) === "ok" ? "ok" : undefined
            }
          ]}
        />
      </Card>

      {rateWindows.length > 0 && (
        <Card
          title="速率限制"
          description="只在会话摘要自带这个信息时显示——工作台不为它额外请求 CLI。"
        >
          <DescriptionList
            items={rateWindows.map((item) => ({
              label: item.label,
              value: `${item.usedPercent ?? "-"}%${
                item.resetsAt ? ` · ${formatWhen(item.resetsAt)} 重置` : ""
              }`,
              tone:
                item.usedPercent !== undefined && item.usedPercent >= 80
                  ? ("warn" as const)
                  : undefined
            }))}
          />
        </Card>
      )}

      {lastSwitch && (
        <Card title="最近路由变更" description={describeSwitchReason(lastSwitch.reason)}>
          <DescriptionList
            items={[
              { label: "供应商", value: activeVendor || lastSwitch.vendorId },
              { label: "时间", value: formatWhen(lastSwitch.at) || "-" }
            ]}
          />
        </Card>
      )}

      <Card title="今日 Gateway 用量" description="今天零点到现在。">
        {data.loading ? (
          <LoadingBlock label="正在汇总今日用量…" />
        ) : !usage ? (
          <EmptyState
            message="今日还没有网关请求。"
            hint="终端接入网关之后，这里会显示请求数与耗时。"
          />
        ) : (
          <DescriptionList
            items={[
              { label: "请求", value: usage.requestCount },
              { label: "成功", value: usage.successCount, tone: "ok" },
              {
                label: "失败",
                value: usage.failureCount,
                tone: usage.failureCount > 0 ? "warn" : undefined
              },
              { label: "故障切换", value: usage.switchedCount },
              { label: "平均耗时", value: formatDuration(usage.averageDurationMs) },
              { label: "Token / 费用", value: `${formatTokenCount(usage.totalTokens)} / ${formatCost(usage.costUsd)}` }
            ]}
          />
        )}
      </Card>

      <Card
        title="最近 Gateway 异常"
        description="最近 5 条。要看全部、按条件筛，去「配置 → 异常」。"
        actions={
          data.loading && (
            <Toolbar>
              <ToolbarSpacer />
            </Toolbar>
          )
        }
      >
        {data.error ? (
          <EmptyState message="异常读取失败。" hint={data.error} />
        ) : failures.length === 0 ? (
          <EmptyState message="没有异常记录。" />
        ) : (
          <ul className="workbench-failures">
            {failures.map((item, index) => (
              <li key={`${item.createdAt}:${item.vendorId}:${index}`}>
                <strong>{item.errorCode ?? `HTTP ${item.upstreamStatus ?? "-"}`}</strong>
                <small>
                  {formatWhen(item.createdAt)} · {item.vendorId} · {formatDuration(item.durationMs)}
                </small>
              </li>
            ))}
          </ul>
        )}
      </Card>
    </>
  );
}

/**
 * 会话那一格。
 *
 * 编号可能缺失（工作台只拿到一个带 `usage` 的片段就够了），因此**逐层兜底**：
 * 标题 → 编号前 8 位 → 「未打开」。直接 `session.id.slice(...)` 会在没有 `id` 时
 * 抛异常——整个工作台跟着白屏，而它本来只是想显示一行字。
 */
export function sessionLabel(session: AiSession | null | undefined): string {
  if (!session) return "未打开";
  const title = session.title?.trim();
  if (title) return title;
  const id = session.id ?? "";
  return id ? id.slice(0, 8) : "未命名会话";
}

/** 一个速率的窗口。取值与旧平台一致：主窗口 / 次窗口。 */

export function getRateLimitWindows(
  session: AiSession | null | undefined
): { label: string; usedPercent?: number; resetsAt?: string; windowMinutes?: number }[] {
  const limits = session?.usage?.rateLimits;
  if (!limits) return [];
  const candidates = [
    limits.primary ? { label: "主窗口", ...limits.primary } : undefined,
    limits.secondary ? { label: "次窗口", ...limits.secondary } : undefined
  ];
  return candidates.filter(
    (item): item is { label: string; usedPercent?: number; resetsAt?: string; windowMinutes?: number } =>
      Boolean(item)
  );
}

/**
 * 路由变更的原因。
 *
 * 三种原因**对用户的意义完全不同**：手动是他自己点的，故障切换是网关替他做的
 * （他什么都没做），候选池调整是配置变了。混成一句「供应商已切换」会让人以为是自己点的。
 */
export function describeSwitchReason(reason: string): string {
  switch (reason) {
    case "manual":
      return "你手动切换的。";
    case "failure":
      return "上游失败后网关自动切换的——你没有做任何操作。";
    case "candidate-pool":
      return "候选池变化引起的调整。";
    default:
      return "路由发生了变化。";
  }
}
