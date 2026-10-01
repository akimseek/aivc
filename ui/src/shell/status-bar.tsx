import { gatewayPortLabel } from "../app/gateway-address";
import { formatWhen } from "../app/session-display";
import { terminalInputButtonTitle, type TerminalInputMode } from "../features/workspace/composer/composer-text";
import { VendorRoutePicker } from "../features/workbench/vendor-route-picker";
import type { useTerminals } from "../app/use-terminals";
import { Icon, ToolbarSpacer } from "../components";
import * as api from "../ipc/commands";
import type { AiSession, AiTarget, GatewayPortStatus } from "../types";

/**
 * 状态栏（28px）。
 *
 * # 它回答的是「我现在在哪」
 *
 * 只有计数回答不了这个问题——用户切了几个会话之后，最常问的是
 * 「我现在看的是哪一个、在哪个目录、由哪个供应商服务」。因此会话编号、目录、
 * 更新时间**必须常驻**，而不是藏在某个面板里。纯计数（会话数 / 终端数）不再单列：
 * 侧栏摘要与工作台已经覆盖（spec §6）。
 *
 * # 会话编号点击可复制
 *
 * 它是**定位一条会话**最稳的标识（标题会重复、目录会变），用户最常做的事就是
 * 把它粘到别处（issue、聊天、命令行）。此前只能从右键菜单复制。
 *
 * # 供应商那一格非有不可
 *
 * 走网关之后供应商可能被**自动**换掉，用户没有做任何操作。界面不显示，
 * 用户只有在账单上才发现。因此这里放的是可交互的 [`VendorRoutePicker`]（浮层），
 * 不只是文本。
 *
 * # 右端是输入模式
 *
 * 「键盘输入去哪一边」是**当前前台那一路**的属性（状态上收在 `useTerminals`），
 * 放在最右端与「网关」相邻——它们都是「现在这个终端怎么连的」。
 */
export function StatusBar({
  busy,
  searching,
  error,
  session,
  target,
  terminals,
  activeTerminalId,
  gateway,
  inputMode,
  onToggleInputMode,
  onNotify
}: {
  busy: boolean;
  searching: boolean;
  error: string;
  session: AiSession | null;
  /** 当前目标。这里显示名字，不显示数量——数量在侧栏。 */
  target: AiTarget | null;
  /**
   * 终端的全部状态与动作。
   *
   * 用**整个 hook 的返回**：供应商选择器要同时拿「前台那一路终端」「切换」「改模式」
   * 三样东西，拆成三个 prop 只是把同一处的耦合摊开写。
   */
  terminals: ReturnType<typeof useTerminals>;
  activeTerminalId?: string;
  /**
   * 网关状态。还没取到时是 `null`——那时显示「未启用」会撒谎，所以按未取到处理。
   *
   * 用**真实类型**而不是手抄的子集：抄漏字段名的话，「后端发的字段名与界面读的不一致」
   * 这件事会被类型系统放过去，两边都编译得过，运行时那一格永远是错的。
   */
  gateway: GatewayPortStatus | null;
  inputMode: TerminalInputMode;
  onToggleInputMode: () => void;
  /** 一句话反馈（复制会话编号之后说一声）。 */
  onNotify?: (message: string) => void;
}) {
  // 优先显示「正在搜索」：它比「处理中」更具体，而且用户刚敲完键盘，正在等这个。
  const state = searching ? "搜索中…" : busy ? "处理中…" : "就绪";

  /**
   * 网关那一格。
   *
   * 四态而不是两态：**没取到**、**未启用**、**已启用但没在监听**、**正在监听**。
   * 把「没取到」显示成「未启用」，用户会去设置里找一个其实已经开着的网关；
   * 把「开着但没监听」也显示成「未启用」是同一个毛病。
   *
   * 判断逻辑在 `gateway-address.ts`，与工作台共用——两边各写一遍的话，
   * 一处算错只有一格显示不对，很难发现。
   */
  const gatewayText = gatewayPortLabel(gateway);

  /** 当前**前台**那一路终端。供应商选择器与输入模式胶囊都讲的是它。 */
  const activeTerminal = terminals.terminals.find((item) => item.id === activeTerminalId);

  return (
    <footer className="statusbar">
      <span>{state}</span>

      {/* 会话编号点一下复制。粘不出去的信息等于不存在。 */}
      <button
        type="button"
        className="status-session"
        disabled={!session}
        title={session ? "点击复制会话编号" : "当前没有选中的会话"}
        onClick={() => {
          if (!session) return;
          void api
            .copyText(session.id)
            .then(() => onNotify?.(`已复制会话编号 ${session.id}。`))
            .catch(() => undefined);
        }}
      >
        会话 <code>{session?.id ?? "-"}</code>
      </button>

      <span className="status-updated">更新 {formatWhen(session?.updatedAt) || "-"}</span>

      <span className="status-cwd" title={session?.cwd ?? undefined}>
        目录 <b>{session?.cwd ?? "-"}</b>
      </span>

      <span className="status-target" title={target?.label ?? undefined}>
        目标 <b>{target?.label ?? "-"}</b>
      </span>

      {/* 当前终端由谁服务。筛选出**前台那一路**：状态栏讲的是「我在看的这个」，
          把所有终端的路由都列出来会让人分不清哪一个在生效。 */}
      <VendorRoutePicker
        terminal={activeTerminal}
        onSwitch={terminals.switchVendor}
        onSetMode={terminals.setRouteMode}
      />

      <ToolbarSpacer />

      {error && (
        <span className="err" title={error}>
          {error}
        </span>
      )}

      <span className="status-gateway">
        网关 <b>{gatewayText}</b>
      </span>

      {/* 输入模式：写的是**当前在哪**（与输入区那个按钮相反——那个写的是点下去会切到哪），
          因为这一格是状态指示，而输入区那个是动作按钮。 */}
      <button
        type="button"
        className={`input-mode-pill${inputMode === "composer" ? " on" : ""}`}
        onClick={onToggleInputMode}
        disabled={!activeTerminalId}
        title={terminalInputButtonTitle(inputMode)}
      >
        <Icon name={inputMode === "composer" ? "chat" : "keyboard"} size={12} />
        <span>{inputMode === "composer" ? "自管输入" : "终端原生输入"}</span>
      </button>
    </footer>
  );
}
