import { Button, Card, EmptyState, Icon, LoadingBlock } from "../../components";
import { formatDuration } from "../../app/gateway-format";
import { formatWhen } from "../../app/session-display";
import type { FailureDiagnostic } from "../../types";

/**
 * 最近 Gateway 异常：最近 5 条失败的一句话摘要，出口是设置中心的异常诊断页。
 *
 * 工作台这一格**故意不带筛选与分页**：它回答的是「刚才有没有出事」，
 * 要按条件翻、要看错误正文，去异常诊断页——那里有区间、供应商与分页。
 */
export function AttentionListCard({
  failures,
  loading,
  error,
  onOpenFailures
}: {
  failures: FailureDiagnostic[];
  loading: boolean;
  error?: string;
  onOpenFailures: () => void;
}) {
  return (
    <Card
      title="最近 Gateway 异常"
      description="最近 5 条。要看全部、按条件筛，去「配置 → 异常」。"
      actions={
        <Button small onClick={onOpenFailures} title="打开设置中心的异常诊断页">
          查看详情
          <Icon name="external" size={12} />
        </Button>
      }
    >
      {error ? (
        <EmptyState message="异常读取失败。" hint={error} />
      ) : loading && failures.length === 0 ? (
        <LoadingBlock label="正在读取最近异常…" />
      ) : failures.length === 0 ? (
        <EmptyState message="没有异常记录。" />
      ) : (
        <ul className="workbench-failures">
          {failures.map((item, index) => (
            <li key={`${item.createdAt}:${item.vendorId}:${index}`}>
              <strong>{item.errorCode ?? `HTTP ${item.upstreamStatus ?? "-"}`}</strong>
              <small>
                {formatWhen(item.createdAt)} · {item.vendorId} · {formatDuration(item.durationMs)}
              </small>
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}
