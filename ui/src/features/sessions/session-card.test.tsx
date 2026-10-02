// @vitest-environment jsdom
import { useState } from "react";

import { act, createEvent, fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AiSession } from "../../types";
import * as sessionDisplay from "../../app/session-display";
import { SessionCard } from "./session-card";

/**
 * 会话卡片。
 *
 * # 这组测试为什么值得写
 *
 * 卡片是**一整个可点区域**，里面还嵌着勾选框与三个动作按钮。卡片最难受的错是
 * 「点勾选框顺手把会话打开了」——它在界面上表现为「我明明只是想勾一条」，
 * 而那条会话莫名其妙变成了当前会话。因此这里钉住的第一条是**冒泡**：
 * 勾选框与动作按钮的点击都不许触发 `onOpen`。
 *
 * # 键盘是**同一份契约**的另一半
 *
 * 鼠标路径由 `stopPropagation` 挡住，键盘路径靠 keydown 处理函数开头的「让位」
 * 守卫。少了它，Enter / Space 会有两处走样：`onOpen` 被多调一次（想勾一条却把
 * 当前会话换掉、还抢了标签），而且 `preventDefault()` 会把内层控件自己的激活
 * 一并取消（按钮按不动）。
 */

/**
 * 按一次键并回传那个事件。
 *
 * 之所以要看 `defaultPrevented`：jsdom **不实现**原生控件的键盘激活（按 Enter 不会
 * 真的变成 click），所以「内层控件的激活被取消」在 jsdom 里只能从
 * `preventDefault()` 有没有被调用来判断——浏览器里它正是「按钮按不动」的原因。
 */
function press(element: HTMLElement, key: string) {
  const event = createEvent.keyDown(element, { key });
  fireEvent(element, event);
  return event;
}

function session(overrides: Partial<AiSession> = {}): AiSession {
  return {
    id: "s1",
    title: "重构会话侧栏",
    filePath: "C:/x/s1.jsonl",
    messageCount: 12,
    messageCountIsLowerBound: true,
    updatedAt: "2026-09-30T08:30:00",
    preview: [{ role: "assistant", text: "最后一条可显示的消息" }],
    ...overrides
  };
}

function setup(overrides: Partial<Parameters<typeof SessionCard>[0]> = {}) {
  const handlers = {
    onOpen: vi.fn(),
    onToggle: vi.fn(),
    onFavorite: vi.fn(),
    onBranch: vi.fn(),
    onMenu: vi.fn()
  };
  const props = {
    session: session(),
    active: false,
    checked: false,
    selectable: true,
    running: false,
    ...handlers,
    ...overrides
  };
  const view = render(<SessionCard {...props} />);
  return { view, handlers, session: props.session };
}

describe("会话卡片内容", () => {
  it("三行：标题 / 预览 / 条数，时间在标题行右端", () => {
    const { view } = setup();
    const card = view.container.querySelector(".session-card")!;
    expect(card.querySelector(".session-card-title")!.textContent).toBe("重构会话侧栏");
    expect(card.querySelector(".session-card-preview")!.textContent).toBe("最后一条可显示的消息");
    // 条数是**下界**（`messageCountIsLowerBound`），不能显示成精确值。
    expect(card.querySelector(".session-card-meta")!.textContent).toContain("≥12 条");
    expect(card.querySelector(".session-card-time")!.textContent).not.toBe("");
  });

  it("标题退化成编号时显示「目录 · 时间」（复用 `displayTitle`）", () => {
    const { view } = setup({ session: session({ id: "98895b66", title: "98895b66", cwd: "/w/项目" }) });
    expect(view.container.querySelector(".session-card-title")!.textContent).toContain("项目");
  });

  it("是分支会话时标出「分支」，不是就省略", () => {
    const { view } = setup({
      session: session({ metadata: { branch: { parentSessionId: "parent" } } })
    });
    expect(view.container.querySelector(".session-card-meta")!.textContent).toContain("分支");

    const plain = setup();
    expect(plain.view.container.querySelector(".session-card-meta")!.textContent).not.toContain(
      "分支"
    );
  });

  it("当前会话带 `on` 类；不勾选 `selectable` 时不渲染勾选框（平台能力门控）", () => {
    const { view } = setup({ active: true });
    expect(view.container.querySelector(".session-card")!.className).toContain("on");
    expect(view.container.querySelector(".session-card-check")).not.toBeNull();

    const locked = setup({ selectable: false });
    expect(locked.view.container.querySelector(".session-card-check")).toBeNull();
  });
});

