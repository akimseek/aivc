/**
 * 网关相关的小格式化：时间、费用、耗时、token、失败摘要、结果文案与色调。
 *
 * # 为什么与 `gateway-range.ts` 分开
 *
 * 那个文件算的是**时间区间**——一次算错，用户看到的数字就是错的，且看起来完全正常；
 * 这里的函数是**显示层折叠**（`-` 占位、`12.3k`、结果色）。两者的出错形态、测试面
 * 与使用方都不同，混在一个文件里改一个格式要先读一整页区间语义。
 */

/** 把 ISO 时间显示成 `MM-DD HH:mm`。日志表里要能一眼扫出时间，秒是噪音。 */
export function formatLogTime(value: string | undefined): string {
  if (!value) return "-";
  const at = new Date(value);
  if (Number.isNaN(at.getTime())) return value;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(at.getMonth() + 1)}-${pad(at.getDate())} ${pad(at.getHours())}:${pad(at.getMinutes())}`;
}

/** 费用显示。`$0.1234` 而不是 `$0.12`——一次请求常常只花几厘钱。 */
export function formatCost(value: number | undefined): string {
  return value === undefined ? "-" : `$${value.toFixed(4)}`;
}

/** 耗时显示。毫秒取整；没数据时给 `-` 而不是 `0 ms`（后者会被读成「瞬间完成」）。 */
export function formatDuration(value: number | undefined): string {
  return value === undefined ? "-" : `${Math.round(value)} ms`;
}

/** token 数显示。原始值常有六位以上，窄表里读不出来。 */
export function formatTokenCount(value: number | undefined): string {
  if (value === undefined) return "-";
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(2)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return String(value);
}

/**
 * 失败原因的一句话摘要。
 *
 * 优先级：错误代码 → 上游状态码 → 错误消息 → 「未记录」。
 * 把 `error_code` 放在最前是因为它**最可操作**（用户能拿它去搜），
 * 而 `error_message` 常常是一整段 HTML 或 JSON，表格里读不了。
 */
export function failureSummary(item: {
  errorCode?: string;
  upstreamStatus?: number;
  errorMessage?: string;
}): string {
  if (item.errorCode) return item.errorCode;
  if (item.upstreamStatus !== undefined) return `HTTP ${item.upstreamStatus}`;
  if (item.errorMessage) return item.errorMessage;
  return "未记录";
}

/** 日志结果的中文名。后端给的是 `ok` / `error` / `timeout` / `client-aborted`。 */
export function outcomeLabel(outcome: string): string {
  switch (outcome) {
    case "ok":
      return "成功";
    case "error":
      return "失败";
    case "timeout":
      return "超时";
    case "client-aborted":
      return "已取消";
    default:
      return outcome;
  }
}

/**
 * 日志结果的语义色。只有 `ok` 是成功色，其余（含未知）一律中性色。
 *
 * 三张网关表的结果列共用这一个判据：请求日志、最近请求、异常诊断此前各写各的
 * 映射，异常诊断那份甚至把 timeout 显示成裸文本——同一个值在两张表里长得不一样。
 */
export function outcomeTone(outcome: string): "accent" | "neutral" {
  return outcome === "ok" ? "accent" : "neutral";
}
