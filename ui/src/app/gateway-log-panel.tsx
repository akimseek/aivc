import { useCallback, useState } from "react";

import {
  Button,
  Card,
  EmptyState,
  HintBar,
  LoadingBlock,
  Select,
  Toolbar,
  ToolbarSpacer,
  useAsyncData,
  useToast
} from "../components";
import * as api from "../ipc/commands";
import { PanelHeader } from "../features/settings/panel-header";
import type { GatewayLogEntry } from "../types";
import {
  customBounds,
  formatLogTime,
  outcomeLabel,
  rangeBounds,
  type UsageRangeKey
} from "./gateway-range";
import { GatewayRangePicker } from "./gateway-range-picker";

/**
 * Gateway 日志的**查询与清理**。
 *
 * # 与「最近请求」那张表的区别
 *
 * `GatewayInsights` 的「最近请求」是只读的一瞥；这里可以做三件那里做不到的事：
 * **按条件筛出要清的**、**逐条勾选**、**按编号删除**。
 *
 * # 「清空全部日志」为什么在这里
 *
 * 它原来是菜单栏「帮助 → 清理 Gateway 日志」的动作，菜单栏被移除后由这里承接
 * （spec §11）。一次全清**不可撤销**，因此必须问一次（`window.confirm`）——
 * 菜单栏那一条点了就直接执行，搬到面板里反而更容易被误点（面板是天天开的，
 * 菜单不是）。
 *
 * 后端 `gateway:query-logs` 与 `gateway:delete-log-entries` 一直在，界面此前没有接。
 */
export function GatewayLogPanel({
  onError,
  onNotify
}: {
  onError: (message: string) => void;
  /** 一句话成功反馈（清空之后要说一声，静默完成与没执行看起来一样）。 */
  onNotify?: (message: string) => void;
}) {
  const [picker, setPicker] = useState({ range: "7d" as UsageRangeKey, start: "", end: "" });
  const [outcome, setOutcome] = useState<"" | "ok" | "error" | "timeout" | "client-aborted">("");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const toast = useToast();

  // 自定义区间没填全时**不发请求**：编一个区间去查会掩盖「还没填完」这件事。
  const custom = picker.range === "custom" ? customBounds(picker.start, picker.end) : null;
  const preset = picker.range === "custom" ? null : rangeBounds(picker.range);
  const start = custom?.ok ? custom.start : (preset?.start ?? "");
  const end = custom?.ok ? custom.end : (preset?.end ?? "");

  const load = useCallback(
    () =>
      start && end
        ? api.queryGatewayLogs({
            ...(outcome ? { outcome } : {}),
            periodStart: start,
            periodEnd: end
          })
        : Promise.resolve([]),
    [outcome, start, end]
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

  /**
   * 清空**全部**日志。
   *
   * 与「删除选中」是两件事：那个只动静筛选出来的那几条，这个把整张审计表清空。
   * 不可撤销，因此先弹一次原生确认——界面上的 toast/hint 都不适合承担
   * 「点错了就没了」的确认，它必须挡住这一步。
   */
  const clearAll = useCallback(async () => {
    if (!window.confirm("确定清空所有网关审计日志吗？")) return;
    try {
      const removed = await api.clearGatewayLogs();
      setSelected(new Set());
      onNotify?.(`已清理 ${removed} 条 Gateway 日志。`);
      await logs.reload();
    } catch (problem) {
      onError(problem instanceof Error ? problem.message : String(problem));
    }
  }, [logs, onNotify, onError]);

  return (
    <div className="observability-panel">
      {/* 删除/清空这类一次性的结果：说完自己消失，也可以立刻收掉。 */}
      {toast.message && <HintBar message={toast.message} onDismiss={toast.dismiss} />}

      {/* 面板头统一：标题、说明与「重新查询」从卡片头搬到这里（卡片本体只管内容）。 */}
      <PanelHeader
        title="请求日志"
        description="按时间区间与结果筛选。逐条勾选后可以只删掉选中的那些。"
        actions={
          <Button small onClick={() => void logs.reload()}>
            重新查询
          </Button>
        }
      />
      <Card>
        <Toolbar>
          <GatewayRangePicker
            range={picker.range}
            start={picker.start}
            end={picker.end}
            onChange={(next) => {
              // 换范围就清空勾选：跨区间保留勾选会让「删除选中」删掉没显示的那几条。
              setSelected(new Set());
              setPicker(next);
            }}
          />

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
          {/* 整表清空。承接被移除的菜单栏「清理 Gateway 日志」（spec §11）。
              与「删除选中」同一条工具条、同一个尺寸与危险色：两个动作的差别只在
              范围（选中的 vs 全部），按钮本身长得不一样会让人以为它们差得更远。 */}
          <Button small variant="danger" onClick={() => void clearAll()} title="清空全部审计日志（不可撤销）">
            清空全部日志
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
          /* `dense`：日志一行信息不多但会很长，行距收一档、单元格居中；
             时间与状态码走等宽数字，同一列才对得齐。 */
          <table className="data-table dense">
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
                  <td className="mono ui-num">{formatLogTime(entry.createdAt)}</td>
                  <td>{entry.providerId}</td>
                  <td>{entry.vendorId}</td>
                  <td>
                    <span className={`tag ${entry.outcome === "ok" ? "ok" : "off"}`}>
                      {outcomeLabel(entry.outcome)}
                    </span>
                  </td>
                  <td className="ui-num">{entry.upstreamStatus ?? "-"}</td>
                  <td className="mono" title={entry.requestId}>
                    {entry.errorCode ?? "-"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>
    </div>
  );
}
