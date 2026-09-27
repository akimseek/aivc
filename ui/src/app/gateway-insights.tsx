import { useCallback, useEffect, useState } from "react";

import { Button, Field, Toolbar, ToolbarSpacer } from "../components";
import * as api from "../ipc/commands";
import type { GatewayLogEntry, GatewayVendorHealth, UsageSummary } from "../types";
import { createRefreshCoalescer } from "./refresh-coalescer";

/**
 * 请求事件的合并窗口。
 *
 * 后端每转发一个请求就推一条，而刷新一次要拉三个接口（健康、用量、日志）。
 * 200ms 足够让人看不出延迟，又能把一次重试循环里的好几条并成一次。
 */
const REQUEST_REFRESH_WINDOW_MS = 200;

/**
 * 网关的健康 / 用量 / 最近请求。
 *
 * 三块都只读，因此放在一个组件里统一加载——它们的刷新时机完全一样。
 *
 * # 为什么从 `config-overlay.tsx` 里搬出来
 *
 * 那个文件已经 1380 行、里面 10 个面板。这个组件又恰好是**唯一有跨组件接线**的一个
 * （订阅事件、合并刷新、调命令），留在大文件里就只能靠肉眼看界面验证。
 * 搬出来之后它可以被单独渲染、单独断言。
 */
export function GatewayInsights({ onError }: { onError: (message: string) => void }) {
  const [health, setHealth] = useState<GatewayVendorHealth[]>([]);
  const [usage, setUsage] = useState<UsageSummary | null>(null);
  const [logs, setLogs] = useState<GatewayLogEntry[]>([]);

  const load = useCallback(async () => {
    // 三块**各自容错**：日志表为空或统计还没数据都是正常状态，
    // 不该让整块面板变成一句错误。
    const [healthResult, usageResult, logResult] = await Promise.all([
      api.getVendorHealth().catch(() => [] as GatewayVendorHealth[]),
      api.getUsageSummary().catch(() => null),
      api.queryGatewayLogs(1, 20).catch(() => [] as GatewayLogEntry[])
    ]);
    setHealth(healthResult);
    setUsage(usageResult);
    setLogs(logResult);
  }, []);

  useEffect(() => {
    void load().catch((error: unknown) =>
      onError(error instanceof Error ? error.message : String(error))
    );
  }, [load, onError]);

  /**
   * **后端每转发一个请求都会推一条事件，面板不该只在打开时取一次。**
   *
   * 这段接线此前完全缺失：后端从第一次实现就在发 `gateway:request-recorded`，
   * 而界面一处都没订阅——于是「实时」那一半从来没成立，用户只能关掉面板再打开。
   *
   * 刷新**必须合并**：一个终端在重试循环里一秒能发好几条，而每次刷新要拉三个接口。
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
      {usage && (
        <div className="field-grid">
          <Field label="请求数">
            <span className="value">{usage.requestCount}</span>
          </Field>
          <Field label="失败">
            <span className="value">{usage.failureCount}</span>
          </Field>
          <Field label="故障切换">
            <span className="value">{usage.switchedCount}</span>
          </Field>
          <Field label="平均耗时">
            <span className="value">
              {usage.averageDurationMs !== undefined
                ? `${Math.round(usage.averageDurationMs)} ms`
                : "-"}
            </span>
          </Field>
          <Field label="总 token">
            <span className="value">{usage.totalTokens ?? "-"}</span>
          </Field>
          <Field label="费用">
            <span className="value">
              {usage.costUsd !== undefined ? `$${usage.costUsd.toFixed(4)}` : "-"}
            </span>
          </Field>
        </div>
      )}

      <h3 className="section-title">供应商健康</h3>
      {/* 重置入口。
          这条命令此前在界面上**没有入口**——而后端那半直到最近才修好：
          它过去只清数据库那一份，而真正生效的熔断在内存里，按下去行为完全不变。
          现在两份一起清，所以这里必须走命令，不能只清本地状态。 */}
      {health.length > 0 && (
        <Toolbar>
          <Button
            onClick={() => void reset()}
            title="清掉全部熔断状态，让被熔断的供应商立刻重新参与"
          >
            重置全部健康
          </Button>
        </Toolbar>
      )}
      {health.length === 0 ? (
        <div className="group-empty">还没有请求记录，因此没有健康数据。</div>
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

      <h3 className="section-title">最近请求</h3>
      <Toolbar>
        <Button small onClick={() => void load()}>
          刷新
        </Button>
        <ToolbarSpacer />
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
      {logs.length === 0 ? (
        <div className="group-empty">还没有请求日志。</div>
      ) : (
        <table className="data-table">
          <thead>
            <tr>
              <th>时间</th>
              <th>供应商</th>
              <th>结果</th>
              <th>上游状态</th>
            </tr>
          </thead>
          <tbody>
            {logs.map((entry) => (
              <tr key={entry.requestId}>
                <td className="mono">{entry.createdAt}</td>
                <td>{entry.vendorId}</td>
                <td>
                  <span className={`tag ${entry.outcome === "ok" ? "ok" : "off"}`}>
                    {entry.outcome}
                  </span>
                </td>
                <td>{entry.upstreamStatus ?? "-"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </>
  );
}
