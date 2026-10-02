// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type {
  AiProviderSummary,
  AiSession,
  AiTarget,
  GatewayPortStatus
} from "../types";
import { App } from "./app";

/**
 * App 级：**单击会话行 = 直接进入该会话的终端**（用户口径，2026-10-02 修正）。
 *
 * # 口径与历史
 *
 * 批次一曾把单击改成「打开/激活标签、未运行显示历史」——详情先于终端。用户裁定
 * 这与预期相反：**单击就进入会话终端**（在跑切过去、没跑起 `--resume`），
 * 详情走卡片上的「消息与分支」按钮。本套件钉住新口径的三条：
 *
 * 1. 单击未打开的会话 → `shell:path-exists`（带目标）→ `terminal:start`
 *    （带会话编号，后端会变成 `--resume`），画布落在终端而不是历史正文；
 * 2. 该会话的终端**已在跑** → 只切标签，**不再起第二路**（替换语义会把在跑的收掉重开）；
 * 3. 恢复**在途**（pathExists 未回）时的连点/双击第二次点击 → 被守卫挡住，
 *    同样不产生第二路终端（判别力：去掉守卫时此用例变红——pathExists 与
 *    terminal:start 都会调两次）。
 *
 * # 终端与主题的替身
 *
 * 与 `app-resume.test.tsx` 同一套理由：`terminal-pane`（xterm）要真实画布，
 * jsdom 没有；`matchMedia` 同样不存在，补一个。终端本体不是这里要钉的东西，
 * 这里钉的是**命令链与画布落点**。
 */

vi.mock("../features/workspace/terminal-pane", () => ({ TerminalPane: () => null }));

const PROVIDERS: AiProviderSummary[] = [
  {
    id: "codex",
    label: "Codex",
    capabilities: {
      trash: true,
      batchActions: true,
      duplicate: true,
      branch: true,
      skills: true,
      export: true,
      sessionSettings: true,
      customCwd: true,
      vendorManagement: true,
      usage: true
    }
  }
];

const GATEWAY: GatewayPortStatus = {
  enabled: true,
  configuredPort: 0,
  activePort: 50064,
  baseUrl: "http://127.0.0.1:50064"
};

const TARGET: AiTarget = {
  id: "codex:local",
  provider: "codex",
  label: "本机 Codex",
  kind: "local",
  available: true,
  detail: "可读写"
};

function session(): AiSession {
  return {
    id: "sess-1",
    title: "一条历史会话",
    cwd: "D:\\work",
    filePath: "sessions/sess-1.jsonl",
    messageCount: 3,
    updatedAt: "2026-09-30T10:00:00",
    preview: [{ role: "user", text: "第一句" }]
  };
}

function fakeRuntime(options?: { holdPathExists?: Promise<void> }) {
  const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
  const answer = async (command: string): Promise<unknown> => {
    switch (command) {
      case "app:get-version":
        return "0.0.0-test";
      case "ai:list-providers":
        return PROVIDERS;
      case "gateway:get-port":
        return GATEWAY;
      case "codex:list-session-quick-access":
        return { favorites: [], recent: [] };
      case "codex:list-cached-targets":
      case "codex:list-targets":
        return [TARGET];
      case "codex:list-cached-sessions":
      case "codex:list-sessions":
        return [session()];
      case "codex:get-session-messages-page":
        return { offset: 0, messages: [{ role: "user", text: "第一句" }], hasMore: false, messageCount: 1 };
      case "codex:list-session-children":
        return [];
      case "shell:path-exists":
        if (options?.holdPathExists) await options.holdPathExists;
        return true;
      case "terminal:start":
        return {};
      default:
        return null;
    }
  };
  window.__TAURI__ = {
    core: {
      invoke: async (_command: string, raw?: unknown) => {
        const payload = raw as { command?: string; args?: Record<string, unknown> } | undefined;
        const command = payload?.command ?? "";
        calls.push({ command, args: payload?.args ?? {} });
        return answer(command);
      }
    },
    event: { listen: async () => () => undefined }
  };
  return {
    called: (command: string) => calls.filter((call) => call.command === command)
  };
}

async function openFirstCard(container: HTMLElement) {
  const providerSelect = (await screen.findByLabelText("平台")) as HTMLSelectElement;
  await waitFor(() =>
    expect([...providerSelect.options].some((option) => option.value === "codex")).toBe(true)
  );
  fireEvent.change(providerSelect, { target: { value: "codex" } });
  const targetSelect = (await screen.findByLabelText("目标")) as HTMLSelectElement;
  await waitFor(() =>
    expect([...targetSelect.options].some((option) => option.value === TARGET.id)).toBe(true)
  );
  fireEvent.change(targetSelect, { target: { value: TARGET.id } });
  return await waitFor(() => {
    const found = container.querySelector(".session-card");
    if (!found) throw new Error("卡片未渲染");
    return found as HTMLElement;
  });
}

/** 排空剩余的异步落地。 */
const settle = () => act(async () => {});

beforeEach(() => {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
    addListener: () => undefined,
    removeListener: () => undefined,
    dispatchEvent: () => false
  })) as unknown as typeof window.matchMedia;
});

afterEach(cleanup);

describe("单击会话行 = 进入会话终端（App 级）", () => {
  it("单击未打开的会话：探原目录 → 起 resume 终端，画布落在终端", async () => {
    const runtime = fakeRuntime();
    const { container } = render(<App />);
    const card = await openFirstCard(container);

    fireEvent.click(card);
    await waitFor(() => {
      expect(runtime.called("terminal:start")).toHaveLength(1);
    });
    await settle();

    const exists = runtime.called("shell:path-exists");
    expect(exists).toHaveLength(1);
    expect(exists[0]!.args.targetId).toBe("codex:local");
    expect(exists[0]!.args.path).toBe("D:\\work");

    const start = runtime.called("terminal:start")[0]!;
    expect(start.args.sessionId).toBe("sess-1");
    expect(start.args.cwd).toBe("D:\\work");

    // 画布 = 终端，不再是历史正文（旧口径的落点）。
    expect(container.querySelector(".history-view")).toBeNull();
    expect(container.querySelector(".terminal-stack")).not.toBeNull();
  });

  it("该会话的终端已在跑：单击只切过去，不再起第二路", async () => {
    const runtime = fakeRuntime();
    const { container } = render(<App />);
    const card = await openFirstCard(container);

    fireEvent.click(card);
    await waitFor(() => {
      expect(runtime.called("terminal:start")).toHaveLength(1);
    });

    // 第二次单击（等价于双击的第二次点击）：终端在跑 → 只激活，不替换。
    fireEvent.click(card);
    await settle();
    expect(runtime.called("terminal:start")).toHaveLength(1);
    expect(container.querySelector(".terminal-stack")).not.toBeNull();
  });

  it("恢复在途时的连点：第二次点击被守卫挡住，不产生第二路终端", async () => {
    let release!: () => void;
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    const runtime = fakeRuntime({ holdPathExists: held });
    const { container } = render(<App />);
    const card = await openFirstCard(container);

    // 第一次点击停在 `shell:path-exists` 的 await 上；第二次点击此刻发生。
    fireEvent.click(card);
    fireEvent.click(card);
    await settle();
    // 守卫挡住第二次点击：探测只发了一次。
    expect(runtime.called("shell:path-exists")).toHaveLength(1);

    await act(async () => {
      release();
    });
    await waitFor(() => {
      expect(runtime.called("terminal:start")).toHaveLength(1);
    });
    await settle();
    // 守卫放行后也只起了一路。
    expect(runtime.called("terminal:start")).toHaveLength(1);
  });
});
