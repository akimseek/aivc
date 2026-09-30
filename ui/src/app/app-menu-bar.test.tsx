// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { AppMenuBar } from "./app-menu-bar";
import { createAppMenus, type AppMenuActions, type AppMenuConditions } from "./app-menus";

/**
 * 菜单栏的测试。
 *
 * 分两层测，因为这个面本来就是「数据 + 渲染」分开的：
 *
 * 1. **结构**（`createAppMenus`）—— 菜单项、顺序、快捷键、**禁用条件**、分隔符、子菜单。
 *    禁用条件是旧平台写死在结构里的行为，不是美化，所以必须断言。
 * 2. **交互**（`AppMenuBar`）—— 点开、点外面关、Escape 关、执行后关闭、
 *    禁用项不执行、子菜单展开。
 */

function actions(overrides: Partial<AppMenuActions> = {}): AppMenuActions {
  return {
    manageSkills: vi.fn(),
    openGatewaySettings: vi.fn(),
    exportSession: vi.fn(),
    quit: vi.fn(),
    manageVendors: vi.fn(),
    installCli: vi.fn(),
    openLogDirectory: vi.fn(),
    clearGatewayLogs: vi.fn(),
    ...overrides
  };
}

const ALL_SUPPORTED: AppMenuConditions = {
  supportsSkills: true,
  isWslTarget: true,
  hasActiveSession: true
};

/** 取出某个菜单里某一项（按标签），找不到就抛——避免断言在 `undefined` 上通过。 */
function item(menus: ReturnType<typeof createAppMenus>, menuId: string, label: string) {
  const menu = menus.find((entry) => entry.id === menuId);
  if (!menu) throw new Error(`没有菜单 ${menuId}`);
  const found = menu.items.find((entry) => entry.label === label);
  if (!found) throw new Error(`菜单 ${menuId} 里没有 ${label}`);
  return found;
}

describe("createAppMenus（结构）", () => {
  it("三个菜单：文件 / 工具箱 / 帮助", () => {
    // 旧平台有四个（多一个「终端」），而那个菜单里唯一的项是**新建终端**——
    // 它调的就是系统终端。系统终端已按用户要求整体移除，因此这个菜单也去掉了。
    const menus = createAppMenus(ALL_SUPPORTED, actions());
    expect(menus.map((menu) => menu.label)).toEqual(["文件", "工具箱", "帮助"]);
  });

  it("导出是子菜单，含三种格式", () => {
    const menus = createAppMenus(ALL_SUPPORTED, actions());
    const exported = item(menus, "file", "导出");
    expect(exported.children?.map((child) => child.label)).toEqual([
      "导出 Markdown",
      "导出 JSON",
      "导出 HTML"
    ]);
  });

  it("没有活动会话时，导出连同三个子项一起禁用", () => {
    // 旧平台是**同步禁用**的：子菜单点不开、三项也点不动。
    const menus = createAppMenus({ ...ALL_SUPPORTED, hasActiveSession: false }, actions());
    const exported = item(menus, "file", "导出");
    expect(exported.disabled).toBe(true);
    expect(exported.children?.every((child) => child.disabled)).toBe(true);
  });

  it("设置会话只对 WSL 目标可用", () => {
    const wsl = createAppMenus(ALL_SUPPORTED, actions({ openSessionSettings: vi.fn() }));
    expect(item(wsl, "file", "设置会话").disabled).toBe(false);

    const local = createAppMenus(
      { ...ALL_SUPPORTED, isWslTarget: false },
      actions({ openSessionSettings: vi.fn() })
    );
    expect(item(local, "file", "设置会话").disabled).toBe(true);
  });

  it("面还没实现的项显示为禁用，而不是死按钮", () => {
    // 「设置会话」(M5) 与「关于」(M6) 的面还没做。**禁用**与「点了没反应」
    // 是两种不同的体验：前者说明「这里有个东西、现在还不能用」。
    const menus = createAppMenus(ALL_SUPPORTED, actions());
    expect(item(menus, "file", "设置会话").disabled).toBe(true);
    expect(item(menus, "help", "关于").disabled).toBe(true);
  });

  it("管理 skill 需要当前目标支持", () => {
    const menus = createAppMenus({ ...ALL_SUPPORTED, supportsSkills: false }, actions());
    expect(item(menus, "file", "管理 skill").disabled).toBe(true);
  });

  it("退出是危险项", () => {
    // 「命令面板 Ctrl + K」那一条随命令面板一起删掉了。
    const menus = createAppMenus(ALL_SUPPORTED, actions());
    expect(item(menus, "file", "退出").danger).toBe(true);
  });

  it("分隔符与项都在（结构里没有漏项）", () => {
    const menus = createAppMenus(ALL_SUPPORTED, actions({ showAbout: vi.fn() }));
    expect(item(menus, "file", "管理 skill")).toBeDefined();
    expect(menus.find((menu) => menu.id === "file")?.items.some((entry) => entry.separator)).toBe(
      true
    );
    expect(
      menus.find((menu) => menu.id === "toolbox")?.items.map((entry) => entry.label)
    ).toEqual(["供应商管理", "网关"]);
  });
});

