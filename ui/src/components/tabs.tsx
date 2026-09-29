import type { ReactNode } from "react";

/**
 * 工作区的标签条。
 *
 * 会话与终端**共用同一个标签条**——它们都是「我正在看的东西」，分成两套标签条会让人
 * 分不清「哪个在前台」。这是旧平台的形状，也是终端能和工作区共存的前提。
 */

export type TabTone = "normal" | "attention";

export type TabItem = {
  /** 稳定标识；终端用 `terminalId`，会话用 `sessionId`。 */
  key: string;
  label: string;
  /** `attention` 表示这一路在等用户——例如终端弹了确认提示。 */
  tone?: TabTone;
  /** 是否可关闭。会话标签通常不关，终端关掉即停止。 */
  closable?: boolean;
};

export function Tabs({
  items,
  activeKey,
  onActivate,
  onClose,
  onContextMenu,
  trailing
}: {
  items: TabItem[];
  activeKey: string;
  onActivate: (key: string) => void;
  onClose?: (key: string) => void;
  /**
   * 标签右键菜单。
   *
   * 终端标签需要「关闭其他 / 关闭全部」这类批量动作——一个一个点 ✕ 在开了七八路
   * 终端时很难受。会话标签不需要（它跟着选中状态走）。
   */
  onContextMenu?: (key: string, x: number, y: number) => void;
  /** 标签条右端的固定内容，例如「新建终端」。 */
  trailing?: ReactNode;
}) {
  return (
    <div className="tabs" role="tablist">
      {items.map((item) => {
        const active = item.key === activeKey;
        return (
          <div
            key={item.key}
            role="tab"
            aria-selected={active}
            className={`tab${active ? " on" : ""}${item.tone === "attention" ? " attention" : ""}`}
            onClick={() => onActivate(item.key)}
            onContextMenu={
              onContextMenu
                ? (event) => {
                    event.preventDefault();
                    onContextMenu(item.key, event.clientX, event.clientY);
                  }
                : undefined
            }
            title={item.label}
          >
            {item.tone === "attention" && <span className="tab-dot" aria-label="等待确认" />}
            <span className="tab-label">{item.label}</span>
            {item.closable !== false && onClose && (
              <button
                type="button"
                className="tab-close"
                aria-label={`关闭 ${item.label}`}
                onClick={(event) => {
                  // 点关闭不该顺带把标签切到前台——那是两件事。
                  event.stopPropagation();
                  onClose(item.key);
                }}
              >
                ✕
              </button>
            )}
          </div>
        );
      })}
      {trailing !== undefined && <div className="tabs-trailing">{trailing}</div>}
    </div>
  );
}
