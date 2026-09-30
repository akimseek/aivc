import type { AiMessage, AiSession } from "../types";

/**
 * 会话在列表与标签上的**显示用**文本。
 *
 * # 为什么需要这一层
 *
 * 后端在头部窗口（64 KB）里找不到合格的用户消息时，会把 `title` 回退成会话编号
 * （见 `docs/rust-rewrite/19`）。于是列表里 5 条会话的标题全是 `98895b66-…` 这样的
 * UUID —— 会话列表的唯一目的就是让用户**认出**要哪条，而 UUID 不提供任何区分信息。
 *
 * 界面能做的两件事：
 *
 * 1. 认出「这是回退结果」，改显示**有意义的替代信息**（目录 + 时间）；
 * 2. 清掉预览里的原始标签 —— 后端过滤掉的是「整条合成消息」，而
 *    `<multi_agent_mode>` 这类标记出现在消息**正文内部**，它不会被过滤。
 */

/** 一行预览的最大字符数。列表很窄，再长也读不到。 */
export const PREVIEW_MAX_CHARS = 80;

/**
 * 去掉正文里的尖括号标记。
 *
 * # 为什么可以宽一点
 *
 * 这个函数**只用于预览行**（会话正文由 `Message` 原样渲染，不经过这里）。预览是
 * 非权威的短文本：把正文里偶尔出现的 `<div>` 清掉，用户看不出损失；而留着
 * `<multi_agent_mode>` 则整行都没法读。所以这里按「小写标签名」一刀切。
 *
 * 大写开头的 `<T>` 不匹配 —— 泛型参数在预览里撞见的概率比 HTML 标签高。
 *
 * 匹配不上时原样返回，**不做任何猜测性清理**。
 */
export function stripMarkup(text: string): string {
  return text.replace(/<\/?[a-z][a-z0-9_]*>/g, " ");
}

/**
 * 预览行：清标记、压空白、截断。
 *
 * 返回空串表示这条消息没有可显示的内容——调用方应当**跳过它去找下一条**，
 * 而不是显示一片空白。
 */
export function previewLine(text: string, max = PREVIEW_MAX_CHARS): string {
  const cleaned = stripMarkup(text).replace(/\s+/g, " ").trim();
  if (cleaned.length <= max) return cleaned;
  return `${cleaned.slice(0, max)}…`;
}

/**
 * 从预览里挑一行可显示的文本。
 *
 * 从**最后往前**找：列表要的是「这条会话聊到哪了」，最后一条最接近。
 * 全部不可显示时返回空串。
 */
export function pickPreview(session: Pick<AiSession, "preview">): string {
  for (let index = session.preview.length - 1; index >= 0; index -= 1) {
    const line = previewLine(session.preview[index]?.text ?? "");
    if (line) return line;
  }
  return "";
}

/** 取路径的最后一段，同时兼容 `\` 与 `/`。 */
export function baseName(path: string | undefined): string {
  if (!path) return "";
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.at(-1) ?? "";
}

/**
 * 显示用的标题。
 *
 * 标题等于编号时说明后端回退了 —— 这时改显示「目录 · 时间」。
 * 那个组合能区分会话，而 UUID 不能。
 */
export function displayTitle(
  session: Pick<AiSession, "id" | "title" | "cwd" | "updatedAt" | "createdAt">
): string {
  const title = session.title?.trim() ?? "";
  if (title && title !== session.id) return title;

  const place = baseName(session.cwd);
  const when = formatWhen(session.updatedAt ?? session.createdAt);
  if (place && when) return `${place} · ${when}`;
  if (place) return place;
  if (when) return `未命名会话 · ${when}`;
  // 连目录和时间都没有时，**至少短一点**：完整 UUID 会把整行撑满。
  return `未命名会话 · ${session.id.slice(0, 8)}`;
}

/** 标题是否退化成了编号（供界面标注用）。 */
export function isTitleFallback(session: Pick<AiSession, "id" | "title">): boolean {
  const title = session.title?.trim() ?? "";
  return title === "" || title === session.id;
}

/**
 * 把 ISO 时间显示成「14:32 / 09-24」这样够用的粒度。
 *
 * 今天只显示时刻：同一天里「09-24 14:32」有六个字符是多余的；跨天再补日期。
 */
export function formatWhen(value: string | undefined): string {
  if (!value) return "";
  const at = new Date(value);
  if (Number.isNaN(at.getTime())) return "";
  const now = new Date();
  const sameDay =
    at.getFullYear() === now.getFullYear() &&
    at.getMonth() === now.getMonth() &&
    at.getDate() === now.getDate();
  const hhmm = `${String(at.getHours()).padStart(2, "0")}:${String(at.getMinutes()).padStart(2, "0")}`;
  if (sameDay) return hhmm;
  return `${String(at.getMonth() + 1).padStart(2, "0")}-${String(at.getDate()).padStart(2, "0")}`;
}

/**
 * 会话条数的显示文字。
 *
 * # 为什么会有「≥」
 *
 * 列表模式只读文件的**头部窗口**，数出来的条数是**真实的下界**：数到 2 条，这个会话就
 * **至少**有 2 条（它可能有 80 条）。所以「≥2 条」是真话，「2 条」是假话，「—」是
 * 放弃表达。
 *
 * 判断依据是**后端随数字一起发来的** `messageCountIsLowerBound`，不是这里猜的。
 * 曾经有一版把「它是下界」只写在渲染层的注释里，被端到端测试挡了回来——那种约定只活在
 * 人的记忆里，正是本项目反复出问题的地方。
 *
 * 见 docs/rust-rewrite/20 第 7 节。
 *
 * @param count 后端给出的条数。
 * @param lowerBound 它是否只是下界（来自 `messageCountIsLowerBound`）。
 */
export function messageCountLabel(count: number, lowerBound = false): string {
  // 0 在这里只有一种含义：一条也没数到。它既可能是空会话，也可能是头部窗口什么都没
  // 读到——两者都给不出一个能说出口的数字。
  if (count <= 0) return "—";
  return lowerBound ? `≥${count} 条` : `${count} 条`;
}

/**
 * 消息里「谁说的」的措辞。
 *
 * 放在这里而不是各渲染处各写一遍：会话正文（`history-view.tsx`）与详情弹层
 * （`session-row.tsx` 的 `Message`）看的是同一批消息，两处措辞漂移时，同一条消息
 * 在两个地方会显示成两个称呼。
 */
export function roleLabel(role: AiMessage["role"]): string {
  if (role === "user") return "你";
  if (role === "assistant") return "助手";
  if (role === "tool") return "工具";
  if (role === "system") return "系统";
  return "记录";
}

/**
 * 把 token 数显示成 `12.4k` 这样的紧凑形式。
 *
 * 原始值常有六位以上（`4194304`），在状态栏那样窄的地方会把相邻字段挤掉。
 */
export function formatTokens(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return String(value);
}

/**
 * 会话区为空时**必须说清是哪一种空**。
 *
 * 几种完全不同的处境曾经长得一模一样，而用户需要做不同的事：
 * 没选目标 → 去选目标；目标进不去 → 看原因或指定目录；真没有 → 换筛选。
 */
export function emptySessionMessage(
  targetId: string,
  target: { available: boolean; detail?: string } | null,
  trash: boolean
): string {
  if (!targetId) return "请先在上面选择目标。";
  if (target && !target.available) return target.detail || "此目标当前不可用。";
  return trash ? "回收站为空。" : "未找到会话。";
}
