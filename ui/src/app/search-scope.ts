import type { AiProviderId, AiTarget } from "../types";

/**
 * 全局检索的范围收窄。
 *
 * # 为什么要求成一个纯函数
 *
 * 收窄范围的价值一半在于**少发请求**：全量搜完再在前端过滤掉，那些请求已经付过代价了。
 * 因此「这次要搜哪些目标」必须是一个可单测的判断，而不是散在渲染里的一段 `filter`。
 *
 * # 两个筛选项
 *
 * 与旧平台 `global-session-search-panel.tsx` 一致：**按平台 + 按目标**。
 * 一个平台常常有多个目标（本机 + 各个 WSL 发行版），先按平台收窄再挑目标最常用。
 *
 * # 平台判定用 `target.provider`，**不解析编号**
 *
 * 靠 `codex:wsl:Ubuntu` 这种前缀来认平台，会在编号格式变化时静默失效——
 * 而 `provider` 本来就是目标自带的字段。
 */
export function scopeTargets(
  targets: AiTarget[],
  providerId: AiProviderId | "",
  targetId: string
): AiTarget[] {
  return targets.filter(
    (target) =>
      (!providerId || target.provider === providerId) &&
      (!targetId || target.id === targetId)
  );
}
