import { ToolbarSpacer } from "../components";
import type { AiSession, GatewayPortStatus } from "../types";
import { gatewayPortLabel } from "./gateway-address";
import { formatWhen } from "./session-display";
import { VendorRoutePicker } from "./vendor-route-picker";
import type { useTerminals } from "./use-terminals";

/**
 * 状态栏。
 *
 * # 它回答的是「我现在在哪」
 *
 * 只有计数（目标 / 会话 / 终端）回答不了这个问题——用户切了几个会话之后，最常问的是
 * 「我现在看的是哪一个、在哪个目录、由哪个供应商服务」。因此会话编号、目录、
 * 更新时间**必须常驻**，而不是藏在某个面板里。
 *
 * # 供应商那一格非有不可
 *
 * 走网关之后供应商可能被**自动**换掉，用户没有做任何操作。界面不显示，
 * 用户只有在账单上才发现。因此这里放的是可交互的 [`VendorRoutePicker`]，
 * 不只是文本。
 *
 * # 抽出来的理由
 *
 * 它在 `app.tsx` 里是一段 100 行的内联 JSX，读起来像「界面的一部分」，
 * 但它其实是**只读呈现**：给什么显示什么，没有任何自己的状态。
 * 这种组件最该独立，因为它的正确性可以逐字断言。
 */
export function StatusBar({
  busy,
  searching,
  session,
  targetCount,
  sessionCount,
  terminalCount,
  activeTerminalId,
  terminals,
  error,
  gateway
}: {
  busy: boolean;
  searching: boolean;
  session: AiSession | null;
  targetCount: number;
  sessionCount: number;
  terminalCount: number;
  activeTerminalId?: string;
  terminals: ReturnType<typeof useTerminals>;
  error: string;
  /**
   * 网关状态。还没取到时是 `null`——那时显示「未启用」会撒谎，所以按未取到处理。
   *
   * 用**真实类型**而不是手抄的子集：抄漏字段名的话，「后端发的字段名与界面读的不一致」
   * 这件事会被类型系统放过去，两边都编译得过，运行时那一格永远是错的。
   */
  gateway: GatewayPortStatus | null;
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

  return (
    <div className="statusbar">
      <span>{state}</span>

      <span className="status-session">
        会话{" "}
        <code title={session?.id ?? undefined}>{session?.id ?? "-"}</code>
      </span>

      <span className="status-cwd" title={session?.cwd ?? undefined}>
        目录 <b>{session?.cwd ?? "-"}</b>
      </span>

      <span className="status-updated">更新 {formatWhen(session?.updatedAt) || "-"}</span>

      <span className="status-target">
        目标 <b>{targetCount}</b>
      </span>
      <span className="status-sessions">
        会话 <b>{sessionCount}</b>
      </span>
      <span className="status-terminals">
        终端 <b>{terminalCount}</b>
      </span>

      {/* 当前终端由谁服务。筛选出**前台那一路**：状态栏讲的是「我在看的这个」，
          把所有终端的路由都列出来会让人分不清哪一个在生效。 */}
      <VendorRoutePicker
        terminal={terminals.terminals.find((item) => item.id === activeTerminalId)}
        onSwitch={terminals.switchVendor}
        onSetMode={terminals.setRouteMode}
      />

      <ToolbarSpacer />

      {error && <span className="err">{error}</span>}

      <span>
        网关 <b>{gatewayText}</b>
      </span>
    </div>
  );
}
