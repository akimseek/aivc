import { useCallback, useEffect, useRef, useState } from "react";

import {
  Button,
  Card,
  ChipGroup,
  DescriptionList,
  EmptyState,
  Field,
  LoadingBlock,
  Toolbar,
  ToolbarSpacer
} from "../../../components";
import * as api from "../../../ipc/commands";
import type { GatewayLogEntry, GatewayVendorHealth, UsageSummary } from "../../../types";
import {
  USAGE_RANGES,
  formatCost,
  formatDuration,
  formatLogTime,
  formatTokenCount,
  outcomeLabel,
  rangeBounds,
  type UsageRangeKey
} from "../../../app/gateway-range";
import { REQUEST_REFRESH_WINDOW_MS, createRefreshCoalescer } from "../../../app/refresh-coalescer";

/**
 * 「最近请求」一次取多少条。
 *
 * 固定 50（后端仓储钳制的上限）：它是**概览**不是审计——一眼能扫完的量，
 * 一次请求画出整张表，不必为此引入翻页控件；审计要看更多时去日志页
 * （那里本来就有分页）。payload 因此有界：区间里有多少请求都只解析这一页。
 */
const RECENT_REQUESTS_PAGE_SIZE = 50;

/**
 * 网关的健康 / 用量 / 最近请求。
 *
 * 三块都只读，因此放在一个组件里统一加载——它们的刷新时机完全一样。
 *
 * # 用量必须带区间
 *
 * 后端 `period()` 在缺 `periodStart` / `periodEnd` 时**直接报错**。此前这里调
 * `getUsageSummary()` 一个参数都不传，所以用量那六格从来没能显示过；
 * 现在区间由界面选，默认「今天」。
 */