describe("AppMenuBar（交互）", () => {
  function setup(overrides: Partial<AppMenuActions> = {}, conditions = ALL_SUPPORTED) {
    const injected = actions(overrides);
    const view = render(<AppMenuBar menus={createAppMenus(conditions, injected)} />);
    return { view, injected };
  }

  const trigger = (view: ReturnType<typeof render>, label: string) => {
    const found = [...view.container.querySelectorAll("button")].find(
      (item) => item.textContent?.trim() === label
    );
    if (!found) throw new Error(`没有菜单按钮 ${label}`);
    return found as HTMLButtonElement;
  };

  const menuItemButton = (view: ReturnType<typeof render>, label: string) => {
    const found = [...view.container.querySelectorAll(".menu-item")].find((item) =>
      item.textContent?.includes(label)
    );
    if (!found) throw new Error(`没有菜单项 ${label}`);
    return found as HTMLButtonElement;
  };

  it("菜单默认不展开，点击才展开", async () => {
    const { view } = setup();
    expect(view.container.querySelector(".menu-dropdown")).toBeNull();

    await act(async () => {
      trigger(view, "文件").click();
    });
    expect(view.container.querySelector(".menu-dropdown")).not.toBeNull();
  });

  it("点菜单项会执行并收起菜单", async () => {
    const manageSkills = vi.fn();
    const { view } = setup({ manageSkills });

    await act(async () => {
      trigger(view, "文件").click();
    });
    await act(async () => {
      menuItemButton(view, "管理 skill").click();
    });

    expect(manageSkills).toHaveBeenCalledTimes(1);
    expect(view.container.querySelector(".menu-dropdown")).toBeNull();
  });

  it("禁用项点了不执行", async () => {
    const { view, injected } = setup();
    await act(async () => {
      trigger(view, "文件").click();
    });
    const disabled = menuItemButton(view, "设置会话");
    expect(disabled.disabled).toBe(true);

    await act(async () => {
      disabled.click();
    });
    // 面没实现，这里本来就没有 action 可调——断言菜单也没被关掉（说明点击没被当成执行）。
    expect(view.container.querySelector(".menu-dropdown")).not.toBeNull();
    expect(injected.exportSession).not.toHaveBeenCalled();
  });

  it("子菜单展开后才能选到具体格式", async () => {
    const exportSession = vi.fn();
    const { view } = setup({ exportSession });

    await act(async () => {
      trigger(view, "文件").click();
    });
    expect(view.container.querySelector(".menu-submenu")).toBeNull();

    await act(async () => {
      menuItemButton(view, "导出").click();
    });
    expect(view.container.querySelector(".menu-submenu")).not.toBeNull();

    await act(async () => {
      menuItemButton(view, "导出 JSON").click();
    });
    expect(exportSession).toHaveBeenCalledWith("json");
  });

  it("Escape 关闭菜单", async () => {
    const { view } = setup();
    await act(async () => {
      trigger(view, "帮助").click();
    });
    expect(view.container.querySelector(".menu-dropdown")).not.toBeNull();

    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    await waitFor(() => {
      expect(view.container.querySelector(".menu-dropdown")).toBeNull();
    });
  });

  it("点外面关闭菜单", async () => {
    const { view } = setup();
    await act(async () => {
      trigger(view, "工具箱").click();
    });
    await act(async () => {
      document.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    });
    await waitFor(() => {
      expect(view.container.querySelector(".menu-dropdown")).toBeNull();
    });
  });
});
