import { Component, type ReactNode } from "react";

import { Button, EmptyState, errorText } from "../components";
import * as api from "../ipc/commands";

type ErrorBoundaryProps = {
  children: ReactNode;
  /**
   * 兜底的标题，同时进埋点。
   *
   * 两处挂载点的说法不同（整页「界面出错」/ 终端段「终端加载失败」），
   * 由调用方给，组件不猜自己在兜哪一段。
   */
  label: string;
};

type ErrorBoundaryState = {
  /** 子树抛过错了；为真时渲染兜底，重置为假即「重试」。 */
  failed: boolean;
  /** 捕获到的抛错原因。`retry` 按它分流：chunk 类失败重置 state 必再抛，只能整页重载。 */
  reason: unknown;
};

/**
 * 「界面资源取不回来」类错误的判据。
 *
 * 运行环境不只有 WebView2（Chromium）：按 `harden.rs` 的受信来源，应用实际覆盖三种
 * 形态——Windows 的 `http://tauri.localhost`（WebView2，Chromium 系）、macOS/Linux 的
 * `tauri://localhost`（WebKit 系）、`file://` 本地打包页。失败文案随引擎与环节分岔：
 * 动态 import 取不到，Chromium 是 `Failed to fetch dynamically imported module`，
 * WebKit 是 `Importing a module script failed`；Vite 的 preload helper 在 chunk 的 CSS
 * 预载依赖失败时**先于**真正的 `import()` 抛 `Unable to preload CSS for`（本应用懒加载
 * chunk 的预载依赖数组含 CSS，CSS 单独损坏/被拦/瞬断会先撞上它）；`Loading chunk` 是
 * webpack 系文案，本工程打不出来，留作备胎。取舍：判据收得窄、只认确切文案——宁可
 * 备而不用，也不要放过真错误。
 */
const CHUNK_LOAD_ERROR =
  /Failed to fetch dynamically imported module|Unable to preload CSS for|Importing a module script failed|Loading chunk/;

function isChunkLoadError(reason: unknown): boolean {
  return CHUNK_LOAD_ERROR.test(reason instanceof Error ? reason.message : String(reason ?? ""));
}

/**
 * 渲染错误的地基：子树抛错时换成一块**有出口**的兜底，而不是让整棵树被卸载成白屏。
 *
 * # 为什么必须是类组件
 *
 * React 只有类组件能在渲染期接住子树的错误（`getDerivedStateFromError`），
 * 函数组件没有对应钩子。
 *
 * # 挂载点各管一段
 *
 * `main.tsx` 兜整页（`<App/>` 崩了至少要看得见一句「界面出错」并点得到重试）；
 * `terminal-view.tsx` 只兜终端那一段——懒加载的 xterm chunk 失败只该换掉终端，
 * 不该把标签栏、侧栏、信息条一起换掉。
 */
export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  override state: ErrorBoundaryState = { failed: false, reason: null };

  /** 抛错后**同一帧**就切到兜底：出错的那次渲染不会提交，坏输出不会露出来。 */
  static getDerivedStateFromError(error: unknown): Partial<ErrorBoundaryState> {
    return { failed: true, reason: error };
  }

  override componentDidCatch(error: Error): void {
    try {
      /*
        渲染层抛错在 Rust 侧完全不可见（`performance:log` 就是为此存在的），
        这里是它的去处。`api.logPerformance` 已吞掉 IPC 失败，这层 try 再兜同步异常——
        **埋点失败绝不能让错误处理路径自己再炸一次**，那会把兜底也带走。
      */
      const reason = errorText(error);
      void api.logPerformance("ui.error-boundary", 0, `${this.props.label}：${reason}`);
    } catch {
      // 静默：兜底已经画出来了，日志失败没有别的出口。
    }
  }

  override render(): ReactNode {
    if (!this.state.failed) return this.props.children;
    /*
      说明文案跟着「重试」的真实行为走：chunk 类失败点「重试」是整页重载，此刻还说
      「再试一次」会让人以为只重挂这一块、白等一次眨眼不到的整页刷新。
    */
    const chunkLost = isChunkLoadError(this.state.reason);
    return (
      <div className="error-boundary" role="alert">
        <EmptyState
          message={this.props.label}
          hint={
            chunkLost
              ? "界面资源没能取回来，点「重试」会重新加载界面。"
              : "这一块没能正常显示，点「重试」再试一次。"
          }
          action={
            <Button variant="primary" onClick={this.retry}>
              重试
            </Button>
          }
        />
      </div>
    );
  }

  /**
   * 「重试」按抛错来源分流：
   *
   * - **界面资源取不回来**（`isChunkLoadError`）：只能整页 reload。`lazy()` 的 rejection
   *   被 React 缓存在组件上（`lazyInitializer` 对已 rejection 的 payload **同步**重抛），
   *   浏览器 module map 也缓存失败的动态 import——重置 state = 同一帧再抛回去，兜底
   *   原地不动，按钮就成了「什么都没发生」。reload 让脚本重新取一次才有自愈机会。
   * - **其余错误**（渲染期的偶发竞态等）：重置边界 state，子树整棵重挂，值得再试一次。
   */
  private retry = () => {
    if (isChunkLoadError(this.state.reason)) {
      window.location.reload();
      return;
    }
    this.setState({ failed: false });
  };
}
