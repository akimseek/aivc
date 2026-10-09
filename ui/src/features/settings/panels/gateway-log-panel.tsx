import { useCallback, useEffect, useState } from "react";

import {
  Badge,
  Button,
  Card,
  EmptyState,
  LoadingBlock,
  Pagination,
  Select,
  Toolbar,
  ToolbarSpacer,
  errorText,
  useAsyncData
} from "../../../components";
import * as api from "../../../ipc/commands";
import { PanelHeader } from "../panel-header";
import type { GatewayLogEntry, GatewayLogsPage } from "../../../types";
import { formatLogTime, outcomeLabel, outcomeTone } from "../../../app/gateway-format";
import { customBounds, rangeBounds, type UsageRangeKey } from "../../../app/gateway-range";
import { GatewayRangePicker } from "./gateway-range-picker";
import { vendorName } from "../../workbench/workbench-cards";
import { REQUEST_REFRESH_WINDOW_MS, createRefreshCoalescer } from "../../../app/refresh-coalescer";

/** 默认每页条数（可在分页栏切换，与异常页一致）。 */
const PAGE_SIZE = 20;

/**
 * Gateway 日志的**查询与清理**。
 *
 * 每个外部 HTTP 请求保存一条独立记录，供应商重试和切换保存在该记录的尝试明细中。
 *
 * # 分页在**后端**
 *
 * 批次二裁定过不做客户端分页：日志只会越攒越多，前端把全表拉回来再切页，
 * 「翻到第 5 页」的开销与「只看一屏」一样大。现在 `gateway:query-logs` 支持
 * `page`/`pageSize`（**带上这两个参数**时返回 `{ items, total, page, pageSize }`，
 * 与异常诊断页逐字同形），这一页因此也同形：默认 20 条一页，使用公共分页栏。
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
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(PAGE_SIZE);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const vendors = useAsyncData(api.listVendors, { errorMessage: "供应商列表读取失败", onError });

  /**
   * 筛选条件变化时**回到第 1 页**，并清空勾选。
   *
   * 不回到第 1 页会出现一个很难解释的现象：在第 5 页筛出 3 条结果，界面显示空白，
   * 用户以为「筛选没生效」，其实是第 5 页不存在（与异常页同一条理由）。
   * 清空勾选则是因为：跨条件保留勾选会让「删除选中」删掉现在根本没显示的那几条。
   */
  const applyFilter = useCallback((action: () => void) => {
    action();
    setSelected(new Set());
    setPage(1);
  }, []);

  /**
   * 翻页同样清空勾选，理由与换筛选一样：**勾选只对当前这一页有效**。
   *
   * 跨页累计的勾选在界面上看不出来（后面几页勾了什么，只有翻回去才知道），
   * 而「删除选中」会照着它删——这是最容易出事的一类操作。
   */
  const gotoPage = useCallback((next: number) => {
    setSelected(new Set());
    setPage(next);
  }, []);

  /**
   * 区间在 `load` **内部**现算（与用量、异常两页同一处修法）。
   *
   * `load` 的身份就是「要不要重查」的信号，只能跟着**查询输入**（结果类型 + 页码 +
   * `picker` 三个字段）走；预设区间的右端是「此刻」，渲染期算出来每次都是新字符串，
   * 进了依赖会拖成每渲染重查一次的死循环。
   *
   * 自定义区间没填全时**不发请求**：编一个区间去查会掩盖「还没填完」这件事。
   * 给一个空页而不是什么都不给，是因为下游按「页」渲染。
   */
  const load = useCallback(() => {
    const bounds =
      picker.range === "custom"
        ? customBounds(picker.start, picker.end)
        : { ok: true as const, ...rangeBounds(picker.range) };
    if (!bounds.ok) {
      return Promise.resolve<GatewayLogsPage>({
        items: [],
        total: 0,
        page: 1,
        pageSize
      });
    }
    return api.queryGatewayLogs({
      page,
      pageSize,
      ...(outcome ? { outcome } : {}),
      periodStart: bounds.start,
      periodEnd: bounds.end
    });
  }, [page, pageSize, outcome, picker.range, picker.start, picker.end]);

  const logs = useAsyncData(load, { errorMessage: "日志查询失败", onError });
  const reloadLogs = logs.reload;
  useEffect(() => {
    const coalescer = createRefreshCoalescer(REQUEST_REFRESH_WINDOW_MS, () => {
      setSelected(new Set());
      void reloadLogs();
    });
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void api.listen(api.CHANNEL_GATEWAY_REQUEST_RECORDED, () => coalescer.schedule()).then((fn) => {
      if (cancelled) fn(); else unlisten = fn;
    });
    return () => { cancelled = true; unlisten?.(); coalescer.cancel(); };
  }, [reloadLogs]);
  const items: GatewayLogEntry[] = logs.data?.items ?? [];
  const total = logs.data?.total ?? 0;
  const pageCount = Math.max(1, Math.ceil(total / pageSize));
  useEffect(() => {
    // 只依据当前查询的响应校正页码，不能用上一页尚未刷新的 total。
    if (!logs.loading && logs.data?.page === page && logs.data.pageSize === pageSize && page > pageCount) {
      gotoPage(pageCount);
    }
  }, [logs.loading, logs.data, page, pageSize, pageCount, gotoPage]);
  // 勾选与这一页的条目对齐：翻页与换筛选都会清空，因此这里比的就是当前页。
  const allSelected = items.length > 0 && selected.size === items.length;

  const toggle = (requestId: string) => {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(requestId)) next.delete(requestId);
      else next.add(requestId);
      return next;
    });
  };

  const toggleAll = () => {
    setSelected(allSelected ? new Set() : new Set(items.map((item) => item.requestId)));
  };

  /** 删掉勾选的条目。**删除后再查一次**，而不是在本地把它们抹掉——
   *  本地抹掉会掩盖「其实没删成功」这种情况。 */
  const removeSelected = useCallback(async () => {
    if (selected.size === 0) return;
    // 先把「这一页是否被整页删光」记下来：下面据此决定要不要退回上一页。
    const emptiedThePage = items.length === selected.size;
    try {
      const removed = await api.deleteGatewayLogEntries([...selected]);
      setSelected(new Set());
      onNotify?.(`已删除 ${removed} 条日志。`);
      // 整页删光还留在原页，只会看到一张空表，看起来像「删多了」；
      // 退回上一页才是这个动作自然的结果（返回上一页本身会触发重查）。
      if (emptiedThePage && page > 1) setPage(page - 1);
      else await logs.reload();
    } catch (problem) {
      onError(errorText(problem));
    }
  }, [selected, items.length, page, logs, onNotify, onError]);

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
      // 全清之后留在第 5 页只会看到一张空表；回到第 1 页才是这份结果该在的位置。
      if (page !== 1) setPage(1);
      else await logs.reload();
    } catch (problem) {
      onError(errorText(problem));
    }
  }, [page, logs, onNotify, onError]);

  return (
    <div className="observability-panel">
      {/* 面板头统一：标题、说明与「重新查询」从卡片头搬到这里（卡片本体只管内容）。 */}
      <PanelHeader
        title="请求日志"
        description="每个 HTTP 请求独立记录，不按对话轮次合并。展开明细可查看全部供应商尝试；请求数与用量按原请求统计。"
        actions={
          <Button
            small
            onClick={() => {
              // 手动重查也清空勾选（留在当前页，只清勾选）：刷新后行会位移，
              // 留在集合里的 id 会让「删除选中（N）」数到屏幕上看不见的条目。
              // 旧平台每次刷新（含手动）都清空，理由相同。
              setSelected(new Set());
              void logs.reload();
            }}
          >
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
            onChange={(next) => applyFilter(() => setPicker(next))}
          />

          <label className="field">
            <span className="field-label">结果类型</span>
            <Select
              value={outcome}
              onChange={(event) =>
                applyFilter(() => setOutcome(event.target.value as typeof outcome))
              }
            >
              <option value="">全部结果</option>
              <option value="ok">成功</option>
              <option value="error">失败</option>
              <option value="timeout">超时</option>
              <option value="client-aborted">已取消</option>
            </Select>
          </label>

          <ToolbarSpacer />

          {/*
            「全选当前页」就是字面意思：勾上**这一页**的条目。
            翻页会清空勾选（见 `gotoPage`），因此不存在「跨页累计」这种状态。
          */}
          <Button small onClick={toggleAll} disabled={items.length === 0}>
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

      {/* 标题里报的是**匹配总数**（不是本页条数）：分页之后两者会不一样，
          而用户要的是「这个条件下到底有多少条」。 */}
      <Card title={`结果（共 ${total} 条）`}>
        {logs.loading ? (
          <LoadingBlock label="正在查询日志…" />
        ) : logs.error ? (
          <EmptyState message="日志查询失败。" hint={logs.error} />
        ) : items.length === 0 ? (
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
                <th>重试 / 切换</th>
                <th>诊断与尝试明细</th>
              </tr>
            </thead>
            <tbody>
              {items.map((entry) => (
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
                  <td>{vendorName(vendors.data ?? [], entry.vendorId)}</td>
                  <td>
                    <Badge small tone={outcomeTone(entry.outcome)}>
                      {outcomeLabel(entry.outcome)}
                    </Badge>
                  </td>
                  <td className="ui-num">{entry.upstreamStatus ?? "-"}</td>
                  <td>{entry.retryCount ?? 0} 次{entry.switched ? " · 已切换" : ""}</td>
                  <td>
                    <details>
                      <summary>{entry.errorCode ?? "请求详情"}{entry.attempts?.length ? ` · ${entry.attempts.length} 次尝试` : ""}</summary>
                      <p className="mono">请求编号：{entry.requestId}</p>
                      {entry.durationMs != null && <p>总耗时：{entry.durationMs} ms</p>}
                      {entry.errorMessage && <p>{entry.errorMessage}</p>}
                      {entry.attempts?.length ? (
                        <ol>
                          {entry.attempts.map((attempt) => (
                            <li key={attempt.number}>
                              {attempt.vendorName || vendorName(vendors.data ?? [], attempt.vendorId)}
                              {` · ${attempt.upstreamStatus ?? "无 HTTP 响应"} · ${attempt.durationMs} ms`}
                              {attempt.errorMessage && ` · ${attempt.errorMessage}`}
                            </li>
                          ))}
                        </ol>
                      ) : <p>此记录没有上游尝试明细。</p>}
                    </details>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <Pagination
          label="请求日志分页"
          total={total}
          page={page}
          pageSize={pageSize}
          onPageChange={gotoPage}
          onPageSizeChange={setPageSize}
          disabled={logs.loading}
        />
      </Card>
    </div>
  );
}
