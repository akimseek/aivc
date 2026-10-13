// @vitest-environment jsdom
import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { HISTORY_MESSAGE_LIMIT, pickTarget, planTargetSwitch, shouldLoadSessionList, toRefs, useConsole } from "./use-console";
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

/**
 * 分支分给谁。
 *
 * # 为什么要在 hook 这一层断言命令参数
 *
 * R30 是一个**可达**的错配：侧栏卡片上的「消息与分支」能为**非选中**会话开弹框，
 * 而 `branchFromMessage` 此前只读 `selectedId`——用户在会话甲上点「从此处分支」，
 * 分出来的却是会话乙的分支。类型、门禁全绿，只有参数是错的。
 *
 * 这里沿用 `fakeRuntime` 那一套（假 Tauri 运行时），但**连参数一起记下来**：
 * 要断言的正是「后端收到哪一条会话的编号」。
 */
function recordingRuntime(answers: Record<string, () => unknown> = {}) {
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  // 页查询必须回**页对象**（`{messages, offset, messageCount}`），不能像其它命令
  // 那样兜底成 `[]`——上一个版本这里返回裸数组，`page.messages` 是 `undefined`，
  // 只是当时没有代码读它才没露馅（假运行时按契约给形状，见 P0-C 的教训）。
  const defaults: Record<string, () => unknown> = {
    "codex:get-session-messages-page": () => ({
      messages: [],
      offset: 0,
      messageCount: 0,
      hasMore: false
    })
  };
  window.__TAURI__ = {
    core: {
      invoke: async (_command: string, payload?: unknown) => {
        const { command = "", args = {} } = (payload ?? {}) as {
          command?: string;
          args?: Record<string, unknown>;
        };
        calls.push({ command, args });
        return answers[command]?.() ?? defaults[command]?.() ?? [];
      }
    }
  };
  return calls;
}

/**
 * `selectProvider` 的取值校验。
 *
 * # 为什么挡在 hook 里
 *
 * 它带一串清空副作用（目标、会话、选中项全作废）。脏值落进 `providerId` 之后，
 * 目标被清空、下拉里又没有匹配项，用户必须重选一次真实平台才能恢复；而这层
 * 「必须是自己校验一遍」的约定靠每个调用方记住是不牢的（`openAt` 也在调它）。
 * 侧栏的 `chooseProvider` 仍然挡在前面，这是第二道。
 */
describe("selectProvider 的取值校验", () => {
  it("只接受空值与清单里存在的编号，其余忽略", async () => {
    const providers = [
      { id: "codex", label: "Codex", capabilities: {} },
      { id: "claude", label: "Claude", capabilities: {} }
    ];
    fakeRuntime({ "ai:list-providers": () => providers });
    const { result } = renderHook(() => useConsole());
    await waitFor(() => expect(result.current.providers).toHaveLength(2));

    act(() => result.current.selectProvider("qoder"));
    expect(result.current.providerId).toBe("");

    act(() => result.current.selectProvider("claude"));
    expect(result.current.providerId).toBe("claude");

    // 空值仍然合法：它是「取消选择」这一动作。
    act(() => result.current.selectProvider(""));
    expect(result.current.providerId).toBe("");
  });
});

