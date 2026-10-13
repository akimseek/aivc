import { memo } from "react";

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
 * # 单击进入终端，详情走「消息与分支」
 *
 * 单击的语义由上层决定：**进入这条会话的终端**（在跑切过去、没跑起 `--resume`，
 * 见 `app.tsx` 的 `openSessionById`）。卡片本身不再绑双击——双击就是两次单击，
 * 第二次由 app 层的在途守卫收敛成「只切选中」。详情走下面那枚
 * 「消息与分支」按钮，不占单击。
 *
 * # 为什么 `memo`，以及回调为什么要「回传会话」
 *
 * 会话多到 300+ 条时，父层任何一次与卡片无关的状态变化（搜索框每敲一个字、
 * 勾掉一条）都会让整列重渲——那是肉眼可见的顿。`memo` 要生效，回调就必须是
 * **稳定引用**：卡片不收「已经绑好某条会话的闭包」，而是收「同一个函数」并把它
 * 自己这条 `session` 回传给调用方（按 id 的事件委托）。父层对全部卡片传同一份
 * `useCallback`，`session` 对象在列表刷新之间保持同一引用，浅比较才通得过。
 */
export const SessionCard = memo(function SessionCard({
  session,
  active,
  checked,
  /**
   * 平台支不支持批量操作。不支持时**不渲染勾选框**——一个勾不出结果的框是误导
   * （与老行同一条规则，见 `provider-capabilities.ts` 的能力矩阵）。
   */
  selectable = true,
  /** 这条会话此刻有终端在跑（卡片左上角那个点会亮起来）。 */
  running = false,
  onOpen,
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
  /** 单击：进入这条会话的终端（切过去或起 `--resume`）。会话由卡片回传（见上面的说明）。 */
  onOpen: (session: AiSession) => void;
  onToggle: (session: AiSession) => void;
  onFavorite: (session: AiSession) => void;
  /** 「消息与分支」：打开详情（消息列表 + 分支关系都在那里）。 */
  onBranch: (session: AiSession) => void;
  onMenu: (session: AiSession, x: number, y: number) => void;
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
      onClick={() => onOpen(session)}
      onKeyDown={(event) => {
        // 焦点在**卡内的控件**上时整个让给它：勾选框与动作按钮自己就认 Enter / Space。
        // 不让的话会同时发生两件坏事——`onOpen` 被多调一次（想勾一条却把会话开了），
        // 而且 `preventDefault()` 会把内层控件自己的激活一并取消（按钮按不动）。
        // 「点勾选框/动作按钮不打开会话」这条契约必须在**键盘路径**上同样成立。
        if (event.target !== event.currentTarget) return;
        if (event.key !== "Enter" && event.key !== " ") return;
        // Space 默认会滚动页面；Enter 在个别控件上会触发默认行为。
        event.preventDefault();
        onOpen(session);
      }}
      onContextMenu={(event) => {
        event.preventDefault();
        onMenu(session, event.clientX, event.clientY);
      }}
      title={title}
    >
      <div className="session-card-top">
        <StatusDot tone={running ? "ok" : "idle"} size={8} />
        {selectable && (
          <input
            type="checkbox"
            className="session-card-check"
            checked={checked}
            aria-label={`选择 ${title}`}
            // 勾选不该顺带打开会话——那是两件事。
            onClick={(event) => event.stopPropagation()}
            onChange={() => onToggle(session)}
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
            // 动作按钮在卡片内部，点它们不该把卡片一起点开（`stopPropagation`）。
            onClick={(event) => {
              event.stopPropagation();
              onFavorite(session);
            }}
          >
            <Icon name="star" size={13} />
          </button>
          <button
            type="button"
            title="消息与分支"
            aria-label="消息与分支"
            onClick={(event) => {
              event.stopPropagation();
              onBranch(session);
            }}
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
              onMenu(session, event.clientX, event.clientY);
            }}
          >
            <Icon name="chevron-down" size={13} />
          </button>
        </div>
      </div>
    </div>
  );
});
