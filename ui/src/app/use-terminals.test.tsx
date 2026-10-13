// @vitest-environment jsdom
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useTerminals } from "./use-terminals";
import type { AiTarget } from "../types";

/**
 * `useTerminals` 的装配测试。
 *
 * # 为什么到这里才引入 jsdom
 *
 * 纯逻辑（缓冲与分发）已经抽到 [`TerminalOutputRouter`] 里、脱离 React 测过了。
 * 这一层测的是**另一种东西**：后端事件 → hook 状态 → 界面。它正是前两轮修过的那两处
 * 所在的位置，而那两处当时**只有端到端截图作为证据**：
 *
 * - `terminal:status` 事件的 `terminalId` 曾经是空串，提醒写到不存在的键上，
 *   关注条永远不出现；
 * - `terminal:exit` 曾经从不自然触发，标签永远显示运行中。
 *
 * 两者都是「装配断了」，而装配断掉时类型、逻辑、门禁全都绿。所以这里不加别的，
 * 只盯着**事件进来之后状态对不对**。
 *
 * # 为什么不去 mock `../ipc/commands`
 *
 * `listen` 只做一件事：读 `window.__TAURI__.event.listen` 并把 `message.payload`
 * 交给回调。装一个假的 `window.__TAURI__` 就能连同这层包装一起测到；
 * mock 掉模块反而会把这段真实代码排除在外。
 */

type Handler = (event: { payload: unknown }) => void;

/** 构造 `terminal:data` 载荷：字节 → base64 + `encoding` 标签（与 Rust 侧实发形状一致）。 */
const dataPayload = (terminalId: string, bytes: number[]) => ({
  terminalId,
  encoding: "base64",
  data: btoa(String.fromCharCode(...bytes))
});

/** 装一个假的 Tauri 运行时，并给出「推事件」与「观察订阅」的入口。 */
function fakeRuntime() {
  const handlers = new Map<string, Handler[]>();
  const unlistened: string[] = [];
  let invocations: { command: string; args: unknown }[] = [];

  window.__TAURI__ = {
    core: {
      invoke: async (command: string, args?: unknown) => {
        invocations.push({ command, args });
        return {};
      }
    },
    event: {
      listen: async (channel: string, handler: Handler) => {
        const list = handlers.get(channel) ?? [];
        list.push(handler);
        handlers.set(channel, list);
        return () => {
          unlistened.push(channel);
        };
      }
    }
  };

  return {
    /** 后端推一条事件。 */
    emit(channel: string, payload: unknown) {
      for (const handler of handlers.get(channel) ?? []) {
        handler({ payload });
      }
    },
    subscriberCount: (channel: string) => (handlers.get(channel) ?? []).length,
    unlistened,
    invocations: () => invocations
  };
}

/** 渲染 hook，并等到「输出订阅」真的挂上——否则第一条事件会推给空气。 */
async function render(current: () => ReturnType<typeof useTerminals>) {
  const view = renderHook(current);
  await waitFor(() => {
    expect(window.__TAURI__).toBeDefined();
  });
  // 订阅是在 effect 里异步挂的，多给一轮微任务。
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
  return view;
}

/** 开一路 Codex 终端，返回它的 id。 */
async function openTerminal(result: { current: ReturnType<typeof useTerminals> }) {
  await act(async () => {
    await result.current.openTerminal(null);
  });
  return result.current.terminals.at(-1)!.id;
}

let runtime: ReturnType<typeof fakeRuntime>;

beforeEach(() => {
  runtime = fakeRuntime();
});

