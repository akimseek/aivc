import { describe, expect, it } from "vitest";

import { PENDING_OUTPUT_LIMIT_BYTES, TerminalOutputRouter } from "./terminal-output";
import type { TerminalSink } from "./terminal-output";

/** 收集收到的块，并记录到达顺序。 */
function collector(): { sink: TerminalSink; chunks: Uint8Array[]; text: () => string } {
  const chunks: Uint8Array[] = [];
  return {
    sink: (bytes) => chunks.push(bytes),
    chunks,
    text: () => chunks.map((chunk) => new TextDecoder().decode(chunk)).join("")
  };
}

const bytes = (text: string) => new TextEncoder().encode(text);

describe("TerminalOutputRouter", () => {
  it("已挂上接收端时直接送达", () => {
    const router = new TerminalOutputRouter();
    const sink = collector();
    router.register("t1", sink.sink);

    router.push("t1", bytes("hello"));

    expect(sink.text()).toBe("hello");
    expect(router.pendingBytes("t1")).toBe(0);
  });

  it("接收端还没挂上时先缓冲，挂上后补发", () => {
    // 这是本文件存在的理由：TerminalPane 是动态加载的，从「终端已启动」到「接收端注册」
    // 之间有几百毫秒，而 shell 的横幅与提示符正好在这段时间里打印完。丢掉它们，终端会
    // **永远空着**，直到用户敲一下键盘为止。
    const router = new TerminalOutputRouter();
    const sink = collector();

    router.push("t1", bytes("PS C:\\> "));
    expect(router.pendingBytes("t1")).toBe(8);
    expect(sink.text()).toBe("");

    router.register("t1", sink.sink);

    expect(sink.text()).toBe("PS C:\\> ");
    expect(router.pendingBytes("t1")).toBe(0);
    expect(router.hasSink("t1")).toBe(true);
  });

  it("补发时保持原来的块顺序", () => {
    // 这些块里混着控制序列（清屏、光标定位），顺序错了界面会是乱的。
    const router = new TerminalOutputRouter();
    const sink = collector();

    router.push("t1", bytes("一"));
    router.push("t1", bytes("二"));
    router.push("t1", bytes("三"));
    router.register("t1", sink.sink);

    expect(sink.chunks).toHaveLength(3);
    expect(sink.text()).toBe("一二三");
  });

  it("补发之后不再重复发送", () => {
    const router = new TerminalOutputRouter();
    const sink = collector();
    router.push("t1", bytes("once"));
    router.register("t1", sink.sink);

    router.push("t1", bytes("twice"));

    expect(sink.text()).toBe("oncetwice");
  });

  it("不同终端互不串台", () => {
    // 后端把**所有**终端的输出推到同一条通道上，只靠载荷里的 terminalId 区分。
    // 分错了，一个终端的提示符会出现在另一个终端的窗口里。
    const router = new TerminalOutputRouter();
    const first = collector();
    const second = collector();
    router.register("t1", first.sink);

    router.push("t1", bytes("给一号"));
    router.push("t2", bytes("给二号"));
    router.register("t2", second.sink);

    expect(first.text()).toBe("给一号");
    expect(second.text()).toBe("给二号");
  });

  it("超出上限时丢新的、保住旧的", () => {
    // 上限只防失控。**提示符在开头**，所以必须丢新块——反过来等于把用户最需要看到的
    // 那一行扔掉，而后面那些早就滚出屏幕了。
    const router = new TerminalOutputRouter();
    const sink = collector();
    const half = Math.floor(PENDING_OUTPUT_LIMIT_BYTES / 2);

    router.push("t1", new Uint8Array(half).fill(65));
    // 第二块还装得下。
    router.push("t1", new Uint8Array(half).fill(66));
    // 第三块越界：丢掉，且不该改变已有缓冲。
    router.push("t1", bytes("会被丢掉"));

    expect(router.pendingBytes("t1")).toBe(half * 2);

    router.register("t1", sink.sink);
    expect(sink.chunks).toHaveLength(2);
  });

  it("卸载后新来的输出重新进入缓冲，而不是丢掉", () => {
    // 标签页切换会让 TerminalPane 卸载再挂载。卸载期间的输出必须留住——
    // 否则切回来会丢掉中间那一段，屏幕内容就断了。
    const router = new TerminalOutputRouter();
    const sink = collector();
    router.register("t1", sink.sink);
    router.push("t1", bytes("挂载时"));

    router.release("t1");
    expect(router.hasSink("t1")).toBe(false);
    router.push("t1", bytes("卸载后"));

    expect(sink.text()).toBe("挂载时");
    const again = collector();
    router.register("t1", again.sink);
    expect(again.text()).toBe("卸载后");
  });

  it("卸载时丢弃未发出的缓冲，避免随开关堆积", () => {
    const router = new TerminalOutputRouter();
    router.push("t1", bytes("没来得及发"));
    expect(router.pendingBytes("t1")).toBeGreaterThan(0);

    router.release("t1");

    expect(router.pendingBytes("t1")).toBe(0);
  });

  it("块列表与字节计数始终同步", () => {
    // 这两处必须一起清。此前它们是 hook 里两个并行的 Map ref，只清一个的话计数会与
    // 实况脱节，上限判断随之失效。
    const router = new TerminalOutputRouter();
    router.push("t1", bytes("abc"));
    router.register("t1", () => undefined);

    expect(router.pendingBytes("t1")).toBe(0);

    router.push("t1", bytes("de"));
    // 已经挂了接收端，不该再进缓冲。
    expect(router.pendingBytes("t1")).toBe(0);

    router.release("t1");
    expect(router.pendingBytes("t1")).toBe(0);
  });
});
