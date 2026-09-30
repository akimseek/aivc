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
 * App 级：恢复会话（`beginResumeSession`）把目标传给 `pathExists` 的**调用链**。
 *
 * # 为什么这一条必须挂在 App 上
 *
 * 缺陷（P0-C）在 `app.tsx` 的一个闭包里：`planResume` 的探针此前调
 * `api.pathExists(path)`，**不传 `target.id`**——后端于是按本机路径判断，
 * WSL 会话的 `/home/...` 在本机看来不存在，用户每次恢复 WSL 会话都被弹
 * 「原目录丢失」。`resume-session.ts` 的单测只能证明「抛错 → 按还在」，
 * 证明不了这个闭包有没有把目标递下去；那个调用点也没有别的渲染入口。
 * 因此这里渲染整个 `App`，像用户一样操作（选平台 → 选目标 → 双击卡片），
 * 断言**命令参数**。
 *
 * # 假后端**忠实复刻** `require_local` 的判据
 *
 * `shell:path-exists`：带 WSL 目标 → 显式拒绝（`require_local`，见
 * `crates/aivc-ipc/src/shell.rs:286-303`），本机目标 → 按用例配置的结果。
 * 判据由下面的 `wslDistroOf` 复刻：平台前缀与各平台的目标形制都照
 * `crates/aivc-core/src/target.rs` 的 `wsl_distro_of` 写，**不是** `includes("wsl:")`
 * 那种简化版——简化版比真后端宽松。修复轮 1 的 RED 证据就出自这里：判据先按
 * **修复前**的形制（Codex 只认裸 `wsl:`）复刻时，本条 WSL 用例如实变红
 * （`terminal:start` 一次都没发生——真实的修复前后端正是把 `codex:wsl:Ubuntu`
 * 当本机路径、判成「不存在」）。形制什么时候再对不上，这条用例就什么时候红。
 *
 * # 终端与主题的替身
 *
 * `terminal-pane`（xterm）要真实画布与 `ResizeObserver`，jsdom 没有
 * （见 `terminal-pane.test.tsx`），换成空组件——这里要钉的不是终端本身。
 * `matchMedia` 同样不存在，按 `use-theme.test.tsx` 的写法补一个。
 */

vi.mock("./terminal-pane", () => ({ TerminalPane: () => null }));

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

/** 后端（`aivc-provider`）真实发出的 Codex WSL 编号：带平台前缀的 `codex:wsl:<distro>`。 */
const WSL_TARGET: AiTarget = {
  id: "codex:wsl:Ubuntu",
  provider: "codex",
  label: "Ubuntu（WSL）",
  kind: "wsl",
  distro: "Ubuntu",
  available: true,
  detail: "可读写"
};

const LOCAL_TARGET: AiTarget = {
  id: "codex:local",
  provider: "codex",
  label: "本机 Codex",
  kind: "local",
  available: true,
  detail: "可读写"
};

/** 旧平台（以及旧缓存）留下的 Codex WSL 编号：裸 `wsl:<distro>`，没有平台前缀。 */
const LEGACY_WSL_TARGET: AiTarget = {
  id: "wsl:Ubuntu-22.04",
  provider: "codex",
  label: "Ubuntu-22.04（WSL）",
  kind: "wsl",
  distro: "Ubuntu-22.04",
  available: true,
  detail: "可读写"
};

/**
 * 复刻后端 `require_local` 的 WSL 判据（`crates/aivc-ipc/src/shell.rs:286-303` 调用
 * `ProviderId::from_target_id(...).wsl_distro_of(...)`，后者在 `aivc-core/src/target.rs`）。
 *
 * 逐条对照（修复轮 1 之后的形制）：
 * - 平台解析 `ProviderId::from_target_id`：`gemini:` / `claude:` / `qoder:` 前缀归各自
 *   平台，其余（含 `codex:` 与未知前缀）兜底 Codex；
 * - 每个平台的 WSL 前缀都是 `<provider>:wsl:`（`wsl_prefix()`，与后端发出的编号一致）；
 * - **Codex 额外**接受旧平台的裸 `wsl:<distro>`：旧缓存与会话记录里还有这种编号，
 *   后端解析时两种都认（生成新编号只用带前缀的那种）。
 *
 * 返回非空即「WSL 目标」，后端显式拒绝探测。**不要简化成 `includes("wsl:")`**：
 * 那种判据比真后端宽松，会让「目标形制对不上」的缺陷在这里静默通过。
 */
