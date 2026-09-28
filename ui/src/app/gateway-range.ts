/**
 * 用量与日志的时间区间。
 *
 * # 为什么单独一个文件
 *
 * 「今天 / 近 7 天 / 近 30 天 / 全部时间 / 自定义」这套区间在**三个**面板里都要用
 * （用量报表、异常诊断、日志清理），而后端对区间的契约是硬的：
 * `period()` 缺 `periodStart` 或 `periodEnd` 就直接报错。
 *
 * 也就是说「算出这两个时间戳」是一个**必须一次做对**的纯函数：算错一天，
 * 用户看到的数字就是错的，而且看起来完全正常。这里把它抽出来，逐条断言。
 */

export type UsageRangeKey = "today" | "7d" | "30d" | "all" | "custom";

export type UsageRange = {
  key: UsageRangeKey;
  label: string;
};

/** 界面上的区间选项。顺序与旧平台一致：今天 → 近 7 天 → 近 30 天 → 全部 → 自定义。 */
export const USAGE_RANGES: UsageRange[] = [
  { key: "today", label: "今天" },
  { key: "7d", label: "近 7 天" },
  { key: "30d", label: "近 30 天" },
  { key: "all", label: "全部时间" },
  { key: "custom", label: "自定义" }
];

/**
 * 区间的**两端**。
 *
 * 「全部时间」不是一个可以算出来的区间——后端要两个具体的时间戳。
 * 这里用 Unix 纪元起点到当下：它覆盖了所有可能存在的记录，
 * 而且是一个**具体值**，不会让后端为难。
 *
 * # 为什么 `today` 用本地零点而不是 UTC 零点
 *
 * 用户说的「今天」是他手表上的今天。用 UTC 零点的话，东八区的用户在北京时间早上 8 点
 * 之前看到的「今天」其实是昨天。
 */
export function rangeBounds(
  key: UsageRangeKey,
  now: Date = new Date()
): { start: string; end: string } {
  const end = now.toISOString();
  if (key === "all") {
    return { start: new Date(0).toISOString(), end };
  }
  if (key === "today") {
    const start = new Date(now.getFullYear(), now.getMonth(), now.getDate());
    return { start: start.toISOString(), end };
  }
  const days = key === "7d" ? 7 : 30;
  const start = new Date(now.getTime() - days * 24 * 60 * 60 * 1000);
  return { start: start.toISOString(), end };
}

/**
 * 从两个日期输入框的值算出区间。
 *
 * 两端都缺失时返回 `null`——调用方应当**不发请求**，而不是拿一个自己编的区间去查：
 * 那会让用户以为「自定义」没生效。
 *
 * 结束日期取**当天 23:59:59.999**：用户选「到 10 月 8 日」的意思是包含 10 月 8 日一整天。
 * 直接用它当 00:00 会把最后一天整个漏掉，而这种错看起来只是「数字小了一点」。
 */
export function customBounds(start: string, end: string): { start: string; end: string } | null {
  if (!start || !end) return null;
  const from = new Date(`${start}T00:00:00`);
  const to = new Date(`${end}T23:59:59.999`);
  if (Number.isNaN(from.getTime()) || Number.isNaN(to.getTime())) return null;
  // 起止颠倒时**交换**而不是报错：用户拖错了顺序是常事，而结果仍然是他想要的。
  return from <= to
    ? { start: from.toISOString(), end: to.toISOString() }
    : { start: to.toISOString(), end: from.toISOString() };
}

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
