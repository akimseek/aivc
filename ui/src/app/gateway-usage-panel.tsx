import { useCallback, useState } from "react";

import {
  Card,
  ChipGroup,
  DescriptionList,
  EmptyState,
  LoadingBlock,
  Toolbar,
  ToolbarSpacer,
  useAsyncData
} from "../components";
import * as api from "../ipc/commands";
import type { UsageDimension } from "../types";
import {
  USAGE_RANGES,
  customBounds,
  formatCost,
  formatDuration,
  formatTokenCount,
  rangeBounds,
  type UsageRangeKey
} from "./gateway-range";
import { GatewayRangePicker } from "./gateway-range-picker";

/**
 * 用量报表：总览 + **按供应商** + **按模型**。
 *
 * # 它与「用量」那一块的分工
 *
 * `GatewayInsights` 里的用量只给总数，回答「花了多少」；
 * 这里回答「**花在哪**」——哪家供应商、哪个模型。两个问题不一样，
 * 因此是两块内容而不是一块：合成一块之后，用户每次想知道「哪个模型最贵」
 * 都要在一堆总数里自己找。
 *
 * 后端 `gateway:get-usage-report` 一直在，界面此前没有接。
 */
export function GatewayUsagePanel({ onError }: { onError: (message: string) => void }) {
  const [picker, setPicker] = useState({ range: "7d" as UsageRangeKey, start: "", end: "" });
  /**
   * 自定义区间还没填全时**不发请求**。
   *
   * 拿一个自己编的区间去查，用户会以为「自定义」没生效——而真相是他还没填完。
   * `bounds` 为 `null` 时下面的 `useAsyncData` 会一直停在加载态，
   * 这也正是想要的：那两条提示语已经把原因说清楚了。
   */
  const custom = picker.range === "custom" ? customBounds(picker.start, picker.end) : null;
  const bounds = picker.range === "custom" ? null : rangeBounds(picker.range);
  const start = custom?.ok ? custom.start : (bounds?.start ?? "");
  const end = custom?.ok ? custom.end : (bounds?.end ?? "");

  const load = useCallback(
    () => (start && end ? api.getUsageReport(start, end) : Promise.resolve(null)),
    [start, end]
  );
  const report = useAsyncData(load, { errorMessage: "用量报表读取失败" });

  return (
    <>
      <Card
        title="用量报表"
        description="按供应商与模型分组。「全部时间」的起点是 Unix 纪元——它覆盖所有已有记录。"
        actions={
          <>
            <ChipGroup
              value={picker.range}
              onChange={(value) =>
                setPicker((current) => ({ ...current, range: value as UsageRangeKey }))
              }
              options={USAGE_RANGES.filter((item) => item.key !== "custom").map((item) => ({
                value: item.key,
                label: item.label
              }))}
            />
            {/*
              自定义日期**单独一行**（不在 actions 里）：两个日期框塞进右上角会把标题挤没。
              三个观测面板共用同一个组件，因此三处的提示语与校验必然一致。
            */}
            <GatewayRangePicker
              range={picker.range}
              start={picker.start}
              end={picker.end}
              onChange={setPicker}
            />
          </>
        }
      >
        {report.loading ? (
          <LoadingBlock label="正在汇总用量…" />
        ) : report.error ? (
          <EmptyState message="用量报表读取失败。" hint={report.error} />
        ) : !report.data ? (
          <EmptyState message="这个区间内没有用量记录。" />
        ) : (
          <DescriptionList
            items={[
              { label: "请求数", value: report.data.summary.requestCount },
              { label: "成功", value: report.data.summary.successCount, tone: "ok" },
              {
                label: "失败",
                value: report.data.summary.failureCount,
                tone: report.data.summary.failureCount > 0 ? "warn" : undefined
              },
              { label: "平均耗时", value: formatDuration(report.data.summary.averageDurationMs) },
              /*
                token 分项逐条列出来，而不是只给一个总数。
                
                「缓存输入」尤其要紧：它通常是**便宜很多**的那一部分（命中缓存的价格
                往往只有几分之一），因此「总数多少」不如「其中多少是缓存的」有用。
                后端一直在发这些字段，此前界面只显示了 `总 token`。
                
                缺字段是常态（不同上游记录的信息多寡不一样），`formatTokenCount`
                对 `undefined` 给 `-` 而不是 `0`——`0` 会被读成「确实没有缓存」。
              */
              { label: "输入 token", value: formatTokenCount(report.data.summary.inputTokens) },
              { label: "输出 token", value: formatTokenCount(report.data.summary.outputTokens) },
              {
                label: "缓存输入",
                value: formatTokenCount(report.data.summary.cachedInputTokens)
              },
              {
                label: "推理 token",
                value: formatTokenCount(report.data.summary.reasoningTokens)
              },
              { label: "总 token", value: formatTokenCount(report.data.summary.totalTokens) },
              { label: "费用", value: formatCost(report.data.summary.costUsd) }
            ]}
          />
        )}
      </Card>

      {report.data && (
        <>
          <DimensionTable
            title="按供应商"
            firstColumn="供应商"
            empty="这个区间内没有供应商维度的记录。"
            rows={report.data.vendors}
          />
          <DimensionTable
            title="按模型"
            firstColumn="模型"
            empty="这个区间内没有模型维度的记录——上游可能没返回模型名。"
            rows={report.data.models}
          />
        </>
      )}

      {report.error === "" && (
        <Toolbar>
          <ToolbarSpacer />
          <button type="button" className="btn btn-sm" onClick={() => void report.reload()}>
            刷新
          </button>
        </Toolbar>
      )}

      {void onError}
    </>
  );
}

/**
 * 一个维度的表格。
 *
 * 供应商与模型**共用同一个形状**（后端的 `UsageDimension` 就是一份），
 * 因此两张表共用一个组件：分开写会很快出现「一张有 token 列、另一张没有」这类不一致。
 */
function DimensionTable({
  title,
  firstColumn,
  empty,
  rows
}: {
  title: string;
  firstColumn: string;
  empty: string;
  rows: UsageDimension[];
}) {
  return (
    <Card title={title}>
      {rows.length === 0 ? (
        <EmptyState message={empty} />
      ) : (
        <table className="data-table">
          <thead>
            <tr>
              <th>{firstColumn}</th>
              <th>平台</th>
              <th>请求</th>
              <th>失败</th>
              <th>切换</th>
              <th>平均耗时</th>
              {/* token 分项：只给总数看不出钱花在哪，「缓存输入」往往是省下来的那部分。 */}
              <th>输入</th>
              <th>输出</th>
              <th>缓存输入</th>
              <th>总 token</th>
              <th>费用</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr key={`${row.providerId}:${row.key}`}>
                {/* `label` 是展示名、`key` 是真实身份。供应商被删掉时 label 是占位文案，
                    因此把 key 放在 title 上，鼠标悬停能看到真正是谁。 */}
                <td title={row.key}>{row.label}</td>
                <td>{row.providerId}</td>
                <td>{row.requestCount}</td>
                <td>
                  <span className={`tag ${row.failureCount > 0 ? "off" : "ok"}`}>
                    {row.failureCount}
                  </span>
                </td>
                <td>{row.switchedCount}</td>
                <td>{formatDuration(row.averageDurationMs)}</td>
                <td>{formatTokenCount(row.inputTokens)}</td>
                <td>{formatTokenCount(row.outputTokens)}</td>
                <td>{formatTokenCount(row.cachedInputTokens)}</td>
                <td>{formatTokenCount(row.totalTokens)}</td>
                <td>{formatCost(row.costUsd)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </Card>
  );
}
