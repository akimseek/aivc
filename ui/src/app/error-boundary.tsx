import { Component, type ReactNode } from "react";

import { Button, EmptyState } from "../components";
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
};

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
  override state: ErrorBoundaryState = { failed: false };

  /** 抛错后**同一帧**就切到兜底：出错的那次渲染不会提交，坏输出不会露出来。 */
  static getDerivedStateFromError(): ErrorBoundaryState {
    return { failed: true };
  }

  override componentDidCatch(error: Error): void {
    try {
      /*
        渲染层抛错在 Rust 侧完全不可见（`performance:log` 就是为此存在的），
        这里是它的去处。`api.logPerformance` 已吞掉 IPC 失败，这层 try 再兜同步异常——
        **埋点失败绝不能让错误处理路径自己再炸一次**，那会把兜底也带走。
      */
      const reason = error instanceof Error ? error.message : String(error);
      void api.logPerformance("ui.error-boundary", 0, `${this.props.label}：${reason}`);
    } catch {
      // 静默：兜底已经画出来了，日志失败没有别的出口。
    }
  }

  override render(): ReactNode {
    if (!this.state.failed) return this.props.children;
    return (
      <div className="error-boundary" role="alert">
        <EmptyState
          message={this.props.label}
          hint="这一块没能正常显示，点「重试」再试一次。"
          action={
            <Button variant="primary" onClick={this.retry}>
              重试
            </Button>
          }
        />
      </div>
    );
  }

  /** 「重试」= 重置边界 state：子树整棵重挂——渲染期的偶发错误（数据竞态等）值得再试一次。 */
  private retry = () => {
    this.setState({ failed: false });
  };
}
