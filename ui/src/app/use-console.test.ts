// @vitest-environment jsdom
import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { pickTarget, planTargetSwitch, shouldLoadSessionList, toRefs, useConsole } from "./use-console";
import type { AiSession, AiTarget, SessionQuickAccess } from "../types";

const target = (id: string, available: boolean): AiTarget => ({
  id,
  provider: "codex",
  label: id,
  kind: id.includes("wsl") ? "wsl" : "local",
  available
});

const session = (id: string): AiSession => ({
  id,
  title: id,
  filePath: `/sessions/${id}.jsonl`,
  messageCount: 1,
  preview: []
});

describe("target selection", () => {
  it("列表刷新时**保住当前选中的那个**（用户已经选过，不能丢）", () => {
    // `preferred` 现在只传「当前值」。这条与「还原上次选择」是两件事：
    // 前者是「别把用户刚选的弄丢」，后者是「替他选一个」——后者已删除。
    const items = [target("a", true), target("b", true)];
    expect(pickTarget(items, ["b"])).toBe("b");
  });

  it("当前选中的目标失效时，**不退回任何别的目标**", () => {
    // 发行版被移除、目录被删——原来这里会退回「第一个可用的」，
    // 那等于替用户换了一个目标，而他并不知道。现在是空。
    const items = [target("a", true), target("b", true)];
    expect(pickTarget(items, ["gone"])).toBe("");
  });

  it("**没有可还原的目标时什么都不选**，也不去挑「第一个可用的」", () => {
    // 这里原来断言「优先选可用的、全都不可用也要选一个」。那是我自己加的便利，
    // 旧平台没有——后果是**启动后什么都没点，界面已经替你选好了一个目标**。
    //
    // 现在只认还原值。要选就得用户自己选；没选时界面显示「请选择目标」，
    // 那本来就是旧平台有的状态。
    const items = [target("codex:wsl:broken", false), target("codex:local", true)];
    expect(pickTarget(items, [])).toBe("");
    expect(pickTarget([target("codex:wsl:broken", false)], [])).toBe("");
    // 还原值仍然优先，而且**优先于可用性**：用户上次选的就是它。
    expect(pickTarget(items, ["codex:wsl:broken"])).toBe("codex:wsl:broken");
  });

  it("returns an empty string for an empty list", () => {
    expect(pickTarget([], [])).toBe("");
  });
});

describe("mutation references", () => {
  it("carries the file path as a hint for the backend's directory fence", () => {
    expect(toRefs([session("s1")], ["s1"])).toEqual([
      { id: "s1", filePath: "/sessions/s1.jsonl" }
    ]);
  });

  it("ignores selected ids that are no longer in the list", () => {
    // 列表可能已经刷新过：选中的条目被别处删掉了。按 id 过滤，不能把不存在的条目
    // 送进批量操作——那会让后端收到无意义的引用。
    expect(toRefs([session("s1")], ["s1", "gone"])).toEqual([
      { id: "s1", filePath: "/sessions/s1.jsonl" }
    ]);
  });

  it("returns nothing when nothing is selected", () => {
    expect(toRefs([session("s1")], [])).toEqual([]);
  });
});

/**
 * 「点击进入终端会话后，列表内容就消失了」的根因就在这一组断言里。
 *
 * `selectTarget` 会清空会话列表，而重载 effect 只在 `targetId` 变化时重跑。
 * 所以在**当前目标**下点会话时绝不能走 `selectTarget`——否则清了没人填。
 */
describe("打开会话时的目标切换", () => {
  it("在当前目标下打开会话：**什么都不换**（否则列表被清空且不会重载）", () => {
    expect(planTargetSwitch("codex", "codex:local", { id: "codex:local", provider: "codex" })).toEqual(
      { provider: null, target: null }
    );
  });

  it("同一个平台下换了目标：只换目标，不动平台", () => {
    expect(
      planTargetSwitch("codex", "codex:local", { id: "codex:wsl:ubuntu", provider: "codex" })
    ).toEqual({ provider: null, target: "codex:wsl:ubuntu" });
  });

  it("跨平台打开会话：平台与目标一起换", () => {
    // 平台变了但目标没跟着换，就会拿新平台去查旧目标——列表必然是空的。
    expect(
      planTargetSwitch("codex", "codex:local", { id: "claude:local", provider: "claude" })
    ).toEqual({ provider: "claude", target: "claude:local" });
  });

  it("目标编号相同但平台变了，仍必须换目标", () => {
    // 「编号相同」在跨平台时是个陷阱：`local` 在两家平台上都叫 `local`，
    // 但它们是两个不同的目标，会话也完全不同。
    expect(planTargetSwitch("codex", "local", { id: "local", provider: "claude" })).toEqual({
      provider: "claude",
      target: "local"
    });
  });
});

