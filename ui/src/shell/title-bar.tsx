import type { CSSProperties } from "react";

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
 * | 中 | 拖拽留白 | Windows 下无这一块就拖不动窗口（见 `-webkit-app-region`） |
 * | 右 | 主题切换 + 提醒铃铛 | 两者都是**全局**的，与当前看哪条会话无关 |
 *
 * # 为什么它同时接 `preference` 与 `resolved`
 *
 * 按钮上写的字是**用户选的偏好**（「跟随系统」是三个状态之一，它本身不是一个主题），
 * 而图标画的是**此刻实际生效的**明暗——`system` 下用户需要知道现在到底是哪个。
 * 只给其中一个都会让另一处说不清。
 *
 * 版本号的来源是这个 prop（`app.tsx` 挂载时取一次）：版本是进程级常量，
 * 每个用到它的地方各取一次只是多几次 IPC，还会让「读不到」的兜底分散。
 */
export function TitleBar({
  version,
  preference,
  resolved,
  onCycleTheme,
  attention,
  onSelectAttention,
  onDismissAttention
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

      {/* 拖拽留白。这一层必须真的在 DOM 里：靠 CSS 给别的元素加 drag 区域会让
          按钮、徽章这些子元素一起变成拖拽面，点不动。 */}
      <div className="title-drag" style={{ WebkitAppRegion: "drag" } as CSSProperties} />

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
    </header>
  );
}
