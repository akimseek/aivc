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
 * 自定义区间的结果。
 *
 * # 为什么不是「返回 `null` 或区间」
 *
 * 旧平台对这两个输入有**两条不同的提示**：
 *
 * | 情况 | 旧平台的提示 |
 * | --- | --- |
 * | 只填了一端 | `请选择完整的起止日期。` |
 * | 起始晚于结束 | `起始日期不能晚于结束日期。` |
 *
 * 用一个 `null` 表达不了「是哪一种」，于是界面只能给一句笼统的话，
 * 而用户看不出自己该补哪个框、还是该调顺序。因此这里把原因带出来。
 */
export type CustomBounds =
  | { ok: true; start: string; end: string }
  | { ok: false; reason: "incomplete" | "reversed" };

/** 两条提示语，与旧平台逐字一致。 */
export const CUSTOM_BOUNDS_MESSAGES: Record<"incomplete" | "reversed", string> = {
  incomplete: "请选择完整的起止日期。",
  reversed: "起始日期不能晚于结束日期。"
};

/**
 * 从两个日期输入框的值算出区间。
 *
 * 结束日期取**当天 23:59:59.999**：用户选「到 10 月 8 日」的意思是包含 10 月 8 日一整天。
 * 直接用它当 00:00 会把最后一天整个漏掉，而这种错看起来只是「数字小了一点」。
 *
 * # 起止颠倒时**报错，不静默交换**
 *
 * 这里原来是交换两端并照常查询。改成报错有两个理由：
 *
 * 1. 静默重排用户输入是那种会让人不信任工具的行为——他明明填反了，
 *    工具却假装没看见，从此他不知道自己填的到底被怎么理解了；
 * 2. 旧平台本来就是报错（`起始日期不能晚于结束日期。`），照它做不必另创一套语义。
 */
export function customBounds(start: string, end: string): CustomBounds {
  if (!start || !end) return { ok: false, reason: "incomplete" };
  const from = new Date(`${start}T00:00:00`);
  const to = new Date(`${end}T23:59:59.999`);
  // 日期串解析失败也按「没填全」处理：用户看到的是「请选择完整的起止日期」，
  // 而他确实是没给一个能用的日期。
  if (Number.isNaN(from.getTime()) || Number.isNaN(to.getTime())) {
    return { ok: false, reason: "incomplete" };
  }
  if (from > to) return { ok: false, reason: "reversed" };
  return { ok: true, start: from.toISOString(), end: to.toISOString() };
}
