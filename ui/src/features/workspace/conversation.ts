import type { AiMessage, AiProviderId } from "../../types";
import { branchAnchor } from "../../app/branch-anchor";

/** 一条消息在当前页中的位置，用于把分页后的界面位置映射回原始消息位置。 */
export type ConversationMessageEntry = {
  message: AiMessage;
  index: number;
};

/** 一轮会话：一条用户消息及其后续回复。 */
export type ConversationTurn = {
  user?: ConversationMessageEntry;
  replies: ConversationMessageEntry[];
};

/**
 * 会话正文中不展示 CLI 初始化/权限提示等合成用户块。
 * 规则与旧平台保持一致；工具和系统记录仍保留，因为它们是会话过程的一部分。
 */
export function isVisibleConversationMessage(message: AiMessage) {
  if (message.text.includes("<turn_aborted>")) return false;
  const text = message.text.trim();
  if (!text) return false;
  const syntheticPrefixes = [
    "<environment_context>",
    "<skill>",
    "<user_instructions>",
    "<developer_instructions>",
    "You are `/root`, the primary agent in a team of agents collaborating with the user."
  ];
  if (syntheticPrefixes.some((prefix) => text.startsWith(prefix))) return false;
  if (text.includes("<permissions instructions>")) return false;
  return !(
    text.includes("<multi_agent_mode>") &&
    text.includes("At the start of your turn, you are the active agent.")
  );
}

/** 把可见消息按用户消息分成会话轮次，index 保留分页前的绝对位置。 */
export function buildConversationTurns(messages: AiMessage[], offset = 0) {
  const turns: ConversationTurn[] = [];
  let currentTurn: ConversationTurn | null = null;

  messages.forEach((message, index) => {
    if (!isVisibleConversationMessage(message)) return;
    const entry = { message, index: offset + index };
    if (message.role === "user") {
      currentTurn = { user: entry, replies: [] };
      turns.push(currentTurn);
      return;
    }

    if (!currentTurn) {
      currentTurn = { replies: [] };
      turns.push(currentTurn);
    }
    currentTurn.replies.push(entry);
  });

  return turns.filter((turn) => turn.user || turn.replies.length > 0);
}

/**
 * 一轮只允许一个分支入口：放在这一轮最后一条可见消息上，锚点取最后一条助手回复。
 * 没有助手回复的未完成轮次不能分支，避免截在用户输入或工具记录中间。
 */
export function branchAnchorForTurn(provider: AiProviderId, turn: ConversationTurn) {
  const assistant = [...turn.replies].reverse().find((entry) => entry.message.role === "assistant");
  if (!assistant) return undefined;
  return branchAnchor(provider, assistant.message, assistant.index);
}

function branchEntryIndexForTurn(turn: ConversationTurn) {
  return [...turn.replies].reverse().find((entry) => entry.message.role === "assistant")?.index;
}

/** 将一轮展开为消息行，并把唯一的分支入口标在该轮最后一行。 */
export function flattenConversationTurns(
  messages: AiMessage[],
  offset: number,
  provider: AiProviderId,
  lastTurnComplete = true,
  completedBranchAnchors: readonly number[] = []
) {
  const turns = buildConversationTurns(messages, offset);
  return turns.flatMap((turn, turnIndex) => {
    const entries = [
      ...(turn.user ? [turn.user] : []),
      ...turn.replies
    ];
    // If the page ends before the session does, the last visible turn may
    // continue on the next page. Only turns closed by a following user message
    // are safe to branch from here.
    const isIncompleteTail = !lastTurnComplete && turnIndex === turns.length - 1;
    const anchor = isIncompleteTail ? undefined : branchAnchorForTurn(provider, turn);
    const anchorEntryIndex = isIncompleteTail ? undefined : branchEntryIndexForTurn(turn);
    return entries.map((entry) => ({
      entry,
      // The anchor belongs to the final assistant response, even when a
      // trailing tool/system record is present in the same turn.
      branchAnchor: provider === "codex"
        ? entry.message.role === "assistant" && entry.message.sourceLine !== undefined && completedBranchAnchors.includes(entry.message.sourceLine)
          ? entry.message.sourceLine
          : undefined
        : entry.index === anchorEntryIndex ? anchor : undefined
    }));
  });
}
