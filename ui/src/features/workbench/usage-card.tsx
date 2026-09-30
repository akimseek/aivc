import { Button, Card, DescriptionList, EmptyState, Icon, LoadingBlock } from "../../components";
import { formatCost, formatDuration, formatTokenCount } from "../../app/gateway-range";
import type { UsageSummary } from "../../types";
import { successRateText } from "./workbench-cards";

/**
 * 今日 Gateway 用量：请求数、成功率、平均耗时，外加一个去用量报表的出口。
 *
 * 三个数是**一行就能读完**的结论（今天忙不忙、有没有出事、慢不慢），
 * 明细（按供应商、按模型）交给设置中心的用量报表——那里有区间筛选与表格，
 * 在工作台里塞一张表只会让人不读。
 */
export function UsageCard({
  usage,
  loading,
  onOpenUsage
}: {
  usage: UsageSummary | null;
  loading: boolean;
  onOpenUsage: () => void;
}) {
  return (
    <Card
      title="今日 Gateway 用量"
      description="今天零点到现在。"
      actions={
        <Button small onClick={onOpenUsage} title="打开设置中心的用量报表">
          用量明细
          <Icon name="external" size={12} />
        </Button>
      }
    >
      {loading ? (
        <LoadingBlock label="正在汇总今日用量…" />
      ) : !usage ? (
        <EmptyState
          message="今日还没有网关请求。"
          hint="终端接入网关之后，这里会显示请求数与耗时。"
        />
      ) : (
        <>
          <div className="usage-metrics">
            <div className="usage-metric">
              <span className="usage-metric-value ui-num">{usage.requestCount}</span>
              <span className="usage-metric-label">请求</span>
            </div>
            <div className="usage-metric">
              <span className="usage-metric-value ui-num">
                {successRateText(usage.requestCount, usage.successCount)}
              </span>
              <span className="usage-metric-label">成功率</span>
            </div>
            <div className="usage-metric">
              <span className="usage-metric-value ui-num">
                {formatDuration(usage.averageDurationMs)}
              </span>
              <span className="usage-metric-label">平均耗时</span>
            </div>
          </div>
          <DescriptionList
            items={[
              {
                label: "失败",
                value: usage.failureCount,
                tone: usage.failureCount > 0 ? "warn" : undefined
              },
              { label: "故障切换", value: usage.switchedCount },
              {
                label: "Token / 费用",
                value: `${formatTokenCount(usage.totalTokens)} / ${formatCost(usage.costUsd)}`
              }
            ]}
          />
        </>
      )}
    </Card>
  );
}
