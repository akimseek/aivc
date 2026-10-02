import { lazy, Suspense } from "react";

import { ErrorBoundary } from "../../app/error-boundary";
import type { TerminalUiState, TerminalSession } from "../../app/use-terminals";

/**
 * 终端**按需加载**。
 *
 * xterm 大约 340 KB，而它只在用户真的打开终端时才需要。静态引入会让每一个用户
 * 在首屏就为它付出代价——实测把主包从 235 KB 顶到 573 KB。
 */
const TerminalPane = lazy(() =>
  import("./terminal-pane").then((module) => ({ default: module.TerminalPane }))
);

/**
 * 终端视图：终端栈。
 *
 * # 会话信息条（标题 · 编号 · 目标 · 供应商 · 网关）为什么没了
 *
 * 2026-10-02 用户裁定：进入终端后不需要常驻那一行。信息不丢——供应商与网关在
 * **状态栏**（底部）与设置中心里都有，会话标题在标签与侧栏上。
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
  uiStates,
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
  uiStates: Record<string, TerminalUiState>;
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
  return (
    <div className={`terminal-view${hidden ? " hidden" : ""}`}>
      <div className="terminal-stack">
        {/*
          边界**只包终端这一段**：懒加载的 xterm chunk 取不到、或某一路终端渲染崩了，
          换掉的只是终端区——整个界面保持可用。把整页包进来等于用
          「终端坏了」赔上「标签栏、侧栏一起没了」，正是这条边界要避免的事。
        */}
        <ErrorBoundary label="终端加载失败">
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
                  composerVisible={ui?.composerVisible ?? false}
                  /*
                    「把焦点交给终端」的消费式信号（状态栏胶囊切到「终端原生输入」时发）。
                    缺席按 0 算：编号还没登记的那一路不会有焦点请求。
                  */
                  focusRequest={ui?.focusRequest ?? 0}
                  onOutput={() => onOutput(item.id)}
                  registerSink={registerSink}
                  unregisterSink={unregisterSink}
                  onError={onError}
                  resolvedTheme={resolvedTheme}
                  findSignal={findSignal}
                  /*
                    模型选择的来源判据要用这三个：平台、当前供应商、目标。
                    目标取自**这一路终端**记住的那个，不是当前选中的那个——两者在
                    多标签时可能不同（见 `use-terminals` 的 `TerminalSession.targetId`）。
                  */
                  providerId={item.providerId}
                  vendorId={item.vendorId}
                  targetId={item.targetId}
                />
              );
            })}
          </Suspense>
        </ErrorBoundary>
      </div>
    </div>
  );
}
