import { useCallback, useEffect, useRef, useState } from "react";

import { Button, Icon, Popover } from "../../../components";
import * as api from "../../../ipc/commands";
import { appendAttachments, attachmentFromPath, type ComposerAttachment } from "./composer-attachments";
import { ComposerImagePreview } from "./composer-image-preview";
import { ModelSelect } from "./model-select";
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
 * # 视觉
 *
 * 它在终端区里，底仍然是终端那一套；但**输入卡本身走界面 token**
 * （`--ui-bg-input` 底、`--ui-border-strong` 边、聚焦时 `--ui-accent` + `--ui-focus` 环）。
 * 此前这里是一整块写死的深色：明亮主题下那是嵌在终端里的一块黑板，
 * 与它右边的工作区完全不是一套语言。
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
  providerId,
  vendorId,
  targetId,
  onSent,
  onError
}: {
  terminalId: string;
  /**
   * 这一路终端的平台 / 供应商 / 目标。
   *
   * 模型选择靠它们决定**有没有清单可列、去哪个接口列**（判据在 `models.ts`）。
   */
  providerId: string;
  vendorId?: string;
  targetId?: string;
  /** 提交成功后的回调：让父组件把焦点还给终端或记住「已提交」。 */
  onSent?: (text: string) => void;
  onError: (message: string) => void;
}) {
  const [text, setText] = useState("");
  const [attachments, setAttachments] = useState<ComposerAttachment[]>([]);
  const [blocks, setBlocks] = useState<PastedBlock[]>([]);
  const [height, setHeight] = useState(DEFAULT_HEIGHT);
  const [sending, setSending] = useState(false);
  /** 正在预览的图片。`null` 表示没开。 */
  const [preview, setPreview] = useState<ComposerAttachment | null>(null);
  /** 正在拖东西进来。用来给整块输入区加一个可见的落点提示。 */
  const [dragging, setDragging] = useState(false);
  /** 附件菜单是否展开。 */
  const [menuOpen, setMenuOpen] = useState(false);
  const areaRef = useRef<HTMLTextAreaElement | null>(null);
  /** 附件菜单的锚点：那个「📎 附件」按钮。`Popover` 靠它算位置、判点击内外。 */
  const menuAnchorRef = useRef<HTMLButtonElement | null>(null);
  /** 附件编号的自增源，只用来做 key。 */
  const nextIdRef = useRef(0);

  const addPaths = useCallback((paths: string[]) => {
    if (paths.length === 0) return;
    setAttachments((current) =>
      appendAttachments(
        current,
        paths.map((path) => attachmentFromPath(path, `a${(nextIdRef.current += 1)}`))
      )
    );
  }, []);

  /**
   * 拖进来的文件。
   *
   * # 为什么听 Tauri 的事件，而不是 HTML5 的 `drop`
   *
   * Electron 会把真实路径挂在 `File` 对象上（`file.path`），旧平台就是这么拿的。
   * **Tauri 的 webview 没有这个属性**——HTML5 的 `dataTransfer.files` 里那些
   * `File` 拿不到路径，而附件交给 CLI 靠的正是路径。
   *
   * Tauri 会把原生拖放转成 `tauri://drag-drop` 事件，载荷里是真实路径。
   * 窗口创建时没有禁用这个处理器（见 `tauri_app.rs` 的 `WebviewWindowBuilder`）。
   */
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void api
      .onDragDrop((paths) => {
        addPaths(paths);
        setDragging(false);
      })
      .then((stop) => {
        if (cancelled) stop();
        else unlisten = stop;
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [addPaths]);

  /** 换终端时把草稿清掉：草稿属于那一路终端，跟着走会写错地方。 */
  useEffect(() => {
    setText("");
    setAttachments([]);
    setBlocks([]);
    setPreview(null);
  }, [terminalId]);

  /**
   * 粘贴图片。
   *
   * webview 在 `paste` 事件里直接给出 `Blob`——**没有路径**。附件最终要以
   * `@路径` 交给 CLI，所以先用 canvas 编成 PNG，再让后端落盘换一个路径回来。
   *
   * 编码放在前端是因为 webview 本来就会（`canvas.toDataURL`），
   * 后端就不必为此引入一个 PNG 编码器。
   */
  const pasteImage = useCallback(
    async (file: File) => {
      try {
        const dataUrl = await blobToPngDataUrl(file);
        const saved = await api.saveImage(dataUrl);
        addPaths([saved.path]);
      } catch (problem) {
        onError(problem instanceof Error ? problem.message : String(problem));
      }
    },
    [addPaths, onError]
  );

  /**
   * 粘贴。
   *
   * **必须接管默认粘贴**：不接管的话，一段几十行的日志会被直接塞进 textarea，
   * 换行原样保留，而用户看不到「它其实会被当成多条消息」。
   * 这里把长内容换成标记，短内容正常插入。
   */
  const onPaste = useCallback(
    (event: React.ClipboardEvent<HTMLTextAreaElement>) => {
      // 图片优先：截图粘贴时 `getData("text")` 也可能是空的，
      // 先判文本会把它当「没有内容」放过去，然后图片被丢掉。
      const image = [...event.clipboardData.files].find((item) =>
        item.type.startsWith("image/")
      );
      if (image) {
        event.preventDefault();
        void pasteImage(image);
        return;
      }
      const pasted = event.clipboardData.getData("text");
      if (!pasted) return;
      // 路径也当成附件处理：从文件管理器复制的路径直接变成 `@路径`。
      if (/^[A-Za-z]:\\|^\/|^~/.test(pasted.trim()) && !pasted.includes("\n")) {
        event.preventDefault();
        addPaths([pasted.trim()]);
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
    [blocks.length, text, addPaths, pasteImage]
  );

  /** 提交。`null` 表示没有可提交的内容——那时**什么都不做**。 */
  const send = useCallback(async () => {
    const payload = composeSubmission(
      text,
      blocks,
      attachments.map((item) => item.path)
    );
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

  /**
   * 「添加图片」与「添加文件」是**两个动作**，因此是两个菜单项而不是一个。
   *
   * 此前只有一个按钮，而它背后的选择框**过滤器写死是 `.md`** —— 图片根本选不中，
   * 界面却写着「添加文件」。合并回去的时候别把这条教训丢了：过滤器由
   * `kind`（`"image"` / `"any"`）决定，选完立刻收起菜单。
   */
  const addImages = useCallback(() => {
    setMenuOpen(false);
    void api
      .chooseFiles("添加图片（可多选）", "image")
      .then((picked) => addPaths(picked))
      .catch((problem: unknown) =>
        onError(problem instanceof Error ? problem.message : String(problem))
      );
  }, [addPaths, onError]);

  const addFiles = useCallback(() => {
    setMenuOpen(false);
    void api
      .chooseFiles("添加文件（可多选）", "any")
      .then((picked) => addPaths(picked))
      .catch((problem: unknown) =>
        onError(problem instanceof Error ? problem.message : String(problem))
      );
  }, [addPaths, onError]);

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
          {attachments.map((item) => (
            <span
              className={`composer-attachment${item.kind === "image" ? " is-image" : ""}`}
              key={item.id}
              title={item.path}
            >
              {item.kind === "image" ? (
                // 图片给一个可见的入口（点开预览）；文件只是一条路径。
                <button
                  type="button"
                  className="composer-attachment-image"
                  onClick={() => setPreview(item)}
                  title={`预览 ${item.path}`}
                >
                  🖼 {item.name}
                </button>
              ) : (
                <span className="composer-attachment-file">@ {item.path}</span>
              )}
              <button
                type="button"
                aria-label={`移除附件 ${item.name}`}
                onClick={() =>
                  setAttachments((current) => current.filter((at) => at.id !== item.id))
                }
              >
                ✕
              </button>
            </span>
          ))}
        </div>
      )}

      {/* 输入卡：底 / 边 / 焦点环都在 `.composer-card` 上，textarea 只是卡里的一块文本。 */}
      <div className="composer-card">
        <textarea
          ref={areaRef}
          className="composer-text"
          style={{ height }}
          value={text}
          placeholder="在这里写完整的提示词，Enter 发送、Shift+Enter 换行。粘贴长内容会自动收成标记，截图可以直接粘进来。"
          onChange={(event) => setText(event.target.value)}
          onPaste={onPaste}
          onKeyDown={onKeyDown}
          // 拖到输入框上方时给一个可见的落点提示。用 `dragover` 而不是 Tauri 的
          // 拖入事件：后者是窗口级的，写在这里会让整块面板一起高亮。
          onDragOver={(event) => {
            event.preventDefault();
            setDragging(true);
          }}
          onDragLeave={() => setDragging(false)}
        />

        <div className={`composer-actions${dragging ? " dragging" : ""}`}>
          {/* 模型选择：这一路没有可列的清单时它自己什么都不渲染（判据在 `models.ts`）。 */}
          <ModelSelect
            terminalId={terminalId}
            providerId={providerId}
            vendorId={vendorId}
            targetId={targetId}
            onSent={onSent}
            onError={onError}
          />

          {/*
            附件入口是**一个按钮带一个下拉**，而不是两个并排的按钮：
            工具行左边现在归模型选择，再加两个按钮会把这一行撑成一排控件。
            两个动作本身没变（见 `addImages` / `addFiles`）。

            用原生 `button` 而不是 `Button`：`Popover` 需要一个能测位置的锚点，
            而组件层的 `Button` 不透传 `ref`。
          */}
          <button
            type="button"
            ref={menuAnchorRef}
            className="btn btn-sm"
            aria-haspopup="dialog"
            aria-expanded={menuOpen}
            title="添加附件：图片或文件，都会以 @路径 的形式附加"
            onClick={() => setMenuOpen((value) => !value)}
          >
            <Icon name="paperclip" size={12} />
            附件
            <Icon name="chevron-down" size={12} />
          </button>
          <Popover
            open={menuOpen}
            onClose={() => setMenuOpen(false)}
            anchorRef={menuAnchorRef}
            className="composer-menu"
            ariaLabel="附件菜单"
          >
            <button
              type="button"
              className="composer-menu-item"
              onClick={addImages}
              title="添加图片：会以 @路径 的形式附加，并在提示词里带上图片"
            >
              <Icon name="image" size={12} />
              添加图片
            </button>
            <button
              type="button"
              className="composer-menu-item"
              onClick={addFiles}
              title="添加文件或文件夹：会以 @路径 的形式附加"
            >
              <Icon name="file" size={12} />
              添加文件
            </button>
          </Popover>

          <span className="row-meta">
            {dragging ? "松手即添加" : blocks.length > 0 ? `已收起 ${blocks.length} 段长内容` : ""}
          </span>
          <span className="spacer" />
          {/* 快捷方式写在按钮旁边：这是这一块最需要被记住的一件事。 */}
          <span className="row-meta">Enter 发送</span>
          <Button
            small
            variant="primary"
            disabled={
              sending ||
              composeSubmission(
                text,
                blocks,
                attachments.map((item) => item.path)
              ) === null
            }
            onClick={() => void send()}
            title="发送 (Enter)"
          >
            {sending ? "发送中…" : "发送"}
          </Button>
        </div>
      </div>

      {preview && (
        <ComposerImagePreview
          attachment={preview}
          onClose={() => setPreview(null)}
          onError={onError}
        />
      )}
    </div>
  );
}

/**
 * 把一张图片 `Blob` 编成 PNG 的 data URL。
 *
 * # 为什么过一遍 canvas
 *
 * 剪贴板里的图片可能是 BMP / TIFF 之类，而**后端只接受 PNG**（落盘的扩展名必须与
 * 内容一致）。canvas 会把任意可以解码的图重新编成 PNG，正好完成这次转换。
 *
 * 只用 width/height 画一遍而不裁剪：这里不改变图的内容，只是换一个容器。
 */
async function blobToPngDataUrl(file: File): Promise<string> {
  const bitmap = await createImageBitmap(file);
  try {
    const canvas = document.createElement("canvas");
    canvas.width = bitmap.width;
    canvas.height = bitmap.height;
    const context = canvas.getContext("2d");
    if (!context) throw new Error("无法创建画布上下文。");
    context.drawImage(bitmap, 0, 0);
    return canvas.toDataURL("image/png");
  } finally {
    // 位图占的是显存/内存，不释放的话连续粘贴几十张会一直涨。
    bitmap.close();
  }
}
