/**
 * 终端输出的分发与「接收端未就绪」缓冲。
 *
 * # 为什么需要缓冲，而不是丢弃
 *
 * `TerminalPane` 是**动态加载**的（xterm 有 330 KB），从「终端已启动」到「接收端注册」
 * 之间有几百毫秒。而 shell 的横幅与提示符正好在这段时间里打印完——丢掉它们，终端会
 * **永远空着**，直到用户敲一下键盘为止。
 *
 * 这里此前写着「缓存只会让首次输出延迟出现，反而更乱」，那个判断是错的：丢掉提示符
 * 比晚半秒显示糟得多。
 *
 * # 为什么是一个独立的类，而不是 hook 里的几个 ref
 *
 * 这段逻辑此前长在 `useTerminals` 里，用**三个并行的 Map ref** 表示（块列表、字节数、
 * 接收端）。它同时承担「分发」「缓冲」「补发」「上限」「清理」五件事，却**一条测试都
 * 没有**——因为要测它就得拉起 React 与 DOM 环境。
 *
 * 它是纯逻辑：输入是「某个终端的输出与注册动作」，输出是「谁收到了什么」。抽出来之后
 * 可以脱离 React 直接测，而 hook 只剩接线。
 */

/** xterm 的写入端。 */
export type TerminalSink = (bytes: Uint8Array) => void;

/**
 * 接收端注册前最多缓冲多少字节。
 *
 * 只防失控——正常只需几十 KB 就够放下提示符。
 */
export const PENDING_OUTPUT_LIMIT_BYTES = 512 * 1024;

/**
 * 按 `terminalId` 把后端推来的输出分发给各自的接收端。
 *
 * 接收端尚未注册时先缓冲，注册时**按原顺序**补发。
 */
export class TerminalOutputRouter {
  readonly #sinks = new Map<string, TerminalSink>();
  readonly #pending = new Map<string, Uint8Array[]>();
  readonly #pendingBytes = new Map<string, number>();

  /**
   * 后端推来一块输出。
   *
   * 接收端已就绪就直接送；否则缓冲起来等 `register`。**不会丢块**，除非超过上限。
   */
  push(terminalId: string, bytes: Uint8Array): void {
    const sink = this.#sinks.get(terminalId);
    if (sink) {
      sink(bytes);
      return;
    }
    const total = this.#pendingBytes.get(terminalId) ?? 0;
    // 上限只防失控：**提示符在开头**，所以超出时丢**新的**、保住旧的——
    // 反过来等于把用户最需要看到的那一行扔掉。
    if (total + bytes.length > PENDING_OUTPUT_LIMIT_BYTES) {
      return;
    }
    const buffered = this.#pending.get(terminalId);
    if (buffered) {
      buffered.push(bytes);
    } else {
      this.#pending.set(terminalId, [bytes]);
    }
    this.#pendingBytes.set(terminalId, total + bytes.length);
  }

  /**
   * 接收端挂载，并补发此前攒下的输出。
   *
   * **顺序必须保持**：这些块里混着控制序列（清屏、光标定位），顺序错了界面会是乱的。
   * 也正因如此，补发用的是**同一批 `Uint8Array` 引用**，不做任何合并或重排。
   */
  register(terminalId: string, sink: TerminalSink): void {
    this.#sinks.set(terminalId, sink);
    const buffered = this.#pending.get(terminalId);
    this.#forget(terminalId);
    for (const chunk of buffered ?? []) {
      sink(chunk);
    }
  }

  /**
   * 接收端卸载，或整路终端关闭。
   *
   * 两种情况在这里**效果相同**，都是有意的：卸载后新来的输出会重新缓冲（等下一次
   * 挂载），而关闭后不会再有输出。未发出的缓冲一律丢弃，避免随终端开关在内存里堆积。
   */
  release(terminalId: string): void {
    this.#sinks.delete(terminalId);
    this.#forget(terminalId);
  }

  /** 尚未发出的缓冲字节数（供诊断与测试）。 */
  pendingBytes(terminalId: string): number {
    return this.#pendingBytes.get(terminalId) ?? 0;
  }

  /** 该终端当前是否已挂上接收端（供诊断与测试）。 */
  hasSink(terminalId: string): boolean {
    return this.#sinks.has(terminalId);
  }

  /** 丢弃缓冲的块列表与计数——两处必须一起清，否则计数会与实况脱节。 */
  #forget(terminalId: string): void {
    this.#pending.delete(terminalId);
    this.#pendingBytes.delete(terminalId);
  }
}