describe("branchFromMessage 分给哪条会话", () => {
  const branchArgs = (calls: { command: string; args: Record<string, unknown> }[]) =>
    calls.find((call) => call.command === "codex:branch-session")?.args;

  it("按明确来源目标创建；不污染当前另一目标的列表", async () => {
    const calls = recordingRuntime({ "codex:branch-session": () => session("b1") });
    const { result } = renderHook(() => useConsole());
    act(() => result.current.selectTarget("codex:local"));
    let branch;
    await act(async () => { branch = await result.current.branchFromMessage(17, "parent", "claude:wsl:Ubuntu"); });
    expect(branch).toMatchObject({ id: "b1" });
    expect(branchArgs(calls)).toEqual({ targetId: "claude:wsl:Ubuntu", sessionId: "parent", messageLine: 17 });
    expect(result.current.sessions).toEqual([]);
  });

  it("刷新失败仍返回并保留已创建的分支", async () => {
    recordingRuntime({
      "codex:branch-session": () => session("b1"),
      "codex:list-sessions": () => { throw new Error("刷新失败"); }
    });
    const { result } = renderHook(() => useConsole());
    act(() => result.current.selectTarget("codex:local"));
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
    await act(async () => {
      expect(await result.current.branchFromMessage(4, "parent")).toMatchObject({ id: "b1" });
    });
    expect(result.current.sessions.some((item) => item.id === "b1")).toBe(true);
    expect(result.current.selectedId).toBe("b1");
    expect(result.current.error).toBe("");
  });

  it("拒绝父会话编号，避免打开原会话", async () => {
    recordingRuntime({ "codex:branch-session": () => session("parent") });
    const { result } = renderHook(() => useConsole());
    act(() => result.current.selectTarget("codex:local"));
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
    await act(async () => { expect(await result.current.branchFromMessage(4, "parent")).toBeNull(); });
    expect(result.current.error).toContain("分支编号无效");
  });

  it("创建在途时连点只产生一个分支", async () => {
    let release!: (value: AiSession) => void;
    const calls = recordingRuntime({ "codex:branch-session": () => new Promise<AiSession>((resolve) => { release = resolve; }) });
    const { result } = renderHook(() => useConsole());
    act(() => result.current.selectTarget("codex:local"));
    await act(async () => {
      const first = result.current.branchFromMessage(4, "parent");
      expect(await result.current.branchFromMessage(4, "parent")).toBeNull();
      release(session("b1"));
      await first;
    });
    expect(calls.filter((call) => call.command === "codex:branch-session")).toHaveLength(1);
  });

  it("创建期间切换目标，不把旧目标分支插进新目标列表", async () => {
    let release!: (value: AiSession) => void;
    recordingRuntime({ "codex:branch-session": () => new Promise<AiSession>((resolve) => { release = resolve; }) });
    const { result } = renderHook(() => useConsole());
    act(() => result.current.selectTarget("codex:local"));
    let pending!: Promise<AiSession | null>;
    act(() => { pending = result.current.branchFromMessage(4, "parent"); });
    act(() => result.current.selectTarget("codex:wsl:Ubuntu"));
    await act(async () => { release(session("b1")); expect(await pending).toMatchObject({ id: "b1" }); });
    expect(result.current.targetId).toBe("codex:wsl:Ubuntu");
    expect(result.current.sessions).toEqual([]);
    expect(result.current.selectedId).toBe("");
  });

  it("显式传入的会话编号优先——弹框开着的那条未必是选中项", async () => {
    const calls = recordingRuntime({
      "codex:branch-session": () => session("分支-1")
    });
    const { result } = renderHook(() => useConsole());
    act(() => result.current.selectTarget("codex:local"));
    act(() => result.current.setSelectedId("selected-1"));

    await act(async () => {
      await result.current.branchFromMessage(42, "modal-1");
    });

    // 后端收到的是**弹框里那条**会话，而不是 `selectedId`。
    expect(branchArgs(calls)).toEqual({
      targetId: "codex:local",
      sessionId: "modal-1",
      messageLine: 42
    });
  });

  it("不传编号时退回选中项（菜单栏那类入口的语义）", async () => {
    const calls = recordingRuntime({
      "codex:branch-session": () => session("分支-1")
    });
    const { result } = renderHook(() => useConsole());
    act(() => result.current.selectTarget("codex:local"));
    // `selectTarget` 会排一个 0ms 防抖的列表加载；它是空列表，会把选中项清掉。
    // 先让那一轮跑完，再选中——否则这条断言验的不是「退回选中项」。
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    act(() => result.current.setSelectedId("selected-1"));

    await act(async () => {
      await result.current.branchFromMessage(7);
    });

    expect(branchArgs(calls)).toEqual({
      targetId: "codex:local",
      sessionId: "selected-1",
      messageLine: 7
    });
  });
});

