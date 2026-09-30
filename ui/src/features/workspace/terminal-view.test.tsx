// @vitest-environment jsdom
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AiSession, AiTarget, ApiVendor, GatewayPortStatus } from "../../types";
import type { TerminalSession } from "../../app/use-terminals";
import { TerminalView } from "./terminal-view";

/**
 * 终端视图的信息条。
 *
 * # 只有信息条是有判据的一层，终端栈不是
 *
 * 栈里是懒加载的 `TerminalPane`（xterm）：它要真实画布与 `ResizeObserver`，jsdom
 * 两样都没有（见 `terminal-pane.test.tsx` 的说明）。这里把那一层换成空组件，把要钉的
 * 判据收在信息条上——**数据不可得的那一项不渲染**：它是一行常驻文本，宁可短一点，
 * 也不要「目标 -」那种占位（既没信息，又让人怀疑是不是加载失败）。
 *
 * 栈那两条（全部终端同时挂着；`hidden` 只收起来、不卸载）是这一层自己的映射逻辑，
 * 顺手用同一个空组件记下 props 钉住——xterm 的**行为**仍然只由截图与
 * `terminal-pane.test.tsx` 负责。
 */

/** 空组件替身：一渲染就把 props 记下来（上面第一条用例）。 */
const mounted = vi.hoisted(() => [] as Array<{ id: string; active: boolean }>);
vi.mock("../../app/terminal-pane", () => ({
  TerminalPane: (props: { terminalId: string; active: boolean }) => {
    mounted.push({ id: props.terminalId, active: props.active });
    return null;
  }
}));

const TERMINAL_ID = "7f3c1a2b-9d4e-4f5a-8b6c-000000000001";

const SESSION: AiSession = {
  id: "abcdef12-3456-7890-abcd-ef1234567890",
  title: "会话甲",
  cwd: "D:\\work",
  filePath: "D:\\work\\rollout.jsonl",
  messageCount: 0,
  preview: []
};

const TARGET: AiTarget = {
  id: "codex:local",
  provider: "codex",
  label: "本机",
  kind: "local",
  available: true
};

const VENDOR: ApiVendor = {
  id: "vendor-3f2a",
  providerId: "codex",
  name: "示例中转",
  apiBaseUrl: "https://api.example.com/v1",
  sort: 0,
  enabled: true
};

const GATEWAY: GatewayPortStatus = {
  enabled: true,
  configuredPort: 0,
  activePort: 50064,
  baseUrl: "http://127.0.0.1:50064"
};

function terminal(overrides: Partial<TerminalSession> = {}): TerminalSession {
  return {
    id: TERMINAL_ID,
    title: "终端自己记的标题",
    running: true,
    providerId: "codex",
    ...overrides
  };
}

/** 最小 props：所有数据源都留空，用例自己按需要给。 */
function props(
  overrides: Partial<Parameters<typeof TerminalView>[0]> = {}
): Parameters<typeof TerminalView>[0] {
  return {
    terminals: [],
    activeTerminalId: "",
    hidden: false,
    session: null,
    target: null,
    gateway: null,
    vendors: [],
    uiStates: {},
    onOutput: vi.fn(),
    registerSink: vi.fn(),
    unregisterSink: vi.fn(),
    onError: vi.fn(),
    resolvedTheme: "dark",
    findSignal: 0,
    ...overrides
  };
}

const strip = (container: HTMLElement) => container.querySelector(".terminal-info-strip")!;

/** 懒加载那一次动态导入在 `act` 里排完，免得它在断言之后才落地（同 `workspace-stage.test.tsx`）。 */
const settle = () => act(async () => {});

beforeEach(() => {
  mounted.length = 0;
});
afterEach(cleanup);

