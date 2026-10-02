// @vitest-environment jsdom
import { act, fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import * as sessionDisplay from "../../app/session-display";

import type {
  AiProviderSummary,
  AiSession,
  AiTarget,
  GatewayPortStatus,
  SessionQuickAccessItem
} from "../../types";
import { SessionsSidebar, type SessionsSidebarConsole } from "./sessions-sidebar";

/**
 * 会话侧栏。
 *
 * # 这组测试钉住的是**几条看起来像废话、丢了却没人发现的规则**
 *
 * 1. 过滤 chips 切的是 `view`：收藏夹是**另一批数据**（`quickAccess`），
 *    不是「按收藏过滤当前列表」。判错了界面照常渲染，只是内容不对。
 * 2. 平台能力门控：能力矩阵里 `trash: false` 的平台，它的用户不该看到一个
 *    点下去必然报错的按钮（回收站），也不该看到勾了没用的勾选框；
 *    `sessionSettings` 为假的平台上，不可用 WSL 目标的「WSL 目录表单」同样
 *    不该出现——它写的是 Codex 的全局设置，放行就是跨平台数据污染。
 * 3. 空态文案三态：没选目标 / 目标不可用 / 真没有——三种处境要做的事不同。
 */

const PROVIDERS: AiProviderSummary[] = [
  {
    id: "codex",
    label: "Codex",
    capabilities: { trash: true, batchActions: true, sessionSettings: true }
  },
  // claude / qoder 的 `sessionSettings` 与后端矩阵一致（仅 codex 为真）；trash 与
  // batchActions **刻意反配**（qoder 禁回收站、claude 禁批量）——两把键取值不同，
  // 「读错键」才会翻红；两个都为 false 时，把两个能力函数写反也照样全绿。
  {
    id: "claude",
    label: "Claude Code",
    capabilities: { trash: true, batchActions: false, sessionSettings: false }
  },
  {
    id: "qoder",
    label: "Qoder CN",
    capabilities: { trash: false, batchActions: true, sessionSettings: false }
  }
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

/** 快捷访问里的一条（收藏 / 最近打开两组共用这个形状）。 */
const quickItem = (id: string, favorite: boolean): SessionQuickAccessItem => ({
  target: TARGET,
  // 标题不能等于编号：`displayTitle` 会把「标题 == id」当成没有标题，退回「未命名会话」。
  session: session({ id, title: `会话 ${id}` }),
  favorite
});

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
    onMenu: vi.fn(),
    onNewSession: vi.fn(),
    onDetail: vi.fn(),
    onOpenGateway: vi.fn(),
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

  it("收藏夹里的本地筛选也是**按关键词命中**，命中的那条留着", () => {
    const hit = setup({
      view: "favorites",
      query: "f1",
      quickAccess: { favorites: [quickItem("f1", true)], recent: [quickItem("r1", false)] }
    });
    expect(hit.view.container.textContent).toContain("会话 f1");
    expect(hit.view.container.querySelectorAll(".session-group-empty")).toHaveLength(1); // 只有「最近打开」那一组空

    const miss = setup({
      view: "favorites",
      query: "查无此词",
      quickAccess: { favorites: [quickItem("f1", true)], recent: [quickItem("r1", false)] }
    });
    expect(miss.view.container.querySelectorAll(".session-group-empty")).toHaveLength(2);
  });

  it("本地筛选与后端查同一批字段：CLI 版本也命中", () => {
    // 用户照着报错里的版本号（`0.154.0`）找那条会话是常见动作；本地筛少一个字段
    // 就会把它判成「没命中」，而跨目标搜索（走后端）却找得到——同一个词两处结果不同。
    const view = setup({
      view: "favorites",
      query: "0.154.0",
      quickAccess: {
        favorites: [
          {
            target: TARGET,
            session: session({ id: "v1", title: "会话 v1", cliVersion: "0.154.0" }),
            favorite: true
          }
        ],
        recent: []
      }
    });
    expect(view.view.container.textContent).toContain("会话 v1");
    // 只有「最近打开」那一组是空的——收藏那一组命中了版本号。
    expect(view.view.container.querySelectorAll(".session-group-empty")).toHaveLength(1);
  });

  it("「★ 收藏 N」的 N **只数收藏**，不含「最近打开」", () => {
    // 裁定 R22：这个数字说的是「收藏夹里有几条星标过的会话」。此前它把
    // 「最近打开」也算进去，与 chip 上的字对不上。
    const { view } = setup({
      quickAccess: {
        favorites: [quickItem("f1", true), quickItem("f2", true)],
        recent: [quickItem("r1", false), quickItem("r2", false), quickItem("r3", false)]
      }
    });
    expect(byText(view, "★ 收藏").textContent).toBe("★ 收藏 2");
  });

  it("收藏夹视图的底部摘要说的是**里面这批**（收藏 + 最近打开），与 chip 的口径不同", () => {
    const { view } = setup({
      view: "favorites",
      quickAccess: {
        favorites: [quickItem("f1", true), quickItem("f2", true)],
        recent: [quickItem("r1", false), quickItem("r2", false), quickItem("r3", false)]
      }
    });
    expect(view.container.querySelector(".sidebar-foot")!.textContent).toContain("共 5 条会话");
  });
});

