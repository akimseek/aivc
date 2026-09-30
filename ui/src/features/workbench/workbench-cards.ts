import type { AiSession, ApiVendor } from "../../types";

/**
 * 工作台五张卡用的**纯逻辑**。
 *
 * # 为什么从组件里搬出来
 *
 * 这些判断（会话怎么称呼、哪几个速率窗口、要不要转 warn、路由为什么变了）
 * 都是「算错一处、显示就错一处」的东西，而它们此前散在
 * `app/sidebar-workbench.tsx` 的渲染里——不渲染就没法断言。
 * 卡片只管画，判断留在这里，逐条有测试。
 */

/** 一条速率窗口。`label` 是「主窗口 / 次窗口」，取值与旧平台一致。 */
export type RateWindow = {
  label: string;
  usedPercent?: number;
  resetsAt?: string;
  windowMinutes?: number;
};

/** 最近一次路由变更。形状与 `app.tsx` 传给侧栏的那个一致。 */
export type RouteChange = { vendorId: string; reason: string; at: string };

/**
 * 会话那一格。
 *
 * 编号可能缺失（工作台只拿到一个带 `usage` 的片段就够了），因此**逐层兜底**：
 * 标题 → 编号前 8 位 → 「未打开」。直接 `session.id.slice(...)` 会在没有 `id` 时
 * 抛异常——整个工作台跟着白屏，而它本来只是想显示一行字。
 */
export function sessionLabel(session: AiSession | null | undefined): string {
  if (!session) return "未打开";
  const title = session.title?.trim();
  if (title) return title;
  const id = session.id ?? "";
  return id ? id.slice(0, 8) : "未命名会话";
}

/**
 * 会话摘要里自带的速率窗口，主窗口在前。
 *
 * `rateLimits` 整个缺席时返回**空数组**而不是一个 `undefined` 的洞：
 * 「这一项没有数据」与「这一行没有内容」是两件事，后者会让卡片渲染出一行空壳，
 * 调用方也会误以为有两条窗口。
 */
export function getRateLimitWindows(session: AiSession | null | undefined): RateWindow[] {
  const limits = session?.usage?.rateLimits;
  if (!limits) return [];
  const candidates = [
    limits.primary ? { label: "主窗口", ...limits.primary } : undefined,
    limits.secondary ? { label: "次窗口", ...limits.secondary } : undefined
  ];
  return candidates.filter((item): item is RateWindow => Boolean(item));
}

/**
 * 一根进度条的语气。
 *
 * 80% 是「再往上就该有人管了」的那条线：到了那里换成 warn，颜色与数字的语义一致。
 * 没有数字时给 accent——条根本不画（见 `rateLimitText`），这里只是不返回 warn。
 */
export function progressTone(usedPercent: number | undefined): "accent" | "warn" {
  return usedPercent !== undefined && usedPercent >= 80 ? "warn" : "accent";
}

/**
 * 一个速率窗口的读数。
 *
 * **没有数字就说「无数据」**：`undefined%` 会被读成一个真实的百分比。
 * 有 `windowMinutes` 时把它带上——单看一个 42% 不知道说的是 5 小时还是 5 分钟。
 */
export function rateLimitText(item: Pick<RateWindow, "usedPercent" | "windowMinutes">): string {
  if (item.usedPercent === undefined) return "无数据";
  const percent = `${item.usedPercent}%`;
  return item.windowMinutes ? `${percent}（${item.windowMinutes} 分钟窗口）` : percent;
}

/**
 * 距重置还有多久，形如 `3h 12m`。
 *
 * # 为什么返回 `null` 而不是编一个字符串
 *
 * 两种情况下这里给不出一个**说得出口**的时长：时间戳解析不出来（`new Date("oops")`
 * 算出 `NaN`，逐层算下去会印出 `NaNh NaNm`），以及那个时刻已经过去了（「还有 -3 分钟」
 * 不是人话）。两种都交给调用方回退到绝对时间——它至少是真的。
 *
 * `now` 由调用方传进来而不是在这里读 clock：这样这条判断不渲染、不等真实时间就能断言。
 */
export function resetCountdown(resetsAt: string, now: number): string | null {
  const target = Date.parse(resetsAt);
  if (!Number.isFinite(target)) return null;
  const remaining = target - now;
  if (remaining <= 0) return null;

  const totalMinutes = Math.floor(remaining / 60_000);
  if (totalMinutes <= 0) {
    // 最后一分钟给秒：`0m` 读起来像「已经重置了」，而它还在跑。
    return `${Math.max(1, Math.floor(remaining / 1000))}s`;
  }
  const days = Math.floor(totalMinutes / (24 * 60));
  const hours = Math.floor((totalMinutes % (24 * 60)) / 60);
  const minutes = totalMinutes % 60;
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  return `${minutes}m`;
}

/**
 * 供应商编号 → 名字。
 *
 * 只显示 `vendor-3f2a` 用户认不出那是谁，所以能翻就翻；翻不出来**退回编号**，
 * 而不是留一个空格（空白会被读成「没有供应商」）。
 */
export function vendorName(vendors: ApiVendor[], vendorId: string): string {
  return vendors.find((item) => item.id === vendorId)?.name ?? vendorId;
}

/**
 * 成功率。
 *
 * 没有请求时返回 `-`：0/0 算出来的 `0.0%` 会被读成「全都失败了」。
 */
export function successRateText(requestCount: number, successCount: number): string {
  if (requestCount <= 0) return "-";
  return `${((successCount / requestCount) * 100).toFixed(1)}%`;
}

/**
 * 路由变更的原因。
 *
 * 三种原因**对用户的意义完全不同**：手动是他自己点的，故障切换是网关替他做的
 * （他什么都没做），候选池调整是配置变了。混成一句「供应商已切换」会让人以为是自己点的。
 */
export function describeSwitchReason(reason: string): string {
  switch (reason) {
    case "manual":
      return "你手动切换的。";
    case "failure":
      return "上游失败后网关自动切换的——你没有做任何操作。";
    case "candidate-pool":
      return "候选池变化引起的调整。";
    default:
      return "路由发生了变化。";
  }
}
