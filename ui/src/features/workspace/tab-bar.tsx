import { Badge, Icon, StatusDot, type StatusDotTone } from "../../components";
import type { WorkspaceTabItem } from "../../app/use-workspace-tabs";

/**
 * 工作区标签栏（现代流体标签）。
 *
 * # 一张标签回答三个问题
 *
 * | 元素 | 回答 |
 * | --- | --- |
 * | 状态点 | 这一路还活着吗（跑着 / 空闲 / 异常退出 / 在等确认） |
 * | 标题 + 后缀 | 这是哪一条（`（退出码 137）` 这种后缀由 `tabSuffix` 给定） |
 * | 平台徽章 | 它由哪个平台服务 |
 *
 * 关闭按钮**悬停才显形**：常驻的 ✕ 会让标签行看起来一直在闪（标签本来就多）；
 * 触屏（`@media (hover: none)`）没有悬停，因此常显——藏起来的按钮永远点不到。
 *
 * # 右端的三个动作
 *
 * `＋`（新建会话）、查找（当前终端的 Ctrl+F，见 `TerminalView` 的转发）、
 * 「消息与分支」（会话详情：消息列表 + 分支关系）。详情入口放在这一行而不是正文头部，
 * 是因为**进了终端之后正文就不渲染了**，放在正文里等于「进了终端就再也看不到详情」。
 */
export function TabBar({
  items,
  activeKey,
  /** 「消息与分支」上的条数；`null` 表示还不知道（不渲染徽章，不编数字）。 */
  messageCount,
  /** 这一刻能不能新建会话（目标可用才有意义，判据在 `app.tsx`）。 */
  canCreate = true,
  /**
   * 平台编号 → 展示名的查询。
   *
   * 名字的真源在后端（`ai:list-providers` 的 `label`），组件本身不存映射表；
   * 查询不到时退回编号本身（`codex` 这类短名至少认得出是哪个平台）。
   */
  providerLabel,
  onActivate,
  onClose,
  onContextMenu,
  onNewSession,
  onFind,
  onOpenMessages
}: {
  items: WorkspaceTabItem[];
  activeKey: string;
  messageCount: number | null;
  canCreate?: boolean;
  providerLabel?: (providerId: string) => string;
  onActivate: (key: string) => void;
  onClose?: (key: string) => void;
  onContextMenu?: (key: string, x: number, y: number) => void;
  onNewSession: () => void;
  /**
   * 查找：打开当前终端的查找栏。
   *
   * 当前标签**没有终端**时不传（那时画布是历史视图，没有可查找的缓冲区）——
   * 按钮显示为禁用，而不是「点得动但什么都不发生」。
   */
  onFind?: () => void;
  /** 没有会话可看时不传，按钮显示为禁用（而不是消失，位置会跳）。 */
  onOpenMessages?: () => void;
}) {
  return (
    <div className="tab-bar">
      {/* 横向滚动只加在标签组上：把动作按钮一起放进滚动区，标签一多它们就被推出屏幕。 */}
      <div className="tab-strip" role="tablist" aria-label="工作区标签">
        {items.map((item) => {
          const active = item.key === activeKey;
          const attention = item.tone === "attention";
          return (
            <div
              key={item.key}
              role="tab"
              aria-selected={active}
              className={`tab-pill${active ? " on" : ""}`}
              onClick={() => onActivate(item.key)}
              onContextMenu={
                onContextMenu
                  ? (event) => {
                      event.preventDefault();
                      onContextMenu(item.key, event.clientX, event.clientY);
                    }
                  : undefined
              }
              // 状态点对读屏不可见（是装饰），因此「在等确认」这一类信息写进标题里。
              title={
                attention
                  ? `${item.label}${item.suffix}（等待确认）`
                  : `${item.label}${item.suffix}`
              }
            >
              <StatusDot
                tone={dotTone(item)}
                // 「跑着」才脉动：它是唯一一种**此刻在进行**的状态。
                pulse={item.state === "running"}
                size={7}
                title={attention ? "等待确认" : undefined}
              />
              <span className="tab-pill-label">{item.label}</span>
              {item.suffix && <span className="tab-pill-suffix">{item.suffix}</span>}
              {item.providerId && (
                <Badge tone="neutral">{platformText(item.providerId, providerLabel)}</Badge>
              )}
              {onClose && (
                <button
                  type="button"
                  className="tab-pill-close"
                  aria-label={`关闭 ${item.label}`}
                  title="关闭"
                  onClick={(event) => {
                    // 点关闭不该顺带把标签切到前台——那是两件事。
                    event.stopPropagation();
                    onClose(item.key);
                  }}
                >
                  <Icon name="close" size={12} />
                </button>
              )}
            </div>
          );
        })}
      </div>

      <div className="tab-bar-actions">
        <button
          type="button"
          className="tab-bar-action"
          title="新建会话"
          aria-label="新建会话"
          disabled={!canCreate}
          onClick={onNewSession}
        >
          <Icon name="plus" size={15} />
        </button>
        <button
          type="button"
          className="tab-bar-action"
          title={onFind ? "在当前终端中查找 (Ctrl+F)" : "当前标签没有终端"}
          aria-label="查找"
          disabled={!onFind}
          onClick={onFind}
        >
          <Icon name="search" size={15} />
        </button>
        <button
          type="button"
          className="tab-bar-action"
          title={onOpenMessages ? "查看这条会话的消息与分支" : "当前标签没有关联会话"}
          disabled={!onOpenMessages}
          onClick={onOpenMessages}
        >
          <Icon name="branch" size={15} />
          <span>消息与分支</span>
          {messageCount !== null && (
            <Badge tone="neutral" mono>
              {messageCount}
            </Badge>
          )}
        </button>
      </div>
    </div>
  );
}

/** 状态点：等确认优先于运行中（它是更需要用户看一眼的那一个）。 */
function dotTone(item: WorkspaceTabItem): StatusDotTone {
  if (item.tone === "attention") return "warn";
  if (item.state === "running") return "ok";
  // 异常退出才留在标签栏上（干净退出会被自动收掉），因此这里给 danger。
  if (item.state === "exited") return "danger";
  return "idle";
}

/**
 * 徽章文案：平台 label 的**首段**。
 *
 * 后端给的是 `Codex` / `Claude Code` / `Qoder CN` 这样的全称，而徽章挤在标题后面，
 * 写全称会把标题挤没——首段足够区分这四个平台。
 */
function platformText(providerId: string, lookup?: (id: string) => string): string {
  const label = lookup?.(providerId)?.trim() || providerId;
  return label.split(/\s+/)[0] || label;
}
