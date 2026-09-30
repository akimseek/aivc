import { describe, expect, it } from "vitest";

import { pickTarget, planTargetSwitch, shouldLoadSessionList, toRefs } from "./use-console";
import type { AiSession, AiTarget } from "../types";

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