describe("历史消息窗口：上限与两个边界（P1-J + 42 §五）", () => {
  /**
   * 分页假运行时：`total` 条消息、页大小 80（与 `MESSAGE_PAGE_SIZE` 一致），
   * 记下每次 `get-session-messages-page` 的 offset/limit。
   */
  function pagedRuntime(total: number) {
    const calls: { offset: number; limit: number }[] = [];
    const all = Array.from({ length: total }, (_, index) => ({
      role: "assistant",
      text: `m${index}`
    }));
    window.__TAURI__ = {
      core: {
        invoke: async (_command: string, payload?: unknown) => {
          const { command = "", args = {} } = (payload ?? {}) as {
            command?: string;
            args?: Record<string, unknown>;
          };
          switch (command) {
            case "codex:get-session-messages-page": {
              const offset = Number(args.offset ?? 0);
              const limit = Number(args.limit ?? 0);
              calls.push({ offset, limit });
              const start = offset < 0 ? Math.max(0, total - limit) : offset;
              return {
                messages: all.slice(start, start + limit),
                offset: start,
                messageCount: total,
                hasMore: start > 0
              };
            }
            case "codex:list-targets":
              return [target("codex:local", true)];
            case "codex:list-sessions":
              return [session("s1")];
            case "codex:list-session-quick-access":
              return { favorites: [], recent: [] };
            default:
              return [];
          }
        }
      }
    };
    return calls;
  }

  /**
   * 可挂起的假运行时：首屏（offset<0）立即返回，**往前翻的那一页先挂起**，
   * 由测试调用 `release()` 决定何时放行——这样才能构造「放行前切了会话」。
   */
  function heldPageRuntime(total: number) {
    const all = Array.from({ length: total }, (_, index) => ({
      role: "assistant",
      text: `m${index}`
    }));
    const pageAt = (start: number, limit: number) => ({
      messages: all.slice(start, start + limit),
      offset: start,
      messageCount: total,
      hasMore: start > 0
    });
    let release: (() => void) | undefined;
    window.__TAURI__ = {
      core: {
        invoke: async (_command: string, payload?: unknown) => {
          const { command = "", args = {} } = (payload ?? {}) as {
            command?: string;
            args?: Record<string, unknown>;
          };
          switch (command) {
            case "codex:get-session-messages-page": {
              const offset = Number(args.offset ?? 0);
              const limit = Number(args.limit ?? 0);
              if (offset < 0) return pageAt(Math.max(0, total - limit), limit);
              return new Promise((resolve) => {
                release = () => resolve(pageAt(offset, limit));
              });
            }
            case "codex:list-targets":
              return [target("codex:local", true)];
            case "codex:list-sessions":
              return [session("s1")];
            case "codex:list-session-quick-access":
              return { favorites: [], recent: [] };
            default:
              return [];
          }
        }
      }
    };
    return {
      release: () => {
        if (!release) throw new Error("没有挂起中的翻页请求");
        release();
      }
    };
  }

  it("翻到上限后不再发请求——累计不再增长", async () => {
    // 不封顶的话，几万条的老会话一路往前翻会把 DOM 与内存推到无界（P1-J）。
    const pageCalls = pagedRuntime(1000);
    const { result } = renderHook(() => useConsole());
    act(() => result.current.selectTarget("codex:local"));
    await waitFor(() => expect(result.current.sessions).toHaveLength(1));
    act(() => result.current.openAt(target("codex:local", true), session("s1")));
    await waitFor(() => expect(result.current.messages).toHaveLength(80));

    // 上限 400 = 80 × 5：还差 4 页到顶。
    for (let index = 0; index < 4; index += 1) {
      await act(async () => {
        await result.current.loadEarlierMessages();
      });
    }
    expect(result.current.messages).toHaveLength(HISTORY_MESSAGE_LIMIT);
    // 到顶不代表「没有更早的消息」——数据还在，只是不再往窗口里装。
    expect(result.current.hasEarlier).toBe(true);

    const requests = pageCalls.length;
    await act(async () => {
      await result.current.loadEarlierMessages();
    });
    expect(pageCalls.length).toBe(requests);
    expect(result.current.messages).toHaveLength(HISTORY_MESSAGE_LIMIT);
  });

  it("翻到头（0 < messageStart < 页大小）不出现重复消息（42 §五.1）", async () => {
    // total=125：首屏从 45 起；再往前翻时 offset 被钳到 0，后端回的是**整页**
    // `[0,80)`——其中 45..79 已经在窗口里，直接前置会把它们插两遍。
    pagedRuntime(125);
    const { result } = renderHook(() => useConsole());
    act(() => result.current.selectTarget("codex:local"));
    await waitFor(() => expect(result.current.sessions).toHaveLength(1));
    act(() => result.current.openAt(target("codex:local", true), session("s1")));
    await waitFor(() => expect(result.current.messages).toHaveLength(80));

    await act(async () => {
      await result.current.loadEarlierMessages();
    });

    const texts = result.current.messages.map((item) => item.text);
    expect(texts).toHaveLength(125);
    expect(new Set(texts).size).toBe(125); // 无重复
    expect(texts[0]).toBe("m0");
    expect(result.current.hasEarlier).toBe(false);
  });

  it("翻页途中切会话：旧页不许插进新窗口（42 §五.2）", async () => {
    const held = heldPageRuntime(1000);
    const { result } = renderHook(() => useConsole());
    act(() => result.current.selectTarget("codex:local"));
    await waitFor(() => expect(result.current.sessions).toHaveLength(1));
    act(() => result.current.openAt(target("codex:local", true), session("s1")));
    await waitFor(() => expect(result.current.messages).toHaveLength(80));

    // 翻页请求被挂起（放行时机由测试决定）。
    let loading: Promise<void> | undefined;
    act(() => {
      loading = result.current.loadEarlierMessages();
    });

    // 中途切到另一条会话——它的最新页立即返回。
    act(() => result.current.setSelectedId("s2"));
    await waitFor(() => expect(result.current.messages).toHaveLength(80));
    const after = result.current.messages.map((item) => item.text);

    // 现在才放行旧会话那一页：它必须被整个丢弃。
    await act(async () => {
      held.release();
      await loading;
    });
    expect(result.current.messages.map((item) => item.text)).toEqual(after);
  });
});
