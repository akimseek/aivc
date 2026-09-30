import { Card, Icon, ProgressBar, ToolbarSpacer } from "../../components";
import { formatWhen } from "../../app/session-display";
import { progressTone, rateLimitText, type RateWindow } from "./workbench-cards";

/**
 * CLI 速率限制监控：主窗口 / 次窗口的用量条与重置时间。
 *
 * 数据**只读会话摘要里已有的字段**，工作台不为它额外请求 CLI 或 Gateway
 * ——旧平台的注释写得很明确：工作台是概览，不该成为新的负担。
 */
export function RateLimitCard({ windows }: { windows: RateWindow[] }) {
  return (
    <Card title="速率限制" description="读数来自会话摘要，不额外请求 CLI。">
      <div className="rate-windows">
        {windows.map((item) => (
          <div className="rate-window" key={item.label}>
            <div className="rate-window-head">
              <span className="rate-window-label">{item.label}</span>
              <ToolbarSpacer />
              <span className="ui-num rate-window-value">{rateLimitText(item)}</span>
            </div>
            {/* 没有数字时 `rateLimitText` 已经写了「无数据」，这里就不再画一根空条。 */}
            {item.usedPercent !== undefined && (
              <ProgressBar
                value={item.usedPercent}
                tone={progressTone(item.usedPercent)}
                label={item.label}
              />
            )}
            {item.resetsAt && (
              <span className="rate-window-reset">
                <Icon name="clock" size={12} />
                {formatWhen(item.resetsAt)} 重置
              </span>
            )}
          </div>
        ))}
      </div>
    </Card>
  );
}
