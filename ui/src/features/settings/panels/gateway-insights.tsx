import { useCallback, useEffect, useRef, useState } from "react";
import { Badge, Button, Card, EmptyState, LoadingBlock, Pagination, errorText, useListPagination } from "../../../components";
import * as api from "../../../ipc/commands";
import type { ApiVendor, GatewayVendorHealth } from "../../../types";
import { vendorName } from "../../workbench/workbench-cards";
import { REQUEST_REFRESH_WINDOW_MS, createRefreshCoalescer } from "../../../app/refresh-coalescer";

/** 网关健康概览。用量和请求明细分别在独立的报表、日志页中查看。 */
export function GatewayInsights({ onError, onNotify }: {
  onError: (message: string) => void;
  onNotify?: (message: string) => void;
}) {
  const [health, setHealth] = useState<GatewayVendorHealth[]>([]);
  const [vendors, setVendors] = useState<ApiVendor[]>([]);
  const [loading, setLoading] = useState(true);
  const seqRef = useRef(0);
  const load = useCallback(async () => {
    const seq = ++seqRef.current;
    try {
      const [healthResult, vendorResult] = await Promise.all([api.getVendorHealth(), api.listVendors()]);
      if (seq !== seqRef.current) return;
      setHealth(healthResult);
      setVendors(vendorResult);
    } catch (error) {
      if (seq === seqRef.current) onError(errorText(error));
    } finally {
      if (seq === seqRef.current) setLoading(false);
    }
  }, [onError]);
  useEffect(() => {
    void load();
    return () => { ++seqRef.current; };
  }, [load]);
  useEffect(() => {
    const coalescer = createRefreshCoalescer(REQUEST_REFRESH_WINDOW_MS, () => { void load(); });
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void api.listen(api.CHANNEL_GATEWAY_REQUEST_RECORDED, () => coalescer.schedule()).then((fn) => {
      if (cancelled) fn(); else unlisten = fn;
    });
    return () => { cancelled = true; unlisten?.(); coalescer.cancel(); };
  }, [load]);
  const reset = useCallback(async (vendorId?: string) => {
    try {
      await api.resetVendorHealth(vendorId);
      onNotify?.(vendorId ? "供应商健康已重置。" : "全部供应商健康已重置。");
      await load();
    }
    catch (error) { onError(errorText(error)); }
  }, [load, onError, onNotify]);
  const pagination = useListPagination(health);
  return (
    <>
      <Card
        title="供应商健康"
        description="按上游尝试累计统计，失败率 = 失败尝试 / 总尝试，不是外部请求失败率。达到熔断阈值后暂时跳过供应商。"
        actions={
          <>
            <Button small onClick={() => void load()}>刷新</Button>
            {health.length > 0 && (
            <Button
              onClick={() => void reset()}
              title="清掉全部熔断状态，让被熔断的供应商立刻重新参与"
            >
              重置全部健康
            </Button>
          )}</>
        }
      >
        {loading ? <LoadingBlock label="正在读取供应商健康…" /> : health.length === 0 ? (
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
              {pagination.items.map((item) => (
                <tr key={`${item.providerId}:${item.vendorId}`}>
                  <td>{vendorName(vendors, item.vendorId)}</td>
                  <td>{item.providerId}</td>
                  <td>
                    <Badge small tone={item.status === "healthy" ? "accent" : "neutral"}>
                      {item.status}
                    </Badge>
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
        {health.length > 0 && (
          <Pagination {...pagination} label="供应商健康分页" />
        )}
      </Card>

    </>
  );
}
