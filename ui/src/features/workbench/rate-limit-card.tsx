import { useEffect, useState } from "react";

import { Card, Icon, ProgressBar, ToolbarSpacer } from "../../components";
import { formatWhen } from "../../app/session-display";
import { progressTone, rateLimitText, resetCountdown, type RateWindow } from "./workbench-cards";

/**
 * CLI 速率限制监控：主窗口 / 次窗口的用量条与重置倒计时。
 *
 * 数据**只读会话摘要里已有的字段**，工作台不为它额外请求 CLI 或 Gateway
 * ——旧平台的注释写得很明确：工作台是概览，不该成为新的负担。
 *
 * # 重置时间为什么是倒计时（裁定 R25）
 *
 * 「14:32 重置」要用户自己做一次减法，而这个时间跨天时连日期都没有（`formatWhen`
 * 只给 `09-24`）。倒计时是这一格真正要回答的问题：**还能用多久**。
 *
 * 它靠一个**纯本地**的每秒 tick 实现：
 *
 * - 不发起任何数据请求，也不改上面的读数——只是把「还有多久」重新算一遍；
 * - 定时器只在这张卡**挂载期间**存在，卸载即清（工作台关掉后不该还有一个定时器在跑）；
 * - 它的状态只在这张卡里，重渲染的也只有这张卡（卡片自己的 `useEffect`）；
 * - 没有一个窗口带 `resetsAt` 时不起定时器：那种卡里没有会变的东西。
 *
 * 这与「不新增轮询」不冲突：轮询说的是**取数**，而这里一个字节都不取。
 */
export function RateLimitCard({ windows }: { windows: RateWindow[] }) {
  const hasReset = windows.some((item) => item.resetsAt);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    if (!hasReset) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [hasReset]);

  return (
    <Card title="CLI 速率限制监控" description="读数来自会话摘要，不额外请求 CLI。">
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
                {resetText(item.resetsAt, now)}
              </span>
            )}
          </div>
        ))}
      </div>
    </Card>
  );
}

/**
 * 一行的重置说明。
 *
 * 倒计时**算得出来才算得出来**：`resetCountdown` 给 `null`（时间戳坏了、或者那个
 * 时刻已经过去）时退回 `formatWhen` 的绝对时间，绝不把 `NaN` 印到脸上。
 */
function resetText(resetsAt: string, now: number): string {
  const countdown = resetCountdown(resetsAt, now);
  return countdown ? `将于 ${countdown} 后重置` : `${formatWhen(resetsAt)} 重置`;
}
