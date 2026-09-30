// @vitest-environment jsdom
import { act, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AiProviderSummary, AiSession, AiTarget, GatewayPortStatus } from "../../types";
import { SessionsSidebar, type SessionsSidebarConsole } from "./sessions-sidebar";

/**
 * 会话侧栏。
 *
 * # 这组测试钉住的是**几条看起来像废话、丢了却没人发现的规则**
 *
 * 1. 过滤 chips 切的是 `view`：收藏夹是**另一批数据**（`quickAccess`），
 *    不是「按收藏过滤当前列表」。判错了界面照常渲染，只是内容不对。
 * 2. 平台能力门控：能力矩阵里 `trash: false` 的平台，它的用户不该看到一个
 *    点下去必然报错的按钮（回收站），也不该看到勾了没用的勾选框。
 * 3. 空态文案三态：没选目标 / 目标不可用 / 真没有——三种处境要做的事不同。
 */

const PROVIDERS: AiProviderSummary[] = [
  { id: "codex", label: "Codex", capabilities: { trash: true, batchActions: true } },
  { id: "qoder", label: "Qoder CN", capabilities: { trash: false, batchActions: false } }
];

const TARGET: AiTarget = {
  id: "codex:local",
  provider: "codex",
  label: "本机 Codex",
  kind: "local",
  available: true,
  detail: "可读写"
};

const GATEWAY: GatewayPortStatus = {
  enabled: true,
  activePort: 50064,
  configuredPort: 0,
  baseUrl: "http://127.0.0.1:50064"
};

function session(overrides: Partial<AiSession> = {}): AiSession {
  return {
    id: "s1",
    title: "重构会话侧栏",
    filePath: "/sessions/s1.jsonl",
    messageCount: 3,
    updatedAt: "2026-09-30T10:00:00",
    preview: [{ role: "user", text: "第一句" }],
    ...overrides
  };
}

function consoleState(overrides: Record<string, unknown> = {}): SessionsSidebarConsole {
  return {
    busy: false,
    providers: PROVIDERS,
    providerId: "codex",
    selectProvider: vi.fn(),
    targets: [TARGET],
    targetId: TARGET.id,
    selectTarget: vi.fn(),
    pane: "sessions",
    view: "active",
    setView: vi.fn(),
    query: "",
    setQuery: vi.fn(),
    searching: false,
    sessions: [session()],
    selectedId: "",
    selectedIds: [],
    toggleSelected: vi.fn(),
    toggleSelectAll: vi.fn(),
    toggleFavorite: vi.fn(),
    reloadTargets: vi.fn(),
    reloadSessions: vi.fn(),
    configureWslCodexHome: vi.fn(),
    clearWslCodexHomeOverride: vi.fn(),
    quickAccess: { favorites: [], recent: [] },
    globalQuery: "",
    setGlobalQuery: vi.fn(),
    globalSearching: false,
    searchTargetId: "",
    setSearchTargetId: vi.fn(),
    searchProviderId: "",
    setSearchProviderId: vi.fn(),
    globalResults: [],
    restoreSelected: vi.fn(),
    purgeSelected: vi.fn(),
    deleteSelected: vi.fn(),
    ...overrides
  } as unknown as SessionsSidebarConsole;
}

function setup(
  consoleOverrides: Record<string, unknown> = {},
  props: Partial<Parameters<typeof SessionsSidebar>[0]> = {}
) {
  const state = consoleState(consoleOverrides);
  const handlers = {
    onOpen: vi.fn(),
    onResume: vi.fn(),
    onMenu: vi.fn(),
    onNewSession: vi.fn(),
    onDetail: vi.fn(),
    onToggleCollapsed: vi.fn()
  };
  const view = render(
    <SessionsSidebar
      console={state}
      selectedTarget={TARGET}
      providers={PROVIDERS}
      gateway={GATEWAY}
      {...handlers}
      {...props}
    />
  );
  return { view, state, handlers };
}

/** 按可见文字找一个 chip / 按钮。 */
const byText = (view: ReturnType<typeof setup>["view"], text: string) =>
  [...view.container.querySelectorAll("button")].find((item) =>
    item.textContent?.includes(text)
  )!;

describe("过滤 chips", () => {
  it("「全部 / ★ 收藏」切的是一个 `view`，回收站是旁边的图标按钮", () => {
    const { view, state } = setup();

    act(() => byText(view, "★ 收藏").click());
    expect(state.setView).toHaveBeenCalledWith("favorites");

    act(() => byText(view, "全部").click());
    expect(state.setView).toHaveBeenLastCalledWith("active");

    act(() => (view.container.querySelector('[aria-label="回收站"]') as HTMLElement).click());
    expect(state.setView).toHaveBeenLastCalledWith("trash");
  });

  it("换 chips 会**退出跨目标搜索**（否则点了界面不动，全局优先）", () => {
    const { view, state } = setup({ globalQuery: "旧关键词" });
    act(() => byText(view, "★ 收藏").click());
    expect(state.setGlobalQuery).toHaveBeenCalledWith("");
  });

  it("收藏夹渲染 `quickAccess` 的两组，**不**渲染当前目标的会话卡片", () => {
    const { view } = setup({
      view: "favorites",
      quickAccess: {
        favorites: [{ target: TARGET, session: session({ id: "f1" }), favorite: true }],
        recent: [{ target: TARGET, session: session({ id: "r1" }), favorite: false }]
      }
    });
    const titles = [...view.container.querySelectorAll(".session-group-title")].map(
      (item) => item.textContent
    );
    expect(titles).toEqual(["收藏", "最近打开"]);
    // 收藏夹是**另一批数据**；列表里那一条不该混进来。
    expect(view.container.querySelectorAll(".session-card")).toHaveLength(0);
  });

  it("收藏夹里打字是在收藏里筛，仍然只显示 quickAccess", () => {
    const { view } = setup({
      view: "favorites",
      query: "找不到的词",
      quickAccess: {
        favorites: [{ target: TARGET, session: session({ id: "f1" }), favorite: true }],
        recent: []
      }
    });
    expect(view.container.querySelectorAll(".session-card")).toHaveLength(0);
    expect(view.container.querySelector(".session-group-empty")!.textContent).toContain(
      "没有匹配的收藏"
    );
  });
});