export function GatewayInsights({ onError }: { onError: (message: string) => void }) {
  const [range, setRange] = useState<UsageRangeKey>("today");
  const [health, setHealth] = useState<GatewayVendorHealth[]>([]);
  const [usage, setUsage] = useState<UsageSummary | null>(null);
  const [logs, setLogs] = useState<GatewayLogEntry[]>([]);
  const [loading, setLoading] = useState(true);

  /**
   * 在途请求的自增序号。
   *
   * 换区间时上一笔可能才回来：不做判定，旧区间的日志会把新区间的结果整表盖掉
   * （连点或刷新期间换区间都会）。与 `useAsyncData` / `use-session-detail` 是
   * **同一手法**；这里没套 `useAsyncData`，是因为三块数据各自容错、错误也各自
   * 展示，它「一份数据一个错误」的骨架套不上。
   */
  const seqRef = useRef(0);

  const load = useCallback(async () => {
    const seq = ++seqRef.current;
    const { start, end } = rangeBounds(range);
    // 三块**各自容错**：日志表为空或统计还没数据都是正常状态，
    // 不该让整块面板变成一句错误。
    const [healthResult, usageResult, logResult] = await Promise.all([
      api.getVendorHealth().catch(() => [] as GatewayVendorHealth[]),
      // 用量这一块单独把错误记下来：区间不对是**我们的**问题，
      // 把它和「健康读不到」混成一句「加载失败」会掩盖真正的原因。
      api
        .getUsageSummary(start, end)
        .then((value) => ({ value, error: "" }))
        .catch((problem: unknown) => ({
          value: null,
          error: problem instanceof Error ? problem.message : String(problem)
        })),
      // 最近请求只看最近一段，因此也带上区间——不然「最近请求」可能全是上个月的。
      // 取首页 50 条：概览要的是「最近发生了什么」，不是全部历史（见页长常量）。
      api
        .queryGatewayLogs({
          page: 1,
          pageSize: RECENT_REQUESTS_PAGE_SIZE,
          periodStart: start,
          periodEnd: end
        })
        .then((page) => page.items)
        .catch(() => [] as GatewayLogEntry[])
    ]);
    // 旧响应一律丢弃（**连收尾都不做**）：这一刻的状态属于更新的那次请求，
    // 失败与成功同理——旧请求的失败也不该报错、更不该清掉新结果。
    if (seq !== seqRef.current) return;
    setHealth(healthResult);
    setUsage(usageResult.value);
    setUsageError(usageResult.error);
    setLogs(logResult);
    setLoading(false);
  }, [range]);

  const [usageError, setUsageError] = useState("");

  useEffect(() => {
    void load().catch((error: unknown) =>
      onError(error instanceof Error ? error.message : String(error))
    );
  }, [load, onError]);

  /**
   * **后端每转发一个请求都会推一条事件，面板不该只在打开时取一次。**
   *
   * 刷新**必须合并且限流**：一个终端在重试循环里一秒能发好几条，而每次刷新要拉
   * 三个接口。合并器把自动刷新的频率压到 ≤1Hz（窗口常量 `REQUEST_REFRESH_WINDOW_MS`
   * 与工作台共用，见 `refresh-coalescer.ts`）；手动刷新不走这里，不受该窗口影响。
   */
  useEffect(() => {
    const coalescer = createRefreshCoalescer(REQUEST_REFRESH_WINDOW_MS, () => {
      void load().catch(() => undefined);
    });
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void api
      .listen(api.CHANNEL_GATEWAY_REQUEST_RECORDED, () => coalescer.schedule())
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      });
    return () => {
      cancelled = true;
      unlisten?.();
      coalescer.cancel();
    };
  }, [load]);

  /** 重置健康：**数据库与内存两份都要清**，所以必须走这条命令而不是自己清本地状态。 */
  const reset = useCallback(
    async (vendorId?: string) => {
      try {
        await api.resetVendorHealth(vendorId);
        await load();
      } catch (error: unknown) {
        onError(error instanceof Error ? error.message : String(error));
      }
    },
    [load, onError]
  );

  return (
    <>
      <Card
        title="用量"
        description="按所选时间区间统计。今天的起点是本机零点。"
        actions={
          <ChipGroup
            value={range}
            onChange={(value) => setRange(value as UsageRangeKey)}
            options={USAGE_RANGES.map((item) => ({ value: item.key, label: item.label }))}
          />
        }
      >
        {usageError ? (
          <EmptyState message="用量读取失败。" hint={usageError} />
        ) : usage ? (
          <DescriptionList
            items={[
              { label: "请求数", value: usage.requestCount },
              { label: "成功", value: usage.successCount, tone: "ok" },
              {
                label: "失败",
                value: usage.failureCount,
                tone: usage.failureCount > 0 ? "warn" : undefined
              },
              { label: "故障切换", value: usage.switchedCount },
              { label: "重试", value: usage.retryCount ?? "-" },
              { label: "平均耗时", value: formatDuration(usage.averageDurationMs) },
              { label: "输入 token", value: formatTokenCount(usage.inputTokens) },
              { label: "输出 token", value: formatTokenCount(usage.outputTokens) },
              { label: "总 token", value: formatTokenCount(usage.totalTokens) },
              { label: "费用", value: formatCost(usage.costUsd) }
            ]}
          />
        ) : (
          <LoadingBlock label="正在读取用量…" />
        )}
      </Card>

      <Card
        title="供应商健康"
        description="熔断状态在内存里，因此重置必须走后端命令，不能只清界面。"
        actions={
          health.length > 0 && (
            <Button
              onClick={() => void reset()}
              title="清掉全部熔断状态，让被熔断的供应商立刻重新参与"
            >
              重置全部健康
            </Button>
          )
        }
      >
        {health.length === 0 ? (
          <EmptyState message="还没有请求记录，因此没有健康数据。" />
        ) : (
          <table className="data-table">
            <thead>
              <tr>
                <th>供应商</th>
                <th>平台</th>
                <th>状态</th>
                <th>成功 / 失败</th>
                <th>失败率</th>
                <th>最近失败原因</th>
              </tr>
            </thead>
            <tbody>
              {health.map((item) => (
                <tr key={item.vendorId}>
                  <td>{item.vendorId}</td>
                  <td>{item.providerId}</td>
                  <td>
                    <span className={`tag ${item.status === "healthy" ? "ok" : "off"}`}>
                      {item.status}
                    </span>
                  </td>
                  <td>
                    {item.successCount} / {item.failureCount}
                  </td>
                  <td>{Math.round(item.failureRate * 100)}%</td>
                  <td className="mono">{item.lastFailureReason ?? "-"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>

      <Card
        title="最近请求"
        description="按上面的时间区间筛选。"
        actions={
          <Toolbar>
            <Button small onClick={() => void load()}>
              刷新
            </Button>
            <ToolbarSpacer />
            {/* 清空只在**这里**做（按区间清是另一个面板的事）：一次全清是不可撤销的，
                所以它单独放在这一个位置，并标成危险动作。 */}
            <Button
              small
              variant="danger"
              onClick={() =>
                void api
                  .clearGatewayLogs()
                  .then(load)
                  .catch((error: unknown) =>
                    onError(error instanceof Error ? error.message : String(error))
                  )
              }
            >
              清空日志
            </Button>
          </Toolbar>
        }
      >
        {loading && logs.length === 0 ? (
          <LoadingBlock label="正在读取请求日志…" />
        ) : logs.length === 0 ? (
          <EmptyState
            message="这个区间内没有请求日志。"
            hint="换个时间区间看看，或确认网关是否在服务请求。"
          />
        ) : (
          <table className="data-table">
            <thead>
              <tr>
                <th>时间</th>
                <th>供应商</th>
                <th>平台</th>
                <th>结果</th>
                <th>上游状态</th>
                <th>错误代码</th>
              </tr>
            </thead>
            <tbody>
              {logs.map((entry) => (
                <tr key={entry.requestId}>
                  <td className="mono">{formatLogTime(entry.createdAt)}</td>
                  <td>{entry.vendorId}</td>
                  <td>{entry.providerId}</td>
                  <td>
                    <span className={`tag ${entry.outcome === "ok" ? "ok" : "off"}`}>
                      {outcomeLabel(entry.outcome)}
                    </span>
                  </td>
                  <td>{entry.upstreamStatus ?? "-"}</td>
                  {/* 这一列非有不可：「路由不存在 / 鉴权失败」记的是 `ok`（那不是供应商的错），
                      于是它与一次真正成功的记录只差这里。 */}
                  <td className="mono">{entry.errorCode ?? "-"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>

      <Field label="说明">
        <span className="value">
          「成功」只表示请求被处理完；是否真的转发到上游要看<strong>错误代码</strong>那一列。
        </span>
      </Field>
    </>
  );
}