function wslDistroOf(targetId: string): string {
  const prefixes = targetId.startsWith("gemini:")
    ? ["gemini:wsl:"]
    : targetId.startsWith("claude:")
      ? ["claude:wsl:"]
      : targetId.startsWith("qoder:")
        ? ["qoder:wsl:"]
        : ["codex:wsl:", "wsl:"];
  for (const prefix of prefixes) {
    if (targetId.startsWith(prefix)) return targetId.slice(prefix.length);
  }
  return "";
}

/** 一条会话。`title` 不写成编号——`displayTitle` 会把「标题 == id」当作没有标题。 */
function session(cwd: string): AiSession {
  return {
    id: "sess-resume-1",
    title: "要恢复的会话",
    cwd,
    filePath: "sessions/sess-resume-1.jsonl",
    messageCount: 3,
    updatedAt: "2026-09-30T10:00:00",
    preview: [{ role: "user", text: "第一句" }]
  };
}

/** 假运行时：记录每次 `aivc_invoke` 的 `{ command, args }`，按命令名作答。 */
function fakeRuntime(options: {
  target: AiTarget;
  session: AiSession;
  /** 本机目标下 `shell:path-exists` 的回答。 */
  localExists: boolean;
}) {
  const calls: Array<{ command: string; args: Record<string, unknown> }> = [];

  const answer = (command: string, args: Record<string, unknown>): unknown => {
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
        return [options.target];
      case "codex:list-cached-sessions":
      case "codex:list-sessions":
        return [options.session];
      case "codex:get-session-messages-page":
        return { offset: 0, messages: [], hasMore: false, messageCount: 0 };
      case "codex:list-session-children":
        // 历史视图的血缘条会拉一次子分支；不答数组会让它拿到 `null`（真后端不会）。
        return [];
      case "shell:path-exists": {
        // 与 `require_local` 同一条分流：WSL 目标显式拒绝，本机照常检查。
        if (wslDistroOf(String(args.targetId ?? "")) !== "") {
          throw new Error("暂不支持 WSL 目标的路径操作。");
        }
        return options.localExists;
      }
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
        const args = payload?.args ?? {};
        calls.push({ command, args });
        return answer(command, args);
      }
    },
    event: { listen: async () => () => undefined }
  };

  return {
    /** 某条命令收到的全部参数，按调用先后。 */
    invoked: (command: string) =>
      calls.filter((call) => call.command === command).map((call) => call.args)
  };
}

/** 像用户一样走到「双击卡片恢复」：选平台 → 选目标 → 双击会话卡片。 */
async function resumeViaSidebar(container: HTMLElement, target: AiTarget) {
  const providerSelect = (await screen.findByLabelText("平台")) as HTMLSelectElement;
  await waitFor(() =>
    expect([...providerSelect.options].some((option) => option.value === "codex")).toBe(true)
  );
  fireEvent.change(providerSelect, { target: { value: "codex" } });

  const targetSelect = (await screen.findByLabelText("目标")) as HTMLSelectElement;
  await waitFor(() =>
    expect([...targetSelect.options].some((option) => option.value === target.id)).toBe(true)
  );
  fireEvent.change(targetSelect, { target: { value: target.id } });

  // 会话列表是异步到位的（缓存先行 + 新鲜探测）。
  const card = await waitFor(() => {
    const found = container.querySelector(".session-card");
    if (!found) throw new Error("会话卡片还没渲染出来");
    return found as HTMLElement;
  });
  fireEvent.doubleClick(card);
}

/** 排空剩余的异步落地（懒加载 chunk、promise 尾），免得它在断言之后才更新。 */
const settle = () => act(async () => {});