describe("useTerminals 的事件装配", () => {
  it("订阅了输出、退出、状态三条通道", async () => {
    await render(() => useTerminals());

    // 三条都必须挂上。少挂一条就是「功能静默缺失」——不报错、也没反应。
    await waitFor(() => {
      expect(runtime.subscriberCount("terminal:data")).toBe(1);
      expect(runtime.subscriberCount("terminal:exit")).toBe(1);
      expect(runtime.subscriberCount("terminal:status")).toBe(1);
    });
  });

  it("订阅了供应商切换通道", async () => {
    // 这条通道**主要是为自动故障转移存在的**。少了它，界面会一直显示一个已经不是
    // 实际服务方的供应商：请求正由第二家服务，状态栏却写着第一家。
    await render(() => useTerminals());

    await waitFor(() => {
      expect(runtime.subscriberCount("gateway:vendor-switched")).toBe(1);
    });
  });

  it("terminal:exit 让标签变成未运行", async () => {
    // 此前**永远不会发生**：ConPTY 下子进程自己退出不会让读取线程收到 EOF，
    // 退出事件只有在我们主动关闭终端时才发得出来。用户敲了 exit，界面看起来毫无变化。
    const view = await render(() => useTerminals());
    const id = await openTerminal(view.result);
    expect(view.result.current.terminals.at(-1)!.running).toBe(true);

    await act(async () => {
      runtime.emit("terminal:exit", { terminalId: id, exitCode: 0 });
    });

    expect(view.result.current.terminals.at(-1)!.running).toBe(false);
  });

  it("terminal:exit 只影响那一路终端", async () => {
    const view = await render(() => useTerminals());
    const first = await openTerminal(view.result);
    const second = await openTerminal(view.result);

    await act(async () => {
      runtime.emit("terminal:exit", { terminalId: first, exitCode: 1 });
    });

    const state = view.result.current.terminals;
    expect(state.find((item) => item.id === first)!.running).toBe(false);
    expect(state.find((item) => item.id === second)!.running).toBe(true);
  });

  it("terminal:status 带提醒时按 terminalId 记下来", async () => {
    // **按 terminalId 记录**是这条测试的重点。`terminalId` 曾经是空串，
    // 提醒被写进 `""` 这个不存在的键，关注条永远凑不出「在等你的终端」。
    const view = await render(() => useTerminals());
    const id = await openTerminal(view.result);

    await act(async () => {
      runtime.emit("terminal:status", {
        terminalId: id,
        status: "awaiting-confirmation",
        attention: { kind: "awaiting-confirmation", title: "等待确认" }
      });
    });

    expect(view.result.current.attention[id]?.title).toBe("等待确认");
    // 落不到空键上——那正是曾经的故障形态。
    expect(view.result.current.attention[""]).toBeUndefined();
  });

  it("terminal:status 不带提醒时清掉已有的提醒", async () => {
    // 不清的话标签上的小点会一直亮着，用户以为还有事没处理。
    const view = await render(() => useTerminals());
    const id = await openTerminal(view.result);

    await act(async () => {
      runtime.emit("terminal:status", {
        terminalId: id,
        status: "awaiting-confirmation",
        attention: { kind: "awaiting-confirmation", title: "等待确认" }
      });
    });
    expect(view.result.current.attention[id]).toBeDefined();

    // 用户响应了：后端发一条「运行中」，提醒就该消失。
    await act(async () => {
      runtime.emit("terminal:status", { terminalId: id, status: "running" });
    });

    expect(view.result.current.attention[id]).toBeUndefined();
  });

  it("相同提醒在重复状态事件或切换标签时只发一次系统通知", async () => {
    const view = await render(() => useTerminals());
    const id = await openTerminal(view.result);
    const payload = {
      terminalId: id,
      status: "awaiting-confirmation",
      attention: {
        kind: "awaiting-confirmation",
        title: "本轮已完成",
        detail: "切回应用可继续。"
      }
    };

    await act(async () => {
      runtime.emit("terminal:status", payload);
      runtime.emit("terminal:status", payload);
    });

    const notifications = runtime
      .invocations()
      .filter((item) => (item.args as { command?: string } | undefined)?.command === "terminal:attention-notification");
    expect(notifications).toHaveLength(1);

    await act(async () => {
      runtime.emit("terminal:status", {
        ...payload,
        attention: { ...payload.attention, detail: "新的确认请求" }
      });
    });
    expect(
      runtime.invocations().filter(
        (item) => (item.args as { command?: string } | undefined)?.command === "terminal:attention-notification"
      )
    ).toHaveLength(2);
  });

  it("接收端挂上之前到达的输出，挂上时按顺序补发", async () => {
    // 这条把 router 与 hook 的装配串起来：`TerminalPane` 是动态加载的，
    // shell 的提示符正好打在「终端已启动、接收端还没挂上」那个窗口里。
    const view = await render(() => useTerminals());
    const id = await openTerminal(view.result);

    await act(async () => {
      runtime.emit("terminal:data", dataPayload(id, [80, 83, 62]));
      runtime.emit("terminal:data", dataPayload(id, [32, 32]));
    });

    const received: Uint8Array[] = [];
    act(() => {
      view.result.current.registerSink(id, (bytes) => received.push(bytes));
    });

    expect(received.map((chunk) => new TextDecoder().decode(chunk)).join("")).toBe("PS>  ");
  });

  it("接收端挂上之后到达的输出直接送达", async () => {
    const view = await render(() => useTerminals());
    const id = await openTerminal(view.result);

    const received: Uint8Array[] = [];
    act(() => {
      view.result.current.registerSink(id, (bytes) => received.push(bytes));
    });
    await act(async () => {
      runtime.emit("terminal:data", dataPayload(id, [104, 105]));
    });

    expect(new TextDecoder().decode(received[0])).toBe("hi");
  });

  it("终端启动失败时标签标记为未运行", async () => {
    // 启动失败必须看得出来，否则用户只看到「点了没反应」。
    const view = await render(() => useTerminals());
    window.__TAURI__!.core!.invoke = vi.fn(async () => {
      throw new Error("PTY 启动失败");
    });

    await act(async () => {
      await expect(view.result.current.openTerminal(null)).rejects.toThrow();
    });

    const terminal = view.result.current.terminals.at(-1)!;
    expect(terminal.running).toBe(false);
    // 初值是**乐观**写进去的（见 `open`）：失败路径必须把它删干净，
    // 否则状态栏会显示一路根本不存在的终端的输入模式。
    expect(view.result.current.uiStates[terminal.id]).toBeUndefined();
  });

  it("关闭终端后不再收到它的输出", async () => {
    const view = await render(() => useTerminals());
    const id = await openTerminal(view.result);
    const received: Uint8Array[] = [];
    act(() => {
      view.result.current.registerSink(id, (bytes) => received.push(bytes));
    });

    await act(async () => {
      await view.result.current.closeTerminal(id);
    });
    await act(async () => {
      runtime.emit("terminal:data", dataPayload(id, [120]));
    });

    expect(received).toHaveLength(0);
    expect(view.result.current.terminals).toHaveLength(0);
  });

  /* ── 供应商绑定 ──────────────────────────────────────── */

  it("启动结果里的供应商被绑到这路终端上", async () => {
    // 界面**不在启动时挑供应商**：它绑定后端选出来的那一家。少了这一步，状态栏
    // 无从显示「当前由谁服务」——用户只会从账单上发现不对。
    const view = await render(() => useTerminals());
    window.__TAURI__!.core!.invoke = vi.fn(async () => ({
      terminalId: "ignored",
      cols: 100,
      rows: 30,
      vendorId: "vendor-a",
      vendorMode: "dynamic"
    }));

    await act(async () => {
      await view.result.current.openTerminal(null);
    });

    const terminal = view.result.current.terminals.at(-1)!;
    expect(terminal.vendorId).toBe("vendor-a");
    expect(terminal.vendorMode).toBe("dynamic");
  });

  it("自动故障转移会把界面上的供应商改成接手的那一家", async () => {
    // **这是界面这一侧最要紧的一条。** 自动转移发生在一次请求内部，用户的输入里
    // 没有任何迹象；界面不跟着改，就会一直写着已经不再服务的那一家。
    const view = await render(() => useTerminals());
    const id = await openTerminal(view.result);

    await act(async () => {
      runtime.emit("gateway:vendor-switched", {
        terminalId: id,
        routeId: id,
        vendorId: "vendor-b",
        reason: "failure",
        mode: "dynamic"
      });
    });

    const terminal = view.result.current.terminals.find((item) => item.id === id)!;
    expect(terminal.vendorId).toBe("vendor-b");
    // 自动切换必须留下痕迹：用户没做任何操作，界面却换了一家，不解释就像 bug。
    expect(terminal.lastSwitch).toEqual({ vendorId: "vendor-b", reason: "failure" });
  });

  it("只认 terminalId 与 routeId 中能对上的那一路终端", async () => {
    const view = await render(() => useTerminals());
    const first = await openTerminal(view.result);
    const second = await openTerminal(view.result);

    await act(async () => {
      runtime.emit("gateway:vendor-switched", {
        routeId: first,
        vendorId: "vendor-b",
        reason: "manual",
        mode: "locked"
      });
    });

    const state = view.result.current.terminals;
    expect(state.find((item) => item.id === first)!.vendorId).toBe("vendor-b");
    expect(state.find((item) => item.id === first)!.vendorMode).toBe("locked");
    expect(state.find((item) => item.id === second)!.vendorId).toBeUndefined();
  });

  it("空供应商切换事件不会覆盖当前供应商", async () => {
    const view = await render(() => useTerminals());
    const id = await openTerminal(view.result);
    await act(async () => {
      runtime.emit("gateway:vendor-switched", { terminalId: id, vendorId: "vendor-a", mode: "dynamic" });
    });
    for (const vendorId of ["", "  ", undefined]) {
      await act(async () => {
        runtime.emit("gateway:vendor-switched", { terminalId: id, vendorId, reason: "failure", mode: "locked" });
      });
      const terminal = view.result.current.terminals.find((item) => item.id === id)!;
      expect(terminal.vendorId).toBe("vendor-a");
      expect(terminal.vendorMode).toBe("dynamic");
      expect(terminal.lastSwitch).toEqual({ vendorId: "vendor-a", reason: "manual" });
    }
  });

  it("只是模式变化时不标成「已自动切换」", async () => {
    // `mode` 不是「换了供应商」。标成自动切换会让用户以为上游出过故障。
    const view = await render(() => useTerminals());
    const id = await openTerminal(view.result);

    await act(async () => {
      runtime.emit("gateway:vendor-switched", {
        terminalId: id,
        vendorId: "vendor-a",
        reason: "mode",
        mode: "locked"
      });
    });

    const terminal = view.result.current.terminals.find((item) => item.id === id)!;
    expect(terminal.vendorMode).toBe("locked");
    expect(terminal.lastSwitch).toBeUndefined();
  });

  it("手动切换失败时不改界面", async () => {
    // 切换会因为「供应商被禁用 / 配置不全 / 平台不匹配」而失败。先改再等失败回滚，
    // 中间那一帧界面就在撒谎。
    const view = await render(() => useTerminals());
    const id = await openTerminal(view.result);
    await act(async () => {
      runtime.emit("gateway:vendor-switched", {
        terminalId: id,
        vendorId: "vendor-a",
        reason: "manual",
        mode: "dynamic"
      });
    });
    window.__TAURI__!.core!.invoke = vi.fn(async () => {
      throw new Error("供应商协议与终端类型不匹配。");
    });

    await act(async () => {
      await expect(view.result.current.switchVendor(id, "vendor-z")).rejects.toThrow();
    });

    expect(
      view.result.current.terminals.find((item) => item.id === id)!.vendorId
    ).toBe("vendor-a");
  });

  it("把 CLI 参数与工作目录递给 terminal:start", async () => {
    // 新会话对话框里的「CLI 参数」要真的到达命令上。旧平台是
    // `parseCliArgs(params.cliArgs)` 追加为额外参数，后端 `terminal.rs` 已经收 `cliArgs`——
    // 中间少接一环就等于用户填了没用。
    const view = await render(() => useTerminals());

    await act(async () => {
      await view.result.current.openTerminal(
        null,
        "D:\\work",
        "  --model gpt-5-codex --sandbox workspace-write  "
      );
    });

    // 请求体在 `aivc_invoke` 的第二个参数里，形状是 `{ command, args }`。
    const start = runtime
      .invocations()
      .find(
        (item) => (item.args as { command?: string } | undefined)?.command === "terminal:start"
      );
    expect(start, "应当发出 terminal:start").toBeDefined();
    const payload = (start!.args as { args?: Record<string, string> }).args ?? {};
    // 参数要**去掉首尾空白**再传：用户随手多敲的空格不该进入命令行。
    expect(payload.cliArgs).toBe("--model gpt-5-codex --sandbox workspace-write");
    expect(payload.cwd).toBe("D:\\work");
  });

  it("没填 CLI 参数时不传这个字段", async () => {
    // 传空串会拼出多余的空参数；不传才是「没有额外参数」。
    const view = await render(() => useTerminals());

    await act(async () => {
      await view.result.current.openTerminal(null, undefined, "   ");
    });

    const start = runtime
      .invocations()
      .find(
        (item) => (item.args as { command?: string } | undefined)?.command === "terminal:start"
      );
    const payload = (start!.args as { args?: Record<string, string> }).args ?? {};
    expect(payload.cliArgs).toBeUndefined();
  });

  it("把目标编号记在这一路终端上（模型清单要用）", async () => {
    // 输入区的模型选择靠它：Qoder 的 `models:list` 只认 `targetId`。
    // 记在**这一路终端**上而不是「当前选中的目标」——多标签时两者未必是同一个。
    const view = await render(() => useTerminals());
    const target: AiTarget = {
      id: "qoder:local",
      provider: "qoder",
      label: "Qoder（本机）",
      kind: "local",
      available: true
    };

    await act(async () => {
      await view.result.current.openTerminal(target);
    });

    expect(view.result.current.terminals.at(-1)!.targetId).toBe("qoder:local");
  });

  it("恢复历史会话时把 sessionId 递给 terminal:start", async () => {
    // 后端把它变成 `--resume <id>`（`command.rs` 的 `resume_args`）。
    // 旧平台有一句要紧的注释：Qoder 按**项目目录**定位 `--resume` 的历史文件，
    // 所以恢复时 cwd 也必须在——这里一并断言。
    const view = await render(() => useTerminals());

    await act(async () => {
      await view.result.current.openTerminal(
        null,
        "D:\\work",
        undefined,
        undefined,
        "sess-123"
      );
    });

    const start = runtime
      .invocations()
      .find(
        (item) => (item.args as { command?: string } | undefined)?.command === "terminal:start"
      );
    const payload = (start!.args as { args?: Record<string, string> }).args ?? {};
    expect(payload.sessionId).toBe("sess-123");
    expect(payload.cwd).toBe("D:\\work");
  });

  it("不恢复时不传 sessionId", async () => {
    // 传了会让 CLI 去找一个不存在的会话——那是新建会话时的灾难。
    const view = await render(() => useTerminals());
    await act(async () => {
      await view.result.current.openTerminal(null, "D:\\work");
    });

    const start = runtime
      .invocations()
      .find(
        (item) => (item.args as { command?: string } | undefined)?.command === "terminal:start"
      );
    const payload = (start!.args as { args?: Record<string, string> }).args ?? {};
    expect(payload.sessionId).toBeUndefined();
  });

  /* ── 输入侧状态：状态栏读的那一份 ────────────────────────
   *
   * 这些状态从 `TerminalPane` 上收到了 `useTerminals`（spec §6：状态栏上的输入模式胶囊
   * 讲的是**当前前台那一路**终端）。它们没有别的可见入口——写错了界面上只有一个
   * 不出现/不消失的输入框，没有任何报错。
   */

  it("新会话：输入框一开始就可见，模式是自管输入", async () => {
    // 新会话没有历史要重放，输入框一开始就该在——藏起来用户会以为终端没起来。
    const view = await render(() => useTerminals());
    await act(async () => {
      await view.result.current.openTerminal(null);
    });
    const id = view.result.current.terminals.at(-1)!.id;

    expect(view.result.current.uiStates[id]).toEqual({
      inputMode: "composer",
      composerVisible: true
    });
  });

  it("恢复会话：输入框先藏着，第一段输出到达后由 noteOutput 放出", async () => {
    // 恢复时终端正在重放历史，输入框会挡住输出，所以初值是藏着的；
    // 而揭示它的时机**只有第一段输出**（旧平台也只在 `onTerminalData` 里 reveal）。
    const view = await render(() => useTerminals());
    await act(async () => {
      await view.result.current.openTerminal(null, "D:\\work", undefined, undefined, "sess-123");
    });
    const id = view.result.current.terminals.at(-1)!.id;

    expect(view.result.current.uiStates[id]).toEqual({
      inputMode: "composer",
      composerVisible: false
    });

    // 照 `TerminalPane` 的契约接上接收端：数据回调里报告「收到了输出」。
    act(() => {
      view.result.current.registerSink(id, () => view.result.current.noteOutput(id));
    });
    await act(async () => {
      runtime.emit("terminal:data", dataPayload(id, [104, 105]));
    });

    expect(view.result.current.uiStates[id]!.composerVisible).toBe(true);
  });

  it("状态栏切到自管输入时，藏着的输入框一并放出", async () => {
    // 状态栏胶囊那条路径。只改模式不放出输入框的话，用户看到的是
    // 「切过去了但看不见框」——切了跟没切一样。
    const view = await render(() => useTerminals());
    await act(async () => {
      await view.result.current.openTerminal(null, "D:\\work", undefined, undefined, "sess-123");
    });
    const id = view.result.current.terminals.at(-1)!.id;
    expect(view.result.current.uiStates[id]!.composerVisible).toBe(false);

    act(() => {
      view.result.current.setInputMode(id, "composer");
    });

    expect(view.result.current.uiStates[id]).toEqual({
      inputMode: "composer",
      composerVisible: true
    });

    // 反向不成立：切到「终端原生输入」只是把键盘交给终端，输入框不该消失
    // （用户还要靠它看草稿、贴图）。
    act(() => {
      view.result.current.setInputMode(id, "terminal");
    });
    expect(view.result.current.uiStates[id]).toEqual({
      inputMode: "terminal",
      composerVisible: true,
      // 切到原生输入**顺带请求一次终端焦点**（见下一条用例）：不请求的话，
      // 键盘用户敲的字仍然落在别处，「终端原生输入」就只是一个标签。
      focusRequest: 1
    });
  });

  it("切到「终端原生输入」请求一次终端焦点；切回自管输入不请求", async () => {
    // R29：输入区那个模式按钮删掉之后，状态栏胶囊是唯一的模式入口。焦点不跟回去，
    // 用户点了「终端原生输入」却打不了字——而这一条在界面上没有任何报错。
    const view = await render(() => useTerminals());
    await act(async () => {
      await view.result.current.openTerminal(null);
    });
    const id = view.result.current.terminals.at(-1)!.id;
    const request = () => view.result.current.uiStates[id]!.focusRequest ?? 0;
    expect(request()).toBe(0);

    act(() => {
      view.result.current.setInputMode(id, "terminal");
    });
    expect(request()).toBe(1);

    // 切回自管输入：那一刻要打字的地方是输入框，不能把焦点抢回终端。
    act(() => {
      view.result.current.setInputMode(id, "composer");
    });
    expect(request()).toBe(1);

    // 再切一次是**新的一次请求**——`TerminalPane` 只认计数变化。
    act(() => {
      view.result.current.setInputMode(id, "terminal");
    });
    expect(request()).toBe(2);
  });

  it("输出早于启动返回时也不丢揭示（R20 竞态）", async () => {
    // `start` 的 IPC 往返与第一段输出的到达是**两条独立的路**。输出先到、且此后
    // 不再有输出时，揭示的机会只有那一次——初值若等启动落定才写，
    // `noteOutput` 会扑空，输入框永远藏着。
    const view = await render(() => useTerminals());

    // 让 `terminal:start` 一直悬着，由测试决定它什么时候返回。
    let settle: (value: unknown) => void = () => undefined;
    window.__TAURI__!.core!.invoke = vi.fn(
      () => new Promise((resolve) => {
        settle = resolve;
      })
    );

    let opening!: Promise<string>;
    await act(async () => {
      opening = view.result.current.openTerminal(null, "D:\\work", undefined, undefined, "sess-123");
      await Promise.resolve();
    });
    const id = view.result.current.terminals.at(-1)!.id;

    // 初值必须在启动返回**之前**就位，否则下面那一次揭示无事可做。
    expect(view.result.current.uiStates[id]).toEqual({
      inputMode: "composer",
      composerVisible: false
    });

    act(() => {
      view.result.current.registerSink(id, () => view.result.current.noteOutput(id));
    });
    await act(async () => {
      runtime.emit("terminal:data", dataPayload(id, [104]));
    });
    expect(view.result.current.uiStates[id]!.composerVisible).toBe(true);

    // 启动随后成功返回，也不能把已经放出来的输入框盖回去。
    await act(async () => {
      settle({});
      await opening;
    });
    expect(view.result.current.uiStates[id]).toEqual({
      inputMode: "composer",
      composerVisible: true
    });
  });

  it("启动提示词要等**第一段输出**之后才发，不能早发", async () => {
    // ConPTY 必须等 CLI 回完 DSR（`\x1b[6n` → `\x1b[1;1R`）会话才开始推进。
    // 在 `terminal:start` 刚返回时就写，会赶在 CLI 准备好读输入之前——
    // 轻则提示词被吞，重则糊在 shell 提示符上。
    const view = await render(() => useTerminals());

    await act(async () => {
      await view.result.current.openTerminal(null, undefined, undefined, "帮我看看这个 bug");
    });
    const id = view.result.current.terminals.at(-1)!.id;

    const writesBefore = runtime
      .invocations()
      .filter((item) => (item.args as { command?: string } | undefined)?.command === "terminal:write");
    expect(writesBefore, "还没输出就不该写").toHaveLength(0);

    await act(async () => {
      runtime.emit(
        "terminal:data",
        dataPayload(id, Array.from(new TextEncoder().encode("ready\r\n")))
      );
    });

    const writes = runtime
      .invocations()
      .filter((item) => (item.args as { command?: string } | undefined)?.command === "terminal:write");
    expect(writes).toHaveLength(1);
    const payload = (writes[0]!.args as { args?: { terminalId?: string; data?: string } }).args;
    expect(payload?.terminalId).toBe(id);
    // 结尾必须有回车：没有它那句话只是躺在输入框里，用户得自己再敲一下。
    expect(payload?.data).toBe("帮我看看这个 bug\r");
  });

  it("启动提示词只发一次，后续输出不再重复发", async () => {
    const view = await render(() => useTerminals());
    await act(async () => {
      await view.result.current.openTerminal(null, undefined, undefined, "只发一次");
    });
    const id = view.result.current.terminals.at(-1)!.id;

    for (const chunk of ["a", "b", "c"]) {
      await act(async () => {
        runtime.emit(
          "terminal:data",
          dataPayload(id, Array.from(new TextEncoder().encode(chunk)))
        );
      });
    }

    const writes = runtime
      .invocations()
      .filter((item) => (item.args as { command?: string } | undefined)?.command === "terminal:write");
    expect(writes).toHaveLength(1);
  });

  it("没填提示词就不写任何东西", async () => {
    // 「空的提示词」不能变成一次空写入——那会往 CLI 里塞一个多余的回车。
    const view = await render(() => useTerminals());
    await act(async () => {
      await view.result.current.openTerminal(null, undefined, undefined, "   ");
    });
    const id = view.result.current.terminals.at(-1)!.id;

    await act(async () => {
      runtime.emit("terminal:data", dataPayload(id, [104, 105]));
    });

    const writes = runtime
      .invocations()
      .filter((item) => (item.args as { command?: string } | undefined)?.command === "terminal:write");
    expect(writes).toHaveLength(0);
  });

  /* ── 退出：干净退出要收掉，异常退出要留现场 ──
   *
   * 这是「`/exit` 后再次进入被锁住」的根因所在：界面此前只把 `running` 置 false，
   * 清理逻辑无从判断「这一次该不该收掉那一路」。旧平台的语义是
   * `if (exitCode !== 0) return;`——**只有干净退出才收**，
   * 异常退出保留现场让用户看得到最后几行报错。
   */
  it("退出码 0 → 标成「干净退出」，供上层收掉标签", async () => {
    const view = await render(() => useTerminals());
    const id = await openTerminal(view.result);

    await act(async () => {
      runtime.emit("terminal:exit", { terminalId: id, exitCode: 0 });
    });

    const terminal = view.result.current.terminals.find((item) => item.id === id)!;
    expect(terminal.running).toBe(false);
    expect(terminal.exitedCleanly).toBe(true);
  });

  it("非零退出 → **不**标成干净退出（异常退出要保留现场）", async () => {
    const view = await render(() => useTerminals());
    const id = await openTerminal(view.result);

    await act(async () => {
      runtime.emit("terminal:exit", { terminalId: id, exitCode: 1 });
    });

    const terminal = view.result.current.terminals.find((item) => item.id === id)!;
    expect(terminal.running).toBe(false);
    expect(terminal.exitedCleanly).not.toBe(true);
  });

  it("退出码缺失时按干净退出处理", async () => {
    // 用户点标签上的 ✕ 主动关闭时不该被当成异常。
    const view = await render(() => useTerminals());
    const id = await openTerminal(view.result);

    await act(async () => {
      runtime.emit("terminal:exit", { terminalId: id, exitCode: null });
    });

    expect(
      view.result.current.terminals.find((item) => item.id === id)!.exitedCleanly
    ).toBe(true);
  });

  it("退出后清掉这一路的提醒", async () => {
    // 提醒说的是「它在等你」，而它已经不在了——留着只会让铃铛一直亮。
    const view = await render(() => useTerminals());
    const id = await openTerminal(view.result);

    await act(async () => {
      runtime.emit("terminal:status", {
        terminalId: id,
        status: "awaiting-confirmation",
        attention: { kind: "awaiting-confirmation", title: "等确认" }
      });
    });
    expect(view.result.current.attention[id]).toBeDefined();

    await act(async () => {
      runtime.emit("terminal:exit", { terminalId: id, exitCode: 0 });
    });
    expect(view.result.current.attention[id]).toBeUndefined();
  });

  it("别路终端退出不影响这一路", async () => {
    const view = await render(() => useTerminals());
    const first = await openTerminal(view.result);
    const second = await openTerminal(view.result);

    await act(async () => {
      runtime.emit("terminal:exit", { terminalId: first, exitCode: 0 });
    });

    const after = view.result.current.terminals;
    expect(after.find((item) => item.id === first)!.exitedCleanly).toBe(true);
    expect(after.find((item) => item.id === second)!.running).toBe(true);
    expect(after.find((item) => item.id === second)!.exitedCleanly).toBeUndefined();
  });
});
