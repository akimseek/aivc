import { useCallback, useEffect, useRef, useState } from "react";

import { Button } from "../components";
import * as api from "../ipc/commands";
import {
  composeSubmission,
  insertAtCursor,
  pasteMarker,
  shouldCompact,
  type PastedBlock
} from "./composer-text";

/**
 * 终端下方的**自管输入**。
 *
 * # 它解决什么
 *
 * CLI 是个 TUI：长提示词在它自己的输入框里编辑很难受，贴一段日志更是灾难
 * （换行会被当成提交）。这个框让用户**在自己的界面里**写完整段话，
 * 一次写进去。
 *
 * # 关键行为都照抄旧平台，且逐条有测试
 *
 * | 行为 | 为什么不能省 |
 * | --- | --- |
 * | 提交时括号粘贴与回车**同写** | 分开写在 Windows 输入法组合态下会只换行不提交 |
 * | 粘贴超长内容收成 `[已粘贴内容 #N]` | 几万字的日志糊满输入框就没法编辑了 |
 * | 提交时把标记**展开**回原文 | 忘了展开，CLI 收到的是字面的标记 |
 * | 清掉 `\x1b` | 粘贴终端输出时常带转义符，会被当转义序列执行 |
 * | 附件用 `@完整路径` | PTY 只能传文本，靠路径让 CLI 自己去读真实文件 |
 * | 空白内容**不提交** | 空提交会往 CLI 里塞一个多余的回车 |
 *
 * # 高度可拖
 *
 * 长提示词的编辑需要空间，而固定高度要么太矮（写不下）要么太高（挡住终端）。
 * 拖动手柄的高度记在组件状态里，不持久化——它是**这一次**的临时偏好。
 */

/** 默认高度与上下限。太矮放不下三行，太高会把终端挤没。 */
const DEFAULT_HEIGHT = 96;
const MIN_HEIGHT = 56;
const MAX_HEIGHT = 420;

export function ComposerInput({
  terminalId,
  onSent,
  onError
}: {
  terminalId: string;
  /** 提交成功后的回调：让父组件把焦点还给终端或记住「已提交」。 */
  onSent?: (text: string) => void;
  onError: (message: string) => void;
}) {
  const [text, setText] = useState("");
  const [attachments, setAttachments] = useState<string[]>([]);
  const [blocks, setBlocks] = useState<PastedBlock[]>([]);
  const [height, setHeight] = useState(DEFAULT_HEIGHT);
  const [sending, setSending] = useState(false);
  const areaRef = useRef<HTMLTextAreaElement | null>(null);

  /** 换终端时把草稿清掉：草稿属于那一路终端，跟着走会写错地方。 */
  useEffect(() => {
    setText("");
    setAttachments([]);
    setBlocks([]);
  }, [terminalId]);

  /**
   * 粘贴。
   *
   * **必须接管默认粘贴**：不接管的话，一段几十行的日志会被直接塞进 textarea，
   * 换行原样保留，而用户看不到「它其实会被当成多条消息」。
   * 这里把长内容换成标记，短内容正常插入。
   */
  const onPaste = useCallback(
    (event: React.ClipboardEvent<HTMLTextAreaElement>) => {
      const pasted = event.clipboardData.getData("text");
      if (!pasted) return;
      // 路径也当成附件处理：从文件管理器复制的路径直接变成 `@路径`。
      if (/^[A-Za-z]:\\|^\/|^~/.test(pasted.trim()) && !pasted.includes("\n")) {
        event.preventDefault();
        setAttachments((current) => [...current, pasted.trim()]);
        return;
      }
      if (!shouldCompact(pasted)) return;
      event.preventDefault();
      const marker = pasteMarker(blocks.length);
      setBlocks((current) => [...current, { marker, text: pasted }]);
      const area = areaRef.current;
      if (!area) {
        setText((current) => current + marker);
        return;
      }
      const { value, cursor } = insertAtCursor(
        text,
        area.selectionStart,
        area.selectionEnd,
        marker
      );
      setText(value);
      // 光标要落到标记之后，否则用户接着打字会插到标记中间。
      window.requestAnimationFrame(() => {
        area.setSelectionRange(cursor, cursor);
      });
    },
    [blocks.length, text]
  );

  /** 提交。`null` 表示没有可提交的内容——那时**什么都不做**。 */
  const send = useCallback(async () => {
    const payload = composeSubmission(text, blocks, attachments);
    if (!payload) return;
    setSending(true);
    try {
      await api.writeTerminal(terminalId, payload);
      setText("");
      setAttachments([]);
      setBlocks([]);
      onSent?.(text);
    } catch (problem) {
      onError(problem instanceof Error ? problem.message : String(problem));
    } finally {
      setSending(false);
    }
  }, [text, blocks, attachments, terminalId, onSent, onError]);

  /**
   * Enter 提交、Shift+Enter 换行。
   *
   * 反过来（Enter 换行）会让「写完按回车」这个最自然的动作失效，而绝大多数
   * 聊天式界面都是这个约定。
   */
  const onKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (event.key === "Enter" && !event.shiftKey) {
        event.preventDefault();
        void send();
      }
    },
    [send]
  );

  /** 拖动手柄调整高度。`pointermove` 挂在 window 上，鼠标移出面板也不会断。 */
  const startResize = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      event.preventDefault();
      const startY = event.clientY;
      const startHeight = height;
      const onMove = (move: PointerEvent) => {
        const next = startHeight + (startY - move.clientY);
        setHeight(Math.min(MAX_HEIGHT, Math.max(MIN_HEIGHT, next)));
      };
      const onUp = () => {
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
      };
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
    },
    [height]
  );

  return (
    <div className="composer">
      <div
        className="composer-resize"
        role="separator"
        aria-label="拖动调整输入框高度"
        title="拖动调整输入框高度"
        onPointerDown={startResize}
      />

      {attachments.length > 0 && (
        <div className="composer-attachments">
          {attachments.map((path, index) => (
            <span className="composer-attachment" key={`${path}:${index}`} title={path}>
              <span className="composer-attachment-file">@ {path}</span>
              <button
                type="button"
                aria-label="移除附件"
                onClick={() =>
                  setAttachments((current) => current.filter((_, at) => at !== index))
                }
              >
                ✕
              </button>
            </span>
          ))}
        </div>
      )}

      <textarea
        ref={areaRef}
        className="composer-text"
        style={{ height }}
        value={text}
        placeholder="在这里写完整的提示词，Enter 发送、Shift+Enter 换行。粘贴长内容会自动收成标记。"
        onChange={(event) => setText(event.target.value)}
        onPaste={onPaste}
        onKeyDown={onKeyDown}
      />

      <div className="composer-actions">
        <Button
          small
          onClick={() =>
            void api.chooseFile("添加文件或文件夹").then((picked: string | null) => {
              if (picked) setAttachments((current) => [...current, picked]);
            })
          }
          title="添加文件或文件夹：会以 @路径 的形式附加"
        >
          添加文件
        </Button>
        <span className="row-meta">
          {blocks.length > 0 && `已收起 ${blocks.length} 段长内容`}
        </span>
        <span className="spacer" />
        <Button
          small
          variant="primary"
          disabled={sending || composeSubmission(text, blocks, attachments) === null}
          onClick={() => void send()}
          title="发送 (Enter)"
        >
          {sending ? "发送中…" : "发送"}
        </Button>
      </div>
    </div>
  );
}
