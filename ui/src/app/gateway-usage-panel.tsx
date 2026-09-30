import { useCallback, useState } from "react";

import {
  Button,
  Card,
  DescriptionList,
  EmptyState,
  LoadingBlock,
  useAsyncData
} from "../components";
import * as api from "../ipc/commands";
import { PanelHeader } from "../features/settings/panel-header";
// 成功率只有一处实现：工作台的「今日用量」与本页读的是同一个比值，
// 各写一遍迟早会出现「一处 `0.0%`、一处 `-`」。
import { successRateText } from "../features/workbench/workbench-cards";
import type { UsageDimension, UsageSummary } from "../types";
import {
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
    <div className="observability-panel">
      {/* 面板头统一：标题与范围选择从卡片头搬到这里，卡片本体只管内容。 */}
      <PanelHeader
        title="用量报表"
        description="按供应商与模型分组。「全部时间」的起点是 Unix 纪元——它覆盖所有已有记录。"
        actions={
          <>
            {/*
              范围选择**只有一个入口**（这个组件）：此前这一页还有一个并列的预设
              chips，同一个状态两个控件，而 chips 里偏偏没有「自定义」——用户会以为
              两者是不同的东西。三个观测面板共用同一个组件，因此三处的提示语与
              校验必然一致。
            */}
            <GatewayRangePicker
              range={picker.range}
              start={picker.start}
              end={picker.end}
              onChange={setPicker}
            />
            {/*
              「重新查询」与异常、日志两页同一个位置、同一个措辞。
              它不随加载结果隐藏：读失败时恰恰最需要它。
            */}
            <Button small onClick={() => void report.reload()}>
              重新查询
            </Button>
          </>
        }
      />

      {/* 三个读数是这一页的结论，放在最上面；没拿到报表时不出（下方卡片会说原因）。 */}
      {report.data && <SummaryCards summary={report.data.summary} />}

      <Card title="区间明细">
        {report.loading ? (
          <LoadingBlock label="正在汇总用量…" />
        ) : report.error ? (
          <EmptyState message="用量报表读取失败。" hint={report.error} />
        ) : !report.data ? (
          <EmptyState message="这个区间内没有用量记录。" />
        ) : (
          /*
            明细里**不再重复**上面三张卡片的数字：请求数、总 token、费用已经在
            卡片上，四十像素外再写一遍只会让人以为它们可能不一样。这里留下卡片
            给不出的分项——成功率之外的成功/失败条数、耗时与 token 的构成。

            token 分项逐条列出来，而不是只给一个总数。「缓存输入」尤其要紧：它通常是
            **便宜很多**的那一部分（命中缓存的价格往往只有几分之一），因此「总数多少」
            不如「其中多少是缓存的」有用。后端一直在发这些字段。

            缺字段是常态（不同上游记录的信息多寡不一样），`formatTokenCount`
            对 `undefined` 给 `-` 而不是 `0`——`0` 会被读成「确实没有缓存」。
          */
          <DescriptionList
            items={[
              { label: "成功", value: report.data.summary.successCount, tone: "ok" },
              {
                label: "失败",
                value: report.data.summary.failureCount,
                tone: report.data.summary.failureCount > 0 ? "warn" : undefined
              },
              { label: "平均耗时", value: formatDuration(report.data.summary.averageDurationMs) },
              { label: "输入 token", value: formatTokenCount(report.data.summary.inputTokens) },
              { label: "输出 token", value: formatTokenCount(report.data.summary.outputTokens) },
              {
                label: "缓存输入",
                value: formatTokenCount(report.data.summary.cachedInputTokens)
              },
              {
                label: "推理 token",
                value: formatTokenCount(report.data.summary.reasoningTokens)
              }
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

      {void onError}
    </div>
  );
}

/**
 * 一行三张汇总卡：请求数（含成功率）、累计 Tokens、预估费用。
 *
 * 它们与下方维度表的分工，和「今日用量」卡与这一页的分工是同一件事：
 * 卡片回答「花了多少」，表格回答「花在哪」。
 */
function SummaryCards({ summary }: { summary: UsageSummary }) {
  return (
    <div className="stat-cards">
      <div className="stat-card">
        <span className="stat-card-label">请求数</span>
        <span className="stat-card-value ui-num">{formatCount(summary.requestCount)}</span>
        {/*
          成功率并进请求数这张卡，而不是单开第四张：没有请求时它只能是 `-`，
          单独一张「-」的卡片占位却什么都没说。
        */}
        <span className="stat-card-note">
          成功率 {rateText(summary.requestCount, summary.successCount)}
        </span>
      </div>
      <div className="stat-card">
        <span className="stat-card-label">累计 Tokens</span>
        <span className="stat-card-value ui-num">{formatTokenCount(summary.totalTokens)}</span>
      </div>
      <div className="stat-card">
        <span className="stat-card-label">预估费用</span>
        <span className="stat-card-value ui-num">{formatCost(summary.costUsd)}</span>
      </div>
    </div>
  );
}

/**
 * 卡片上的计数。
 *
 * 缺字段给 `-` 而不是 `0`：`0` 会被读成「确实一次请求都没有」，
 * 而真相是这份报表没给这个数。`NaN` 同理不能落到界面上。
 */
function formatCount(value: number | undefined): string {
  return value === undefined || Number.isNaN(value) ? "-" : String(value);
}

/**
 * 成功率。
 *
 * **缺任何一个都给 `-`**，而不是把缺的那个补齐成 `0` 再算：请求数有值而成功数缺失时，
 * `0/1280` 会算出 `0.0%`——那读起来是「请求全都失败了」，真相却是这个数没给
 * （与 `formatCount` 同一条规矩：不知道就说不知道）。
 *
 * `UsageSummary` 里这两项是必有字段，但界面此前就按「可能缺」写过（`?? 0`）：
 * 值来自 IPC、绕不过类型，因此这里保留那份防御，只是不再用 0 去凑一个比值。
 */
function rateText(requestCount: number | undefined, successCount: number | undefined): string {
  if (requestCount === undefined || successCount === undefined) return "-";
  return successRateText(requestCount, successCount);
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
        /* `dense`：这两张表列多、行里全是数字，行距收一档、单元格居中，
           数字列再加 `.ui-num`（等宽数字）——不等宽时列对不齐，扫一列很难。 */
        <table className="data-table dense">
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
                <td className="ui-num">{row.requestCount}</td>
                <td>
                  <span className={`tag ui-num ${row.failureCount > 0 ? "off" : "ok"}`}>
                    {row.failureCount}
                  </span>
                </td>
                <td className="ui-num">{row.switchedCount}</td>
                <td className="ui-num">{formatDuration(row.averageDurationMs)}</td>
                <td className="ui-num">{formatTokenCount(row.inputTokens)}</td>
                <td className="ui-num">{formatTokenCount(row.outputTokens)}</td>
                <td className="ui-num">{formatTokenCount(row.cachedInputTokens)}</td>
                <td className="ui-num">{formatTokenCount(row.totalTokens)}</td>
                <td className="ui-num">{formatCost(row.costUsd)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </Card>
  );
}
