import { lazy, Suspense } from "react";

import type { AiSession, AiTarget, ApiVendor, GatewayPortStatus } from "../../types";
import { displayTitle } from "../../app/session-display";
import type { TerminalUiState, TerminalSession } from "../../app/use-terminals";
import type { TerminalInputMode } from "../../app/composer-text";

/**
 * 终端**按需加载**。
 *
 * xterm 大约 340 KB，而它只在用户真的打开终端时才需要。静态引入会让每一个用户
 * 在首屏就为它付出代价——实测把主包从 235 KB 顶到 573 KB。
 */
const TerminalPane = lazy(() =>
  import("../../app/terminal-pane").then((module) => ({ default: module.TerminalPane }))
);

/**
 * 终端视图：会话信息条 + 终端栈。
 *
 * # 所有终端**同时挂载**，非前台的只隐藏不卸载
 *
 * 卸载会让 PTY 的输出无人接收而丢失、xterm 的缓冲也会没——用户切回来会发现
 * 中间断了一截。因此这一层永远渲染**全部**终端，只有 `.terminal-slot.inactive`
 * （`visibility: hidden`）区分前台。这也解释了它为什么不能按「当前 stage 是不是终端」
 * 来挂载：切到历史视图时它必须继续活着（`hidden` 只是把整块收起来）。
 *
 * # 搜索条不在这里
 *
 * 查找栏（Ctrl+F）与它的 `readLines` / 滚动选中都**留在 `TerminalPane` 里**：
 * 它读的是 xterm 的缓冲区，只有那一层拿得到。这里只负责把「用户点了查找」这个
 * 计数转发下去（`findSignal`），前台那一路会自己把它翻译成「打开查找栏」。
 */
export function TerminalView({
  terminals,
  /** 前台那一路的编号；空串表示此刻没有前台（例如已切到历史视图）。 */
  activeTerminalId,
  /** 非终端阶段：整块收起来，但**不卸载**（见文件头）。 */
  hidden,
  session,
  target,
  gateway,
  vendors,
  uiStates,
  onInputModeChange,
  onOutput,
  registerSink,
  unregisterSink,
  onError,
  resolvedTheme,
  findSignal
}: {
  terminals: TerminalSession[];
  activeTerminalId: string;
  hidden: boolean;
  session: AiSession | null;
  target: AiTarget | null;
  /** 网关状态。还没取到时是 `null`——那一项就不渲染（不写「未知」）。 */
  gateway: GatewayPortStatus | null;
  /** 供应商清单：把 `vendor-3f2a` 这样的编号翻成名字。还没取到就是空数组。 */
  vendors: ApiVendor[];
  uiStates: Record<string, TerminalUiState>;
  onInputModeChange: (terminalId: string, mode: TerminalInputMode) => void;
  onOutput: (terminalId: string) => void;
  registerSink: (id: string, sink: (data: Uint8Array) => void) => void;
  unregisterSink: (id: string) => void;
  onError: (message: string) => void;
  resolvedTheme: "light" | "dark";
  /**
   * 递增即「打开查找栏」。计数器而不是事件总线：两个入口（按钮与 Ctrl+F）落在同一处。
   * **只有变化才算一次请求**——挂载时拿到的那个数不算，否则新建的终端会自弹查找栏（见 `TerminalPane`）。
   */
  findSignal: number;
}) {
  const activeTerminal = terminals.find((item) => item.id === activeTerminalId);

  /**
   * 信息条上的每一项**数据不可得就不渲染**。
   *
   * 它是常驻的一行，宁可短一点也不要出现「目标 -」这种占位：那既没信息，
   * 又让人怀疑是不是加载失败了。
   */
  const title = session ? displayTitle(session) : activeTerminal?.title;
  const shortId = (session?.id ?? activeTerminal?.id)?.slice(0, 8);
  const vendor = activeTerminal?.vendorId
    ? `${vendorName(vendors, activeTerminal.vendorId)}（${
        activeTerminal.vendorMode === "locked" ? "锁定" : "动态"
      }）`
    : // 没有供应商编号 = 这一路没走网关，CLI 用自己的凭据直连上游（见 `use-terminals`）。
      "直连（用自己的密钥）";
  const gatewayUrl = gateway?.baseUrl?.trim();

  return (
    <div className={`terminal-view${hidden ? " hidden" : ""}`}>
      <div className="terminal-info-strip">
        {title && (
          <span className="strip-title" title={title}>
            {title}
          </span>
        )}
        {shortId && <span className="strip-id ui-mono">{shortId}</span>}
        {target && (
          <span className="strip-item">
            目标 <b>{target.label}</b>
          </span>
        )}
        {vendor && (
          <span className="strip-item">
            供应商 <b>{vendor}</b>
          </span>
        )}
        {gatewayUrl && (
          <span className="strip-item">
            网关 <b className="ui-mono">{gatewayUrl}</b>
          </span>
        )}
      </div>

      <div className="terminal-stack">
        <Suspense fallback={<div className="terminal-loading">正在加载终端…</div>}>
          {terminals.map((item) => {
            const ui = uiStates[item.id];
            return (
              <TerminalPane
                key={item.id}
                terminalId={item.id}
                active={item.id === activeTerminalId}
                running={item.running}
                /*
                  编号还没被登记时（启动中）按「自管输入、先藏着」显示：
                  藏着的输入框不会挡住终端正在回放的内容，等 `noteOutput`
                  按 `initialComposerVisible` 的规则把它放出来。
                */
                inputMode={ui?.inputMode ?? "composer"}
                composerVisible={ui?.composerVisible ?? false}
                onInputModeChange={(mode) => onInputModeChange(item.id, mode)}
                onOutput={() => onOutput(item.id)}
                registerSink={registerSink}
                unregisterSink={unregisterSink}
                onError={onError}
                resolvedTheme={resolvedTheme}
                findSignal={findSignal}
              />
            );
          })}
        </Suspense>
      </div>
    </div>
  );
}

/** 供应商编号 → 名字。列表还没取到时退回编号——认得出总比空一块强。 */
function vendorName(vendors: ApiVendor[], vendorId: string): string {
  return vendors.find((item) => item.id === vendorId)?.name ?? vendorId;
}
