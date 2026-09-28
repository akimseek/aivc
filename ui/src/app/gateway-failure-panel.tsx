import { useCallback, useState } from "react";

import {
  Button,
  Card,
  ChipGroup,
  EmptyState,
  LoadingBlock,
  Select,
  Toolbar,
  ToolbarSpacer,
  useAsyncData
} from "../components";
import * as api from "../ipc/commands";
import type { AiProviderId } from "../types";
import { failureSummary, formatDuration, formatLogTime, outcomeLabel } from "./gateway-range";

/** 每页条数。后端会把它钳制在上限内，因此这里给的是一个「意愿」。 */
const PAGE_SIZE = 20;

/**
 * Gateway 异常诊断。
 *
 * # 它回答的是「为什么失败」
 *
 * `GatewayInsights` 里的日志表只列了几条最近请求，用来扫一眼；
 * 这个面板才是**查问题**用的：按平台 / 供应商 / 结果 / 时间筛选，可翻页，
 * 每条带上错误代码、上游状态、耗时、重试次数。
 *
 * 后端 `gateway:get-failure-diagnostics` 一直在，界面此前没有接。
 */
export function GatewayFailurePanel({
  providerId,
  onError
}: {
  providerId: AiProviderId | "";
  onError: (message: string) => void;
}) {
  const [page, setPage] = useState(1);
  const [outcome, setOutcome] = useState<"" | "error" | "timeout">("");
  const [vendorId, setVendorId] = useState("");
  const [useCurrentProvider, setUseCurrentProvider] = useState(true);

  /**
   * 筛选条件变化时**回到第 1 页**。
   *
   * 不回到第 1 页会出现一个很难解释的现象：在第 5 页筛出 3 条结果，界面显示空白，
   * 用户以为「筛选没生效」，其实是第 5 页不存在。
   */
  const applyFilter = useCallback((action: () => void) => {
    action();
    setPage(1);
  }, []);

  const load = useCallback(
    () =>
      api.getFailureDiagnostics({
        page,
        pageSize: PAGE_SIZE,
        // 空字符串**不传**，而不是传空串：后端把空串当「不筛选」，
        // 但少传一个无意义参数总是更清楚。
        ...(vendorId ? { vendorId } : {}),
        ...(useCurrentProvider && providerId ? { providerId } : {}),
        ...(outcome ? { outcome } : {})
      }),
    [page, vendorId, useCurrentProvider, providerId, outcome]
  );

  const diagnostics = useAsyncData(load, { errorMessage: "异常诊断读取失败" });
  const total = diagnostics.data?.total ?? 0;
  const pageCount = Math.max(1, Math.ceil(total / PAGE_SIZE));

  return (
    <>
      <Card
        title="Gateway 异常诊断"
        description="按平台、供应商、结果筛选。错误代码比「结果」更能说明问题——「路由不存在 / 鉴权失败」记的是成功。"
        actions={
          <Button small onClick={() => void diagnostics.reload()}>
            重新查询
          </Button>
        }
      >
        <Toolbar>
          <label className="field">
            <span className="field-label">按平台筛选</span>
            <span className="control-check">
              <input
                type="checkbox"
                checked={useCurrentProvider}
                onChange={(event) => applyFilter(() => setUseCurrentProvider(event.target.checked))}
              />
              <span>{useCurrentProvider && providerId ? `仅限 ${providerId}` : "全部平台"}</span>
            </span>
          </label>

          <label className="field">
            <span className="field-label">按结果筛选</span>
            <Select
              value={outcome}
              onChange={(event) =>
                applyFilter(() => setOutcome(event.target.value as "" | "error" | "timeout"))
              }
            >
              <option value="">全部结果</option>
              <option value="error">请求失败</option>
              <option value="timeout">请求超时</option>
            </Select>
          </label>

          <label className="field">
            <span className="field-label">按供应商筛选</span>
            <input
              className="control"
              value={vendorId}
              placeholder="供应商编号，留空表示全部"
              onChange={(event) => applyFilter(() => setVendorId(event.target.value.trim()))}
            />
          </label>
        </Toolbar>
      </Card>

      <Card
        title={`诊断明细（共 ${total} 条）`}
        actions={
          pageCount > 1 && (
            <Toolbar>
              <Button small disabled={page <= 1} onClick={() => setPage(1)}>
                第一页
              </Button>
              <Button small disabled={page <= 1} onClick={() => setPage((current) => current - 1)}>
                上一页
              </Button>
              <span className="row-meta">
                第 {page} / {pageCount} 页
              </span>
              <Button
                small
                disabled={page >= pageCount}
                onClick={() => setPage((current) => current + 1)}
              >
                下一页
              </Button>
              <ToolbarSpacer />
            </Toolbar>
          )
        }
      >
        {diagnostics.loading ? (
          <LoadingBlock label="正在读取诊断明细…" />
        ) : diagnostics.error ? (
          <EmptyState message="诊断读取失败。" hint={diagnostics.error} />
        ) : !diagnostics.data || diagnostics.data.items.length === 0 ? (
          <EmptyState
            message="没有符合条件的异常记录。"
            hint="这是好消息；如果确实期望有记录，先确认网关是否在服务请求。"
          />
        ) : (
          <table className="data-table">
            <thead>
              <tr>
                <th>时间</th>
                <th>平台</th>
                <th>供应商</th>
                <th>结果</th>
                <th>上游状态</th>
                <th>重试</th>
                <th>耗时</th>
                <th>原因</th>
              </tr>
            </thead>
            <tbody>
              {diagnostics.data.items.map((item, index) => (
                <tr key={`${item.createdAt}:${item.vendorId}:${index}`}>
                  <td className="mono">{formatLogTime(item.createdAt)}</td>
                  <td>{item.providerId}</td>
                  <td>{item.vendorId}</td>
                  <td>
                    <span className={`tag ${item.outcome === "error" ? "off" : ""}`}>
                      {outcomeLabel(item.outcome)}
                    </span>
                  </td>
                  <td>{item.upstreamStatus ?? "-"}</td>
                  <td>{item.retryCount}</td>
                  <td>{formatDuration(item.durationMs)}</td>
                  {/* 原因按「错误代码 → 上游状态 → 错误消息」取第一个可用的：
                      错误消息常常是一整段 HTML/JSON，表格里读不了。 */}
                  <td className="mono" title={item.errorMessage}>
                    {failureSummary(item)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>

      {void onError}
    </>
  );
}

/** 结果筛选用的分段控件。放在这里是因为它只服务于本面板的语义。 */
export function OutcomeChips({
  value,
  onChange
}: {
  value: string;
  onChange: (next: string) => void;
}) {
  return (
    <ChipGroup
      value={value}
      onChange={onChange}
      options={[
        { value: "", label: "全部" },
        { value: "error", label: "失败" },
        { value: "timeout", label: "超时" }
      ]}
    />
  );
}