beforeEach(() => {
  // jsdom 没有 matchMedia（useTheme 挂载时要用），按 use-theme.test.tsx 的写法补一个。
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

describe("恢复会话的调用链（App 级）", () => {
  it("假后端的 WSL 判据与后端 `wsl_distro_of` 的形制一致", () => {
    // 这张表就是「各平台前缀 + 该平台的目标形制」的清单，照 `aivc-core/src/target.rs`
    // 抄写；后端改了形制，这里跟着改——改不动就说明生产里那两处会错判。
    expect(wslDistroOf("codex:wsl:Ubuntu-22.04")).toBe("Ubuntu-22.04");
    expect(wslDistroOf("wsl:Ubuntu")).toBe("Ubuntu"); // 旧平台/旧缓存的裸编号
    expect(wslDistroOf("gemini:wsl:Debian")).toBe("Debian");
    expect(wslDistroOf("claude:wsl:Ubuntu-22.04")).toBe("Ubuntu-22.04");
    expect(wslDistroOf("qoder:wsl:Ubuntu")).toBe("Ubuntu");
    // 别家不得借道 Codex 的旧形兼容分支。
    expect(wslDistroOf("gemini:wsl:")).toBe("");
    expect(wslDistroOf("codex:local")).toBe("");
    expect(wslDistroOf("unknown:local")).toBe("");
    expect(wslDistroOf("")).toBe("");
  });

  it("WSL 目标：pathExists 带上 targetId；后端拒绝探测时按「还在」直接进终端", async () => {
    const cwd = "/home/me/proj";
    const runtime = fakeRuntime({
      target: WSL_TARGET,
      session: session(cwd),
      // 若前端不传目标，后端会按本机路径判断——这里模拟「本机没有这个目录」，
      // 也就是缺陷当时的用户所见：每次恢复都被弹「原目录丢失」。
      localExists: false
    });

    const view = render(<App />);
    await resumeViaSidebar(view.container, WSL_TARGET);

    await waitFor(() => expect(runtime.invoked("shell:path-exists")).toHaveLength(1));
    expect(runtime.invoked("shell:path-exists")[0]).toEqual({ path: cwd, targetId: WSL_TARGET.id });

    // 后端拒绝探测 ⇒ `planResume` 的 catch 走「按还在」：直接起终端（带 --resume 的
    // 会话编号与原始工作目录），**不弹**「设置工作目录」。
    await waitFor(() => expect(runtime.invoked("terminal:start")).toHaveLength(1));
    expect(runtime.invoked("terminal:start")[0]).toMatchObject({
      targetId: WSL_TARGET.id,
      sessionId: "sess-resume-1",
      cwd
    });
    const text = view.container.textContent ?? "";
    expect(text).not.toContain("原工作目录不存在");
    expect(text).not.toContain("设置工作目录");
    await settle();
  });

  it("旧形 WSL 目标（裸 `wsl:`，旧缓存里的编号）：同样被拒绝、同样直接进终端", async () => {
    const cwd = "/home/me/legacy";
    const runtime = fakeRuntime({
      target: LEGACY_WSL_TARGET,
      session: session(cwd),
      localExists: false
    });

    const view = render(<App />);
    await resumeViaSidebar(view.container, LEGACY_WSL_TARGET);

    await waitFor(() => expect(runtime.invoked("shell:path-exists")).toHaveLength(1));
    expect(runtime.invoked("shell:path-exists")[0]).toEqual({
      path: cwd,
      targetId: LEGACY_WSL_TARGET.id
    });

    await waitFor(() => expect(runtime.invoked("terminal:start")).toHaveLength(1));
    expect(view.container.textContent ?? "").not.toContain("原工作目录不存在");
    await settle();
  });

  it("本地目标回归：目录真的不在时仍然弹「设置工作目录」，不自动起终端", async () => {
    const cwd = "D:\\gone";
    const runtime = fakeRuntime({
      target: LOCAL_TARGET,
      session: session(cwd),
      localExists: false
    });

    const view = render(<App />);
    await resumeViaSidebar(view.container, LOCAL_TARGET);

    await waitFor(() => expect(runtime.invoked("shell:path-exists")).toHaveLength(1));
    expect(runtime.invoked("shell:path-exists")[0]).toEqual({
      path: cwd,
      targetId: LOCAL_TARGET.id
    });

    // 「原目录已不在」这一支不变：弹对话框等用户重设，而不是背地里起终端。
    await screen.findByText("继续打开");
    expect(view.container.textContent ?? "").toContain("原工作目录不存在");
    expect(runtime.invoked("terminal:start")).toHaveLength(0);
    await settle();
  });
});