describe("点击的归属", () => {
  it("点卡片本体 → 打开会话", () => {
    const { view, handlers } = setup();
    act(() => (view.container.querySelector(".session-card") as HTMLElement).click());
    expect(handlers.onOpen).toHaveBeenCalledTimes(1);
  });

  it("**点勾选框不会打开会话**（否则「我只是想勾一条」会把当前会话换掉）", () => {
    const { view, handlers } = setup({ checked: false });
    const box = view.container.querySelector(".session-card-check") as HTMLInputElement;
    act(() => box.click());
    expect(handlers.onToggle).toHaveBeenCalledTimes(1);
    expect(handlers.onOpen).not.toHaveBeenCalled();
  });

  it("三个动作按钮分别回传，且都不触发 `onOpen`", () => {
    const { view, handlers } = setup();
    const buttons = [...view.container.querySelectorAll<HTMLButtonElement>(".session-card-actions button")];
    expect(buttons).toHaveLength(3);
    act(() => buttons[0]!.click());
    act(() => buttons[1]!.click());
    act(() => buttons[2]!.click());
    expect(handlers.onFavorite).toHaveBeenCalledTimes(1);
    expect(handlers.onBranch).toHaveBeenCalledTimes(1);
    expect(handlers.onMenu).toHaveBeenCalledTimes(1);
    expect(handlers.onOpen).not.toHaveBeenCalled();
  });

  it("「更多」回传的是**点击坐标**——右键菜单按它就着的位置开", () => {
    const { view, handlers, session: cardSession } = setup();
    const more = [...view.container.querySelectorAll<HTMLButtonElement>(".session-card-actions button")][2]!;
    act(() => {
      more.dispatchEvent(new MouseEvent("click", { bubbles: true, clientX: 123, clientY: 45 }));
    });
    expect(handlers.onMenu).toHaveBeenCalledWith(cardSession, 123, 45);
  });

  it("`dblclick` 不再有独立动作——单击进入终端的语义由上层承担，双击即两次单击", () => {
    // 2026-10-02 口径修正：单击 = 进入终端；卡片不再绑 `onResume`。
    // 真实的双击序列（click → click → dblclick）里，后两次由 app 层的在途
    // 守卫收敛；这里只钉「`dblclick` 事件本身不再触发任何动作」。
    const { view, handlers } = setup();
    const card = view.container.querySelector(".session-card") as HTMLElement;
    act(() => {
      card.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    });
    expect(handlers.onOpen).not.toHaveBeenCalled();
  });

  it("右键回传坐标，并阻止浏览器默认菜单", () => {
    const { view, handlers, session: cardSession } = setup();
    const card = view.container.querySelector(".session-card") as HTMLElement;
    const event = new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      clientX: 210,
      clientY: 90
    });
    act(() => {
      card.dispatchEvent(event);
    });
    expect(handlers.onMenu).toHaveBeenCalledWith(cardSession, 210, 90);
    expect(event.defaultPrevented).toBe(true);
  });
});