describe("头部", () => {
  it("网关胶囊是**按钮**（点击 → 设置中心网关页，spec 的逐字要求）", () => {
    const { view, handlers } = setup();
    const pill = view.container.querySelector(".gateway-pill") as HTMLButtonElement;
    expect(pill.tagName).toBe("BUTTON");
    expect(pill.textContent).toContain("网关");
    expect(pill.textContent).toContain("50064");
    expect(pill.title).toContain("设置中心网关页");
    act(() => pill.click());
    expect(handlers.onOpenGateway).toHaveBeenCalledTimes(1);
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
    // claude 的 `batchActions` 为假而 `trash` 为真：若「有没有勾选框」误读了
    // trash，这条会翻红（qoder 两个键同值时做不到这件事）。
    const { view } = setup({ providerId: "claude", selectedIds: ["s1"] });
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

  it("WSL 目录表单按平台能力放行：claude 的不可用 WSL 目标拿不到，codex 的拿得到", () => {
    // 表单的保存/清除走 `codex:set-wsl-codex-home`，写的是 Codex 的**全局**
    // `wsl_codex_homes` 映射——对 claude/qoder 放行就是跨平台数据污染，
    // 与环境页「WSL 会话目录」卡同一判据（`supportsSessionSettings`）。
    const claudeTarget: AiTarget = {
      ...TARGET,
      id: "claude:wsl",
      provider: "claude",
      available: false,
      kind: "wsl",
      distro: "Ubuntu",
      detail: "发行版进不去"
    };
    const blocked = setup(
      { providerId: "claude", targets: [claudeTarget], targetId: claudeTarget.id },
      { selectedTarget: claudeTarget }
    );
    expect(blocked.view.container.textContent).toContain("发行版进不去");
    expect(blocked.view.container.querySelector(".wsl-home-form")).toBeNull();

    // 反向：codex 的不可用 WSL 目标照旧拿得到入口（判据是能力，不是「干脆都不给」）。
    const codexTarget: AiTarget = {
      ...TARGET,
      available: false,
      kind: "wsl",
      distro: "Ubuntu",
      detail: "发行版进不去"
    };
    const allowed = setup({ targets: [codexTarget] }, { selectedTarget: codexTarget });
    expect(allowed.view.container.querySelector(".wsl-home-form")).not.toBeNull();
  });
});

describe("保留的既有语义", () => {
  it("平台选择器有「请选择平台」占位项（否则下拉会高亮第一项）", () => {
    const { view } = setup({ providerId: "" });
    const options = [...view.container.querySelectorAll("select")][0]!;
    expect(options.querySelector('option[value=""]')!.textContent).toBe("请选择平台");
  });

  it("平台列表为空时，「加载中…」不与占位项撞同一个空值", () => {
    // 两个 `value=""` 的 option 会让原生 select 的匹配变得有歧义（只认第一个）：
    // 加载提示因此不带 value，空值只留给占位项。
    const { view } = setup({ providers: [], providerId: "" });
    const select = [...view.container.querySelectorAll("select")][0]!;
    const empties = [...select.querySelectorAll('option[value=""]')];
    expect(empties).toHaveLength(1);
    expect(empties[0]!.textContent).toBe("请选择平台");
    expect(select.textContent).toContain("加载中…");
  });

  it("加载期的「加载中…」是**禁用项**：选中它等于把 `providerId` 写成一句提示文字", () => {
    // 它不带 `value`（值退化成文字），所以不禁用就是一个可被选中的脏值：
    // 写进去之后目标被清空、下拉里又没有这一项，用户得重选一次真实平台才能恢复。
    const { view } = setup({ providers: [], providerId: "" });
    const select = [...view.container.querySelectorAll("select")][0]!;
    const loading = [...select.querySelectorAll("option")].find(
      (option) => option.textContent === "加载中…"
    )!;
    expect(loading.disabled).toBe(true);
    // 但这一项要**留着**：下拉空着而不说明原因，用户分不清「没有平台」与「还在加载」。
    expect(loading.textContent).toBe("加载中…");
  });

  it("未知值不进 `providerId`：换平台的入口只放行空值与清单里的真实编号", () => {
    // 禁用项挡不住所有来路（这个值本来就存在，只是不该被当成平台编号收下），
    // 所以 `onChange` 里还有一层防御：不在清单里的值一律丢弃。
    const { view, state } = setup({ providers: [], providerId: "" });
    const select = [...view.container.querySelectorAll("select")][0]!;
    fireEvent.change(select, { target: { value: "加载中…" } });
    expect(state.selectProvider).not.toHaveBeenCalled();
  });

  it("防御不挡正常路径：选中真实平台照常换平台", () => {
    const { view, state } = setup();
    const select = [...view.container.querySelectorAll("select")][0]!;
    fireEvent.change(select, { target: { value: "qoder" } });
    expect(state.selectProvider).toHaveBeenCalledWith("qoder");

    // 占位项的空值也是合法取值（用户把平台清回「请选择平台」）。
    fireEvent.change(select, { target: { value: "" } });
    expect(state.selectProvider).toHaveBeenLastCalledWith("");
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
  it("折叠后节点**留在流内**（宽度归零），而不是 `display: none`", () => {
    // jsdom 量不了布局，所以这里钉住能够钉的部分：节点仍然渲染、带着
    // `.sidebar.hidden`，并且**没有**任何会让它退出网格放置的东西——
    // HTML 的 `hidden` 属性或行内样式都等于 `display: none`（那正是要避免的：
    // 网格项一旦退出放置，主区会被放进第 2 列 `auto`、第 3 列的 `1fr` 空着，
    // 右侧留一条空白）。宽度归零的规则在 `app.css` 的 `.sidebar.hidden`。
    // **主区宽度最终只有批次末截图能确认。**
    const { view } = setup({}, { collapsed: true });
    const aside = view.container.querySelector("aside.sidebar")!;
    expect(aside.className).toBe("sidebar hidden");
    expect(aside.hasAttribute("hidden")).toBe(false);
    expect(aside.getAttribute("style")).toBeNull();
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

describe("列表重渲面（P1-I）", () => {
  /**
   * 钉的是**侧栏这一层必须给卡片喂稳定回调**：`SessionCard` 有 `memo`，
   * 而 memo 只在 props 引用全等时才拦得住渲染。此前 `map` 里每张卡现造一组
   * 内联箭头，父层任何一次无关重渲都会把整列拖下水——这条用例就是那个回归的防线。
   *
   * 渲染次数用 `displayTitle` 的调用次数数（卡片渲染体必经的纯函数）。
   */
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("父层因无关状态重渲（console 换新对象、其余引用不变）时整列卡片不重渲", () => {
    const displaySpy = vi.spyOn(sessionDisplay, "displayTitle");
    const state = consoleState({
      sessions: [session(), session({ id: "s2", title: "第二条会话" })]
    });
    const handlers = {
      onOpen: vi.fn(),
      onMenu: vi.fn(),
      onNewSession: vi.fn(),
      onDetail: vi.fn(),
      onOpenGateway: vi.fn(),
      onToggleCollapsed: vi.fn()
    };
    const renderSidebar = (consoleValue: SessionsSidebarConsole) => (
      <SessionsSidebar
        console={consoleValue}
        selectedTarget={TARGET}
        providers={PROVIDERS}
        gateway={GATEWAY}
        {...handlers}
      />
    );

    const view = render(renderSidebar(state));
    expect(view.container.querySelectorAll(".session-card")).toHaveLength(2);
    const baseline = displaySpy.mock.calls.length;
    expect(baseline).toBeGreaterThanOrEqual(2);

    // 模拟「App 因无关 state 重渲」：`console` 是**新对象**（字段与函数仍是同一批
    // 引用）、其余 props 全等。老写法下这里每张卡都会重渲；memo + 稳定回调下应为 0。
    view.rerender(renderSidebar({ ...state }));
    expect(displaySpy.mock.calls.length).toBe(baseline);
  });
});
