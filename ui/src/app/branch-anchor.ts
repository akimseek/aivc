import type { AiMessage, AiProviderId } from "../types";

/** Codex 按日志行定位轮次；Claude/Gemini 按去重后的绝对消息序号定位。 */
export function branchAnchor(provider: AiProviderId, message: AiMessage, messageOffset: number) {
  if (provider === "qoder") return undefined;
  if (provider === "codex") return message.sourceLine;
  return messageOffset + 1;
}