/**
 * 收藏夹是一处**独立的内容**，不是当前目标列表的一个筛选。
 *
 * 它是 chips 里的一个 `view`，与「全部 / 回收站」并排，所以「切过去要不要发会话列表
 * 请求」不能再靠 pane 判断——这条断言就是那个约定的唯一守卫：收藏夹的数据来自
 * `quickAccess`，请求列表既多余，结果也不是用户要的那一批。
 */
describe("哪些 view 需要取会话列表", () => {
  it("收藏夹**不发**列表请求", () => {
    expect(shouldLoadSessionList("favorites")).toBe(false);
  });

  it("全部与回收站照常刷新列表", () => {
    expect(shouldLoadSessionList("active")).toBe(true);
    expect(shouldLoadSessionList("trash")).toBe(true);
  });
});

/**
 * `quickAccess` 的取数时机（在真 hook 上观察）。
 *
 * # 为什么值得单测
 *
 * 它是两处界面的**同一个数字**：收藏夹的内容，以及侧栏 chip 上「★ 收藏 N」。
 * 只在进收藏夹时取一次（改动前的行为）会让从没点过收藏 chip 的用户看到
 * 「★ 收藏 0」，点星之后数字也不动——这两条路径都只有 hook 层面测得出来。
 *
 * 装一个假的 Tauri 运行时（`window.__TAURI__.core.invoke`）：`useConsole` 的取数
 * 都经过 `ipc/commands` → 这一层，按命令名回数据就能既测到真 hook、
 * 又不碰真的 IPC。
 */
function fakeRuntime(commands: Record<string, () => unknown>) {
  const calls: string[] = [];
  const defaults: Record<string, () => unknown> = {
    "codex:list-session-quick-access": () => ({ favorites: [], recent: [] })
  };
  window.__TAURI__ = {
    core: {
      invoke: async (_command: string, payload?: unknown) => {
        const name = (payload as { command?: string } | undefined)?.command ?? "";
        calls.push(name);
        return (commands[name] ?? defaults[name] ?? (() => []))();
      }
    }
  };
  return calls;
}

const quickFavorites = (ids: string[]): SessionQuickAccess => ({
  favorites: ids.map((id) => ({
    target: target("codex:local", true),
    session: session(id),
    favorite: true
  })),
  recent: []
});

describe("quickAccess 的取数时机", () => {
  it("启动就取一次：没进过收藏夹，chip 上的数字也要是对的", async () => {
    const calls = fakeRuntime({
      "codex:list-session-quick-access": () => quickFavorites(["f1"])
    });
    const { result } = renderHook(() => useConsole());
    await waitFor(() => expect(result.current.quickAccess.favorites).toHaveLength(1));
    expect(
      calls.filter((name) => name === "codex:list-session-quick-access")
    ).toHaveLength(1);
  });

  it("收藏成功后刷新：数字从 0 变 1（不从「等用户进收藏夹」开始）", async () => {
    let stored: string[] = [];
    fakeRuntime({
      "codex:list-session-quick-access": () => quickFavorites(stored),
      "codex:set-session-favorite": () => {
        stored = ["s1"];
        return {};
      }
    });
    const { result } = renderHook(() => useConsole());
    await waitFor(() => expect(result.current.quickAccess.favorites).toHaveLength(0));
    act(() => result.current.selectTarget("codex:local"));
    await act(async () => {
      await result.current.toggleFavorite(session("s1"));
    });
    expect(result.current.quickAccess.favorites.map((item) => item.session.id)).toEqual(["s1"]);
    // `selectTarget` 还会排一个 0ms 防抖的列表加载。让它在本用例里跑完，
    // 否则它的请求会推到下一个用例的假运行时里（假运行时是窗口级的）。
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  });

  it("收藏夹里点「刷新列表」取的是 quickAccess，不是会话列表", async () => {
    // 改动前它是一个**死按钮**：`reloadSessions` → `loadSessions` 在收藏夹上
    // 直接早退（不发请求、也不重取 quickAccess）。
    const calls = fakeRuntime({
      "codex:list-session-quick-access": () => quickFavorites(["f1"])
    });
    const { result } = renderHook(() => useConsole());
    await waitFor(() => expect(result.current.quickAccess.favorites).toHaveLength(1));
    act(() => result.current.setView("favorites"));
    const before = calls.filter((name) => name === "codex:list-session-quick-access").length;
    await act(async () => {
      await result.current.reloadSessions();
    });
    const after = calls.filter((name) => name === "codex:list-session-quick-access").length;
    expect(after).toBe(before + 1);
    expect(calls).not.toContain("codex:list-sessions");
  });
});
