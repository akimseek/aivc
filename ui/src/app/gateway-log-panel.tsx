import { useCallback, useState } from "react";

import {
  Button,
  Card,
  EmptyState,
  LoadingBlock,
  Select,
  Toolbar,
  ToolbarSpacer,
  useAsyncData,
  useToast
} from "../components";
import * as api from "../ipc/commands";
import type { GatewayLogEntry } from "../types";
import { formatLogTime, outcomeLabel, rangeBounds, USAGE_RANGES, type UsageRangeKey } from "./gateway-range";

/**
 * Gateway 日志的**查询与清理**。
 *
 * # 与「最近请求」那张表的区别
 *
 * `GatewayInsights` 的「最近请求」是只读的一瞥；这里可以做三件那里做不到的事：
 * **按条件筛出要清的**、**逐条勾选**、**按编号删除**。
 *
 * 「清空日志」是一个不可撤销的全清动作，因此它**不在这里**——它单独留在
 * `GatewayInsights` 那一处，并且标成危险动作。把「按条件清」和「全清」放在一起，
 * 迟早会有人点错。
 *
 * 后端 `gateway:query-logs` 与 `gateway:delete-log-entries` 一直在，界面此前没有接。
 */
export function GatewayLogPanel({ onError }: { onError: (message: string) => void }) {
  const [range, setRange] = useState<UsageRangeKey>("7d");
  const [outcome, setOutcome] = useState<"" | "ok" | "error" | "timeout" | "client-aborted">("");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const toast = useToast();

  const bounds = rangeBounds(range);

  const load = useCallback(
    () =>
      api.queryGatewayLogs({
        ...(outcome ? { outcome } : {}),
        periodStart: bounds.start,
        periodEnd: bounds.end
      }),
    [outcome, bounds.start, bounds.end]
  );

  const logs = useAsyncData(load, { errorMessage: "日志查询失败" });
  const entries: GatewayLogEntry[] = logs.data ?? [];
  const allSelected = entries.length > 0 && selected.size === entries.length;

  const toggle = (requestId: string) => {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(requestId)) next.delete(requestId);
      else next.add(requestId);
      return next;
    });
  };

  const toggleAll = () => {
    setSelected(allSelected ? new Set() : new Set(entries.map((item) => item.requestId)));
  };

  /** 删掉勾选的条目。**删除后再查一次**，而不是在本地把它们抹掉——
   *  本地抹掉会掩盖「其实没删成功」这种情况。 */
  const removeSelected = useCallback(async () => {
    if (selected.size === 0) return;
    try {
      const removed = await api.deleteGatewayLogEntries([...selected]);
      setSelected(new Set());
      toast.show(`已删除 ${removed} 条日志。`);
      await logs.reload();
    } catch (problem) {
      onError(problem instanceof Error ? problem.message : String(problem));
    }
  }, [selected, logs, toast, onError]);

  return (
    <>
      {toast.message && (
        <div className="export-hint" role="status">
          {toast.message}
          <Button small onClick={toast.dismiss}>
            知道了
          </Button>
        </div>
      )}

      <Card
        title="Gateway 日志"
        description="按时间区间与结果筛选。逐条勾选后可以只删掉选中的那些。"
        actions={
          <Button small onClick={() => void logs.reload()}>
            重新查询
          </Button>
        }
      >
        <Toolbar>
          <label className="field">
            <span className="field-label">查询范围</span>
            <Select
              value={range}
              onChange={(event) => {
                setSelected(new Set());
                setRange(event.target.value as UsageRangeKey);
              }}
            >
              {USAGE_RANGES.filter((item) => item.key !== "custom").map((item) => (
                <option key={item.key} value={item.key}>
                  {item.label}
                </option>
              ))}
            </Select>
          </label>

          <label className="field">
            <span className="field-label">结果类型</span>
            <Select
              value={outcome}
              onChange={(event) => {
                setSelected(new Set());
                setOutcome(event.target.value as typeof outcome);
              }}
            >
              <option value="">全部结果</option>
              <option value="ok">成功</option>
              <option value="error">失败</option>
              <option value="timeout">超时</option>
              <option value="client-aborted">已取消</option>
            </Select>
          </label>

          <ToolbarSpacer />

          <Button small onClick={toggleAll} disabled={entries.length === 0}>
            {allSelected ? "取消全选" : "全选当前页"}
          </Button>
          <Button
            small
            variant="danger"
            disabled={selected.size === 0}
            onClick={() => void removeSelected()}
          >
            删除选中（{selected.size}）
          </Button>
        </Toolbar>
      </Card>

      <Card title={`结果（${entries.length} 条）`}>
        {logs.loading ? (
          <LoadingBlock label="正在查询日志…" />
        ) : logs.error ? (
          <EmptyState message="日志查询失败。" hint={logs.error} />
        ) : entries.length === 0 ? (
          <EmptyState
            message="这个条件下没有日志。"
            hint="换个时间区间或结果类型看看。"
          />
        ) : (
          <table className="data-table">
            <thead>
              <tr>
                <th>选择</th>
                <th>时间</th>
                <th>平台</th>
                <th>供应商</th>
                <th>结果</th>
                <th>上游状态</th>
                <th>错误代码</th>
              </tr>
            </thead>
            <tbody>
              {entries.map((entry) => (
                <tr key={entry.requestId}>
                  <td>
                    <input
                      type="checkbox"
                      checked={selected.has(entry.requestId)}
                      aria-label={`选择 ${entry.requestId}`}
                      onChange={() => toggle(entry.requestId)}
                    />
                  </td>
                  <td className="mono">{formatLogTime(entry.createdAt)}</td>
                  <td>{entry.providerId}</td>
                  <td>{entry.vendorId}</td>
                  <td>
                    <span className={`tag ${entry.outcome === "ok" ? "ok" : "off"}`}>
                      {outcomeLabel(entry.outcome)}
                    </span>
                  </td>
                  <td>{entry.upstreamStatus ?? "-"}</td>
                  <td className="mono" title={entry.requestId}>
                    {entry.errorCode ?? "-"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>
    </>
  );
}