describe("平台能力门控", () => {
  it("不支持回收站的平台：图标禁用且说明原因，而不是点了报错", () => {
    const { view } = setup({ providerId: "qoder", providers: PROVIDERS });
    const trash = view.container.querySelector('[aria-label="回收站"]') as HTMLButtonElement;
    expect(trash.disabled).toBe(true);
    expect(trash.title).toContain("支持回收站的平台");
  });

  it("不支持批量操作的平台：勾选框与批量条都不出现", () => {
    const { view } = setup({ providerId: "qoder", selectedIds: ["s1"] });
    expect(view.container.querySelector(".session-card-check")).toBeNull();
    expect(view.container.querySelector(".batchbar")).toBeNull();
  });

  it("支持批量 + 有勾选才出现批量条，且按回收站分三态按钮", () => {
    const plain = setup({ selectedIds: ["s1"] });
    expect(plain.view.container.querySelector(".batchbar")!.textContent).toContain("移入回收站");

    const trashed = setup({ selectedIds: ["s1"], view: "trash" });
    const bar = trashed.view.container.querySelector(".batchbar")!;
    expect(bar.textContent).toContain("已选 1 项");
    expect(bar.textContent).toContain("恢复");
    expect(bar.textContent).toContain("彻底删除");
  });
});

describe("保留的既有语义", () => {
  it("平台选择器有「请选择平台」占位项（否则下拉会高亮第一项）", () => {
    const { view } = setup({ providerId: "" });
    const options = [...view.container.querySelectorAll("select")][0]!;
    expect(options.querySelector('option[value=""]')!.textContent).toBe("请选择平台");
  });

  it("目标下拉两态文案：「暂无可用目标」与「请选择目标」", () => {
    const empty = setup({ targets: [], targetId: "" });
    expect(
      [...empty.view.container.querySelectorAll("select")][1]!.querySelector('option[value=""]')!
        .textContent
    ).toBe("暂无可用目标");

    const ready = setup();
    expect(
      [...ready.view.container.querySelectorAll("select")][1]!.querySelector('option[value=""]')!
        .textContent
    ).toBe("请选择目标");
  });

  it("不可用的目标在下拉里标「（不可用）」", () => {
    const { view } = setup({
      targets: [{ ...TARGET, available: false, detail: "发行版进不去" }]
    });
    expect(
      [...view.container.querySelectorAll("select")][1]!.textContent
    ).toContain("本机 Codex（不可用）");
  });

  it("空态是三句话之一：没选目标时说的是「先去选目标」", () => {
    const { view } = setup({ targetId: "", sessions: [] }, { selectedTarget: null });
    expect(view.container.textContent).toContain("请先在上面选择目标。");
  });

  it("目标不可用时给出重探与 WSL 目录表单", () => {
    const broken = { ...TARGET, available: false, detail: "发行版进不去", kind: "wsl" as const, distro: "Ubuntu" };
    const { view } = setup(
      { targets: [broken] },
      { selectedTarget: broken }
    );
    expect(view.container.textContent).toContain("发行版进不去");
    expect(view.container.querySelector(".wsl-home-form")).not.toBeNull();
  });

  it("底部摘要是「平台 · 共 N 条会话」+ 刷新", () => {
    const { view, state } = setup();
    const foot = view.container.querySelector(".sidebar-foot")!;
    expect(foot.textContent).toContain("Codex · 共 1 条会话");
    act(() => (foot.querySelector('[aria-label="刷新列表"]') as HTMLElement).click());
    expect(state.reloadSessions).toHaveBeenCalledTimes(1);
  });

  it("卡片的右键与「更多」都把坐标回传上去（菜单按鼠标位置开）", () => {
    const { view, handlers } = setup();
    const card = view.container.querySelector(".session-card") as HTMLElement;
    act(() => {
      card.dispatchEvent(
        new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 300, clientY: 120 })
      );
    });
    expect(handlers.onMenu).toHaveBeenCalledWith(300, 120, "s1");
  });
});

describe("折叠", () => {
  it("折叠后侧栏整个收掉，但**节点仍渲染**（否则主区会掉进那一格网格）", () => {
    const { view } = setup({}, { collapsed: true });
    const aside = view.container.querySelector("aside.sidebar")!;
    expect(aside.className).toContain("hidden");
    expect(view.container.querySelector(".session-card")).toBeNull();
  });

  it("折叠按钮在侧栏头右侧（`panel-left`），点击把状态交回上层", () => {
    const { view, handlers } = setup();
    const button = view.container.querySelector('[aria-label="折叠侧栏"]') as HTMLButtonElement;
    expect(button.querySelector("svg")).not.toBeNull();
    act(() => button.click());
    expect(handlers.onToggleCollapsed).toHaveBeenCalledTimes(1);
  });
});
