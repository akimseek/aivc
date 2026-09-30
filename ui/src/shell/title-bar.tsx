import { themePreferenceLabel, type ResolvedTheme, type ThemePreference } from "../app/theme";
import { Badge, Icon } from "../components";
import { AttentionCenter, type AttentionEntry } from "../features/attention/attention-center";

/**
 * 标题栏（40px）。
 *
 * # 三段各司其职
 *
 * | 段 | 内容 | 为什么在标题栏 |
 * | --- | --- | --- |
 * | 左 | 品牌 + 版本徽章 | 「这是什么软件、哪个版本」每次报问题时都要看 |
 * | 中 | 拖拽留白 | 窗口是无边框的，没有它用户挪不动窗口（见下面的拖拽说明） |
 * | 右 | 主题切换 + 提醒铃铛 + 窗口控制 | 前三者都是**全局**的；窗口控制则必须在这一条上——系统框已经没有了 |
 *
 * # 为什么它同时接 `preference` 与 `resolved`
 *
 * 按钮上写的字是**用户选的偏好**（「跟随系统」是三个状态之一，它本身不是一个主题），
 * 而图标画的是**此刻实际生效的**明暗——`system` 下用户需要知道现在到底是哪个。
 * 只给其中一个都会让另一处说不清。
 *
 * 版本号的来源是这个 prop（`app.tsx` 挂载时取一次）：版本是进程级常量，
 * 每个用到它的地方各取一次只是多几次 IPC，还会让「读不到」的兜底分散。
 *
 * # 窗口动作为什么走命令而不是 `-webkit-app-region` / `window.__TAURI__`
 *
 * 本工程的前端只允许经**单一命令面**（`aivc_invoke`）与后端通信（见 `ipc/client.ts`），
 * 直调 `window.__TAURI__` 会绕开这层契约；而 `-webkit-app-region: drag` 在各版本
 * WebView2 上的行为并不一致，且它一旦生效就会**吃掉** mousedown，拖拽与关闭按钮
 * 之外的交互都跟着失效。因此动作一律走命令，事件一律在 JS 里判断。
 */
export function TitleBar({
  version,
  preference,
  resolved,
  onCycleTheme,
  attention,
  onSelectAttention,
  onDismissAttention,
  onMinimize,
  onToggleMaximize,
  onStartDrag,
  onClose
}: {
  version: string;
  preference: ThemePreference;
  resolved: ResolvedTheme;
  /** 周而复始：明亮 → 暗黑 → 跟随系统。 */
  onCycleTheme: () => void;
  /** 会话提醒（形状与 `AttentionCenter` 的 `entries` 一致）。 */
  attention: AttentionEntry[];
  onSelectAttention: (terminalId: string) => void;
  onDismissAttention: (terminalId: string) => void;
  onMinimize: () => void;
  onToggleMaximize: () => void;
  /** 开始拖动窗口（后端接管后续鼠标移动）。 */
  onStartDrag: () => void;
  onClose: () => void;
}) {
  return (
    <header className="title-bar">
      <div className="title-brand">
        <span className="title-logo" aria-hidden="true">
          AI
        </span>
        <span className="title-name">AI Visual Console</span>
        <Badge tone="accent" mono>
          v{version || "…"}
        </Badge>
      </div>

      {/* 拖拽留白。这一层必须真的在 DOM 里：靠 CSS 给别的元素加拖拽面会让
          按钮、徽章这些子元素一起变成拖拽面，点不动。

          只看 `detail === 1`：双击的**第二次** mousedown 也带左键，若不排掉，
          双击最大化会先被拖拽吃掉。 */}
      <div
        className="title-drag"
        onMouseDown={(event) => {
          if (event.button === 0 && event.detail === 1) onStartDrag();
        }}
        onDoubleClick={onToggleMaximize}
      />

      <div className="title-actions">
        <button
          type="button"
          className="title-theme"
          title="切换明亮 / 暗黑 / 跟随系统"
          aria-label={`当前主题：${themePreferenceLabel(preference)}，点击切换`}
          onClick={onCycleTheme}
        >
          <Icon name={resolved === "dark" ? "moon" : "sun"} size={14} />
          <span>{themePreferenceLabel(preference)}</span>
        </button>

        <AttentionCenter
          entries={attention}
          onSelect={onSelectAttention}
          onDismiss={onDismissAttention}
        />
      </div>

      {/* 窗口控制：无边框之后系统框不再提供这三个按钮，标题栏必须补齐。
          顺序固定为最小化 / 最大化 / 关闭——与 Windows 一致，肌肉记忆不该被改。 */}
      <div className="title-window">
        <button
          type="button"
          className="title-window-btn"
          aria-label="最小化窗口"
          title="最小化"
          onClick={onMinimize}
        >
          <Icon name="minus" size={14} />
        </button>
        <button
          type="button"
          className="title-window-btn"
          aria-label="最大化窗口"
          title="最大化 / 还原"
          onClick={onToggleMaximize}
        >
          <Icon name="square" size={13} />
        </button>
        <button
          type="button"
          className="title-window-btn title-window-btn--close"
          aria-label="关闭窗口"
          title="关闭"
          onClick={onClose}
        >
          <Icon name="close" size={14} />
        </button>
      </div>
    </header>
  );
}