describe("键盘的归属", () => {
  it("卡片进 Tab 序列；Enter / Space 与单击同一个动作，方向键不被劫持", () => {
    // 卡片是 `<div>` 加 `onClick`：不给 `role` / `tabIndex` / 键处理的话，
    // 整个会话列表就是**鼠标专属**——键盘用户打不开任何一条会话。
    const { view, handlers } = setup();
    const card = view.container.querySelector(".session-card") as HTMLElement;
    expect(card.getAttribute("role")).toBe("button");
    expect(card.tabIndex).toBe(0);

    fireEvent.keyDown(card, { key: "Enter" });
    expect(handlers.onOpen).toHaveBeenCalledTimes(1);

    fireEvent.keyDown(card, { key: " " });
    expect(handlers.onOpen).toHaveBeenCalledTimes(2);

    // 方向键留给列表导航（将来），卡片不该先抢。
    fireEvent.keyDown(card, { key: "ArrowDown" });
    expect(handlers.onOpen).toHaveBeenCalledTimes(2);
  });

  it("焦点在勾选框上时按 Space / Enter：让给它，既不打开会话也不取消它的激活", () => {
    // 「点勾选框不打开会话」这条契约必须在**键盘**上同样成立，否则键盘用户
    // 一按 Space 就换来「当前会话被换掉 + 标签被抢走」。
    const { view, handlers } = setup({ checked: false });
    const box = view.container.querySelector<HTMLInputElement>(".session-card-check")!;
    box.focus();

    for (const key of [" ", "Enter"]) {
      const event = press(box, key);
      expect(handlers.onOpen).not.toHaveBeenCalled();
      // 默认行为不许被取消：浏览器靠它把这个按键变成勾选框的激活。
      expect(event.defaultPrevented).toBe(false);
      // jsdom 不做那一步（见 `press` 的注释），这里补上浏览器随后会合成的 click。
      fireEvent.click(box);
    }

    expect(handlers.onToggle).toHaveBeenCalledTimes(2);
    expect(handlers.onOpen).not.toHaveBeenCalled();
  });

  it("焦点在每个动作按钮上时按 Enter / Space：只做按钮自己的动作，不打开会话", () => {
    const { view, handlers } = setup();
    const buttons = [
      ...view.container.querySelectorAll<HTMLButtonElement>(".session-card-actions button")
    ];
    const expected = [
      { label: "收藏", action: handlers.onFavorite },
      { label: "消息与分支", action: handlers.onBranch },
      { label: "更多", action: handlers.onMenu }
    ];

    buttons.forEach((button, index) => {
      button.focus();
      for (const key of ["Enter", " "]) {
        const event = press(button, key);
        expect(event.defaultPrevented, `${expected[index]!.label}：默认激活被取消了`).toBe(false);
        fireEvent.click(button);
      }
      expect(expected[index]!.action).toHaveBeenCalledTimes(2);
    });

    // 六次按键一次都不许漏到卡片上——漏了就是「想收藏，结果把会话打开并抢了标签」。
    expect(handlers.onOpen).not.toHaveBeenCalled();
  });
});

describe("memo（P1-I）", () => {
  /**
   * 渲染次数用 `displayTitle` 的调用次数来数——它是卡片渲染体里必经的纯函数。
   * 不用 React Profiler：断言嵌在 React 的内部回调里，改动一两行就读不出意图。
   */
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("props 引用不变时不重渲；对照：换了 prop 必须重渲", () => {
    const displaySpy = vi.spyOn(sessionDisplay, "displayTitle");
    const handlers = {
      onOpen: vi.fn(),
      onToggle: vi.fn(),
      onFavorite: vi.fn(),
      onBranch: vi.fn(),
      onMenu: vi.fn()
    };
    // 同一个对象引用贯穿全程——会话对象在列表刷新之间保持同一引用是 memo 的前提。
    const fixed = session();

    function Harness() {
      // 计数本身不参与渲染，只要它变化会让 Harness 重渲就够（这就是「无关状态」）。
      const [, setNoise] = useState(0);
      const [active, setActive] = useState(false);
      return (
        <div>
          <button onClick={() => setNoise((value) => value + 1)}>无关状态</button>
          <button onClick={() => setActive((value) => !value)}>切换选中</button>
          <SessionCard
            session={fixed}
            active={active}
            checked={false}
            selectable
            running={false}
            {...handlers}
          />
        </div>
      );
    }

    const view = render(<Harness />);
    expect(displaySpy).toHaveBeenCalledTimes(1);

    // 父组件因**无关状态**重渲：所有 props 都是同一引用 → memo 拦下，卡片一次都不重渲。
    fireEvent.click(view.getByText("无关状态"));
    expect(displaySpy).toHaveBeenCalledTimes(1);

    // 对照组：`active` 真的变了 → 必须重渲。证明上面的计数抓的是卡片渲染次数，
    // 而不是「计数从来没在工作」。
    fireEvent.click(view.getByText("切换选中"));
    expect(displaySpy).toHaveBeenCalledTimes(2);
  });
});
