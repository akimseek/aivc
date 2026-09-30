import { Icon, StatusDot } from "../../components";
import type { AiSession } from "../../types";
import {
  displayTitle,
  formatWhen,
  messageCountLabel,
  pickPreview
} from "../../app/session-display";

/**
 * 会话卡片：三行结构 + 悬停动作。
 *
 * # 为什么与 `SessionRow` 分开写，而不是给行加 variant
 *
 * 新侧栏的每一行要承担三件事（认出是哪条 / 聊到哪了 / 就着它动手），而
 * 老行的 `variant` 只分「有没有勾选框」。继续往那个枚举里加值，会让它同时表达
 * 「哪些按钮该出现」和「元信息写什么」——两件不相关的事混在一个参数里。
 *
 * # 三行分别回答一个问题
 *
 * | 行 | 内容 | 回答 |
 * | --- | --- | --- |
 * | 1 | 状态点 + 勾选框 + 标题 + 时间 | 是哪条 |
 * | 2 | 预览（最后一条可显示的消息） | 聊到哪了 |
 * | 3 | 条数（+ 分支标记）与悬停动作 | 就着它做什么 |
 *
 * 悬停动作**默认透明度 0**（`app.css`），鼠标移上来才出现：三个按钮常驻会让
 * 列表看起来一直在闪。触屏与键盘（`:focus-within`）下常显——藏起来的按钮
 * 点不到也看不到。
 *
 * # 单击打开历史，双击在终端中恢复
 *
 * 单击的语义由上层决定（`onOpen` = 打开/激活标签，不看就只显示历史）；
 * 双击是**显式恢复**的手势，与标签栏的「继续会话」同一个动作。它不放在按钮里
 * 是因为卡片上已经有三个动作了，第四个按钮会把标题挤掉。
 */
export function SessionCard({
  session,
  active,
  checked,
  /**
   * 平台支不支持批量操作。不支持时**不渲染勾选框**——一个勾不出结果的框是误导
   * （与老行同一条规则，见 `sidebar-nav.ts` 的能力矩阵）。
   */
  selectable = true,
  /** 这条会话此刻有终端在跑（卡片左上角那个点会亮起来）。 */
  running = false,
  onOpen,
  onResume,
  onToggle,
  onFavorite,
  onBranch,
  onMenu
}: {
  session: AiSession;
  active: boolean;
  checked: boolean;
  selectable?: boolean;
  running?: boolean;
  onOpen: () => void;
  /** 双击：在终端中恢复。没给（例如这条会话的终端正在跑）时双击什么都不做。 */
  onResume?: () => void;
  onToggle: () => void;
  onFavorite: () => void;
  /** 「消息与分支」：打开详情（消息列表 + 分支关系都在那里）。 */
  onBranch: () => void;
  onMenu: (x: number, y: number) => void;
}) {
  const title = displayTitle(session);
  const favorite = session.metadata?.favorite === true;
  /**
   * 分支标记。
   *
   * # 为什么只标「是分支」而不写数量
   *
   * 列表数据里**没有**「这条会话分出了几支」这个字段——那只有
   * `codex:list-session-children` 按需查得到，而列表不会为每一行再发一次请求。
   * `metadata.branch` 说明的是**这条会话自己是某个分支的产物**（它带父会话编号），
   * 那是一个布尔而不是数量。因此有就标出来、没有就整段省略，**不编数字**。
   */
  const branched = session.metadata?.branch != null;

  return (
    <div
      className={`session-card${active ? " on" : ""}`}
      // 整张卡片是一个可点区域，键盘上必须等价：`role` + `tabIndex` 让它进 Tab
      // 序列，`onKeyDown` 让 Enter / Space 与单击同一个动作（不劫持方向键——
      // 列表里的上下移动留给将来的列表导航，卡片不该先抢）。
      role="button"
      tabIndex={0}
      onClick={onOpen}
      onKeyDown={(event) => {
        if (event.key !== "Enter" && event.key !== " ") return;
        // Space 默认会滚动页面；Enter 在个别控件上会触发默认行为。
        event.preventDefault();
        onOpen();
      }}
      onDoubleClick={onResume}
      onContextMenu={(event) => {
        event.preventDefault();
        onMenu(event.clientX, event.clientY);
      }}
      title={title}
    >
      <div className="session-card-top">
        <StatusDot tone={running ? "ok" : "idle"} pulse={running} size={8} />
        {selectable && (
          <input
            type="checkbox"
            className="session-card-check"
            checked={checked}
            aria-label={`选择 ${title}`}
            // 勾选 / 双击都不该顺带打开会话或恢复终端——那是三件事。
            onClick={(event) => event.stopPropagation()}
            onDoubleClick={(event) => event.stopPropagation()}
            onChange={onToggle}
          />
        )}
        <span className="session-card-title">{title}</span>
        <span className="session-card-time ui-num">{formatWhen(session.updatedAt)}</span>
      </div>
      <div className="session-card-preview">{pickPreview(session)}</div>
      <div className="session-card-bottom">
        <span className="session-card-meta ui-num">
          {/*
            条数按会话自带的 `messageCountIsLowerBound` 说话：列表模式读的是文件头部
            窗口，那个数字是**下界**（见 `session-display.ts`），不能一律当成精确值。
          */}
          {messageCountLabel(session.messageCount, session.messageCountIsLowerBound === true)}
          {branched ? " · 分支" : ""}
        </span>
        <div className="session-card-actions">
          <button
            type="button"
            className={favorite ? "on" : undefined}
            title={favorite ? "取消收藏" : "标记收藏"}
            aria-label={favorite ? "取消收藏" : "标记收藏"}
            // 动作按钮在卡片内部，点它们不该把卡片一起点开（`stopPropagation`）；
            // 双击同理——`dblclick` 是独立事件，光挡 `click` 挡不住它。
            onClick={(event) => {
              event.stopPropagation();
              onFavorite();
            }}
            onDoubleClick={(event) => event.stopPropagation()}
          >
            <Icon name="star" size={13} />
          </button>
          <button
            type="button"
            title="消息与分支"
            aria-label="消息与分支"
            onClick={(event) => {
              event.stopPropagation();
              onBranch();
            }}
            onDoubleClick={(event) => event.stopPropagation()}
          >
            <Icon name="branch" size={13} />
          </button>
          <button
            type="button"
            title="更多"
            aria-label="更多"
            onClick={(event) => {
              // 右键菜单按**鼠标位置**开，所以这里回传的是点击坐标。
              event.stopPropagation();
              onMenu(event.clientX, event.clientY);
            }}
            onDoubleClick={(event) => event.stopPropagation()}
          >
            <Icon name="chevron-down" size={13} />
          </button>
        </div>
      </div>
    </div>
  );
}
