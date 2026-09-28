/**
 * 自管输入的文本处理。
 *
 * # 这些都是纯函数，因为它们的错**看不见**
 *
 * 「自管输入」把用户敲的一整段文字写进 PTY。写进去的字节与 CLI 收到的输入之间
 * 隔着三层处理：附件前缀、粘贴转义、括号粘贴包裹。任何一层错了，用户看到的是
 * 「它把我的话吞了」或者「一句话被拆成了十几条」——而没有任何报错。
 *
 * 因此这里每一步都单独可测，而不是埋在一个 600 行的组件里。
 */

/** 一段附件的替换标记：正文里留下 `[已粘贴内容 #1]`，提交时再展开。 */
export type PastedBlock = { marker: string; text: string };

/** 超过这个长度就收成标记，避免几万字的日志糊满输入框。 */
export const PASTE_COMPACT_THRESHOLD = 1200;

/** 生成一个粘贴标记。序号让同一段内容粘两次也能区分。 */
export function pasteMarker(index: number): string {
  return `[已粘贴内容 #${index + 1}]`;
}

/**
 * 一段文本该不该收成标记。
 *
 * 判据是**长度或行数**：一段 40 行的报错可能只有 800 字符，但它同样会糊满输入框。
 * 只按字符数判断会漏掉这种——而那正是最常被粘贴的东西。
 */
export function shouldCompact(text: string): boolean {
  if (text.length >= PASTE_COMPACT_THRESHOLD) return true;
  return text.split("\n").length > 12;
}

/**
 * 把一段文本插入到光标处。
 *
 * 返回新文本与新的光标位置。**光标位置必须一起返回**：只返回文本的话，
 * 调用方只能把光标丢到末尾，用户接着打字就会跑到整段的最后而不是他插入的位置。
 */
export function insertAtCursor(
  value: string,
  start: number,
  end: number,
  inserted: string
): { value: string; cursor: number } {
  const next = `${value.slice(0, start)}${inserted}${value.slice(end)}`;
  return { value: next, cursor: start + inserted.length };
}

/**
 * 把标记展开回原文。
 *
 * 提交前必须做这一步：标记只是输入框里的显示，写进 PTY 的必须是**完整原文**。
 * 忘了展开的后果很隐蔽——CLI 收到的是字面的「[已粘贴内容 #1]」。
 */
export function expandBlocks(text: string, blocks: PastedBlock[]): string {
  let expanded = text;
  for (const block of blocks) {
    if (expanded.includes(block.marker)) {
      expanded = expanded.split(block.marker).join(block.text);
    }
  }
  return expanded;
}

/**
 * 附件变成 `@路径` 前缀。
 *
 * PTY 只能传文本，因此附件靠**完整路径**让 CLI 自己去读真实文件——
 * 只写文件名的话 CLI 找不到它，用户会以为附件功能坏了。
 *
 * 含空格的路径要加引号：不加的话 CLI 会把一个路径当成两个。
 */
export function formatAttachments(paths: string[]): string {
  if (paths.length === 0) return "";
  return paths
    .map((path) => (/[\s"]/.test(path) ? `"${path.replace(/"/g, '\\"')}"` : path))
    .map((path) => `@${path}`)
    .join("\n");
}

/**
 * 提交时真正写进 PTY 的字节。
 *
 * # 两个必须照抄的细节
 *
 * 1. **控制字符要清掉**。用户粘贴的文本里可能带 `\x1b`（从终端复制输出时很常见），
 *    直接写进 PTY 会被当成转义序列执行——那可能改变终端的显示状态。
 * 2. **括号粘贴标记与回车必须在同一次写入里**。分开写的话，Windows 的输入法组合态下
 *    回车可能只表现为终端换行而没有提交。旧平台为此专门写过注释，这里照抄。
 *
 * 返回 `null` 表示**没有可提交的内容**（全是空白），调用方应当什么都不做——
 * 空提交会往 CLI 里塞一个多余的回车。
 */
export function composeSubmission(
  text: string,
  blocks: PastedBlock[],
  attachmentPaths: string[]
): string | null {
  const expanded = expandBlocks(text, blocks).trim();
  const attachments = formatAttachments(attachmentPaths);
  /**
   * 只在**两边都有**时插换行。
   *
   * 写成 `${attachments}\n${expanded}` 会在「只有附件、没有正文」时留下一个
   * 尾随换行——那个换行会让 CLI 把 `@文件` 当成一条空消息处理。
   * 这一条是测试抓出来的，不是想出来的。
   */
  const parts = [attachments, expanded].filter((part) => part !== "");
  const full = parts.join("\n");
  if (!full.trim()) return null;
  const safe = full.replace(/\x1b/g, "");
  return `\x1b[200~${safe}\x1b[201~\r`;
}

/**
 * 收到**第一段输出**时要不要把输入框露出来。
 *
 * # 这条规则不是可有可无的
 *
 * 旧平台的行为是：`composerVisible` 初值 `= !sessionId`（新会话显示、恢复历史隐藏），
 * 然后**在终端吐出第一段输出时自动显示**（`embedded-terminal.tsx` 的
 * `onTerminalData` 里那句 `revealComposer()`）。
 *
 * 而 `revealComposer()` 是**唯一**的显示入口——没有按钮能把它调出来。
 * 少了这条自动显示，恢复的会话里用户**永远看不到**自管输入。
 */
export function shouldRevealComposer(visible: boolean, hasOutput: boolean): boolean {
  return !visible && hasOutput;
}

/**
 * 初始是否显示。
 *
 * 新会话（`sessionId` 为空）一进去就有；恢复历史会话时先藏着，
 * 等第一段输出再出现——因为恢复时终端正在重放历史，此时输入框会挡住输出。
 */
export function initialComposerVisible(sessionId: string | undefined): boolean {
  return !sessionId;
}