describe("终端信息条", () => {
  it("数据不可得的那一项不渲染：不写「目标 -」这种占位", () => {
    // 最小 props：没有会话、没有目标、没有网关——也就没有终端，供应商同样无从谈起。
    const view = render(<TerminalView {...props()} />);
    expect(strip(view.container).children).toHaveLength(0);
    expect(strip(view.container).textContent).toBe("");
  });

  it("标题与编号：有会话用它，会话缺席时退回终端自己记的", async () => {
    const own = render(
      <TerminalView {...props({ terminals: [terminal()], activeTerminalId: TERMINAL_ID })} />
    );
    expect(strip(own.container).querySelector(".strip-title")!.textContent).toBe(
      "终端自己记的标题"
    );
    expect(strip(own.container).querySelector(".strip-id")!.textContent).toBe("7f3c1a2b");
    await settle();

    const withSession = render(
      <TerminalView
        {...props({
          terminals: [terminal()],
          activeTerminalId: TERMINAL_ID,
          session: SESSION
        })}
      />
    );
    expect(strip(withSession.container).querySelector(".strip-title")!.textContent).toBe("会话甲");
    expect(strip(withSession.container).querySelector(".strip-id")!.textContent).toBe("abcdef12");
    await settle();
  });

  it("给全了每一项都在：目标 / 供应商（带锁定态）/ 网关", async () => {
    const view = render(
      <TerminalView
        {...props({
          terminals: [terminal({ vendorId: VENDOR.id, vendorMode: "locked" })],
          activeTerminalId: TERMINAL_ID,
          session: SESSION,
          target: TARGET,
          gateway: GATEWAY,
          vendors: [VENDOR]
        })}
      />
    );
    const text = strip(view.container).textContent ?? "";
    expect(text).toContain("目标 本机");
    // 供应商编号翻成名字，并标出这一路是锁定的还是可以动态切换的。
    expect(text).toContain("供应商 示例中转（锁定）");
    expect(text).toContain("网关 http://127.0.0.1:50064");
    await settle();
  });

  it("没有供应商编号 = 这一路没走网关：如实标「直连」，不是留空也不是占位", async () => {
    const view = render(
      <TerminalView {...props({ terminals: [terminal()], activeTerminalId: TERMINAL_ID })} />
    );
    const item = [...strip(view.container).querySelectorAll(".strip-item")].find((element) =>
      element.textContent?.startsWith("供应商")
    )!;
    // 「没走网关」是数据（见 `use-terminals` 对 `vendorId` 缺席的说明），不是缺失。
    expect(item.textContent).toBe("供应商 直连（用自己的密钥）");
    await settle();
  });

  it("供应商清单还没取到时退回编号（认得出总比空一块强）", async () => {
    const view = render(
      <TerminalView
        {...props({
          terminals: [terminal({ vendorId: VENDOR.id, vendorMode: "dynamic" })],
          activeTerminalId: TERMINAL_ID
        })}
      />
    );
    expect(strip(view.container).textContent).toContain("vendor-3f2a（动态）");
    await settle();
  });
});

describe("终端栈", () => {
  it("把全部终端都挂着：非前台只标 inactive，不卸载", async () => {
    // 卸载会让 PTY 的输出无人接收、xterm 的缓冲也没了；切回来就断一截。
    render(
      <TerminalView
        {...props({
          terminals: [terminal({ id: "t1" }), terminal({ id: "t2" })],
          activeTerminalId: "t2"
        })}
      />
    );
    await settle();

    const final = new Map(mounted.map((item) => [item.id, item.active]));
    expect([...final.keys()]).toEqual(["t1", "t2"]);
    expect(final.get("t2")).toBe(true);
    expect(final.get("t1")).toBe(false);
  });

  it("hidden 只是给容器加个类：终端照旧挂着，切回来的状态不丢", async () => {
    const view = render(
      <TerminalView {...props({ terminals: [terminal()], activeTerminalId: "", hidden: true })} />
    );
    await settle();
    expect(view.container.querySelector(".terminal-view")!.className).toContain("hidden");
    expect(mounted.map((item) => item.id)).toContain(TERMINAL_ID);
  });
});
