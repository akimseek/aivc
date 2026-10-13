// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  VendorsPanel,
  balanceLabel,
  balanceText,
  parseAdvancedJson,
  parsePrice,
  rateText
} from "./vendors-panel";
import type { AiProviderId } from "../../../types";

/**
 * 供应商管理。
 *
 * # 这组测试的由来
 *
 * 用户报「**供应商没有模型厂商的选择**」。原因是明确的：表单保存时用的是
 * 外层的 `providerId`（侧栏当前平台），界面上**没有任何地方能改它**。
 *
 * 所以这里钉住的第一条就是：**保存时用的是草稿里的厂商，不是外层的当前平台**。
 * 这一条不测的话，界面看起来有选择器、保存时却被覆盖掉——那是最难发现的一类错。
 *
 * # 批次二补上的几条
 *
 * 重排把「厂商筛选」从下拉换成带条数的 chips、把余额搬进每一行、
 * 把探活工具收进面板头的两个按钮。因此新增的断言集中在**这些新结构说的还是不是真话**：
 * 条数按全量算、健康列不编造数据（后端根本没有延时）、删除要确认。
 */

type Invoke = { command: string; args: Record<string, unknown> };

function fakeRuntime(
  existing: unknown[] = [],
  extra: {
    health?: unknown[];
    /** 单家余额：`"fail"` 表示后端返回带 `error` 的结果。 */
    balance?: Record<string, unknown> | "fail";
    allBalances?: unknown[];
    /** `"fail"` 表示写配置文件抛错（`vendor:write-configs`）。 */
    writeConfigs?: "fail";
    templates?: Record<string, unknown[]>;
  } = {}
) {
  const calls: Invoke[] = [];
  window.__TAURI__ = {
    core: {
      invoke: async (_command: string, args?: unknown) => {
        const payload = args as { command?: string; args?: Record<string, unknown> } | undefined;
        const name = payload?.command ?? "";
        calls.push({ command: name, args: payload?.args ?? {} });
        if (name === "vendor:list") return existing;
        if (name === "gateway:get-vendor-health") return extra.health ?? [];
        if (name === "vendor:default-configs") return extra.templates?.[String(payload?.args?.providerId)] ?? [];
        if (name === "vendor:save") return { id: "new" };
        if (name === "vendor:set-enabled") return {};
        if (name === "vendor:delete") return true;
        if (name === "vendor:refresh-balance") {
          const vendorId = String(payload?.args?.vendorId ?? "");
          if (extra.balance === "fail") return { vendorId, error: "连不上上游" };
          return { vendorId, balance: extra.balance ?? {} };
        }
        if (name === "vendor:refresh-balances") return extra.allBalances ?? [];
        if (name === "vendor:write-configs") {
          if (extra.writeConfigs === "fail") throw new Error("磁盘只读");
          return {
            vendorId: String(payload?.args?.id ?? ""),
            written: ["~/.codex/auth.json"],
            backupRoot: "data/vendor-backups/测试家/2026"
          };
        }
        if (name === "vendor:list-models") return [];
        if (name === "vendor:read-configs") return [];
        return null;
      }
    },
    event: { listen: async () => () => undefined }
  };
  return { calls };
}

const PROVIDERS = [
  { id: "codex" as const, label: "Codex", capabilities: {} },
  { id: "claude" as const, label: "Claude Code", capabilities: {} },
  { id: "gemini" as const, label: "Gemini", capabilities: {} },
  { id: "qoder" as const, label: "Qoder CN", capabilities: {} }
];

function setup(
  overrides: { existing?: unknown[]; providerId?: AiProviderId | "" } = {},
  extra: Parameters<typeof fakeRuntime>[1] = {}
) {
  const runtime = fakeRuntime(overrides.existing ?? [], extra);
  const onError = vi.fn();
  const onNotify = vi.fn();
  const view = render(
    <VendorsPanel
      providerId={overrides.providerId === undefined ? "codex" : overrides.providerId}
      providers={PROVIDERS}
      onError={onError}
      onNotify={onNotify}
    />
  );
  const byText = (label: string) => {
    if ((label === "复制" || label === "删除") && !view.container.querySelector(".vendor-action-menu")) {
      fireEvent.click(view.container.querySelector<HTMLButtonElement>(".vendor-more-button")!);
    }
    const found = [...view.container.querySelectorAll("button")].find(
      (item) => item.textContent?.trim() === label || item.getAttribute("aria-label") === label || (label === "刷新" && item.getAttribute("aria-label")?.startsWith("刷新 "))
    );
    return found as HTMLButtonElement | undefined;
  };
  /** chips 的文字带条数（`Codex (1)`），因此按前缀找。 */
  const chip = (prefix: string) =>
    [...view.container.querySelectorAll<HTMLButtonElement>(".chip")].find((item) =>
      (item.textContent ?? "").startsWith(prefix)
    );
  const typeInto = async (input: HTMLInputElement, value: string) => {
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      setter.call(input, value);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
  };
  const inputsByPlaceholder = (part: string) =>
    [...view.container.querySelectorAll<HTMLInputElement>("input")].find((item) =>
      (item.placeholder ?? "").includes(part)
    );
  const rows = () => [...view.container.querySelectorAll("tbody tr")];
  return { view, runtime, onError, onNotify, byText, chip, typeInto, inputsByPlaceholder, rows };
}

/** 一条供应商。除 `id` 外都可覆盖——不同断言要的形状不一样。 */
function vendor(over: Partial<Record<string, unknown>> = {}) {
  return {
    id: "v1",
    providerId: "codex",
    name: "测试家",
    apiBaseUrl: "https://a/v1",
    sort: 0,
    enabled: true,
    ...over
  };
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("VendorsPanel：配置模板", () => {
  const custom = { providerId: "codex", enabled: true, targetPath: "~/.codex/config.toml", content: 'model = "custom-model"\nbase_url = "{{BASE_URL}}"' };

  it("新增加载所选厂商模板，勾选候选池和修改模板后完整提交，默认不写文件", async () => {
    const { view, byText, runtime, typeInto, inputsByPlaceholder } = setup({}, { templates: { codex: [custom] } });
    fireEvent.click(byText("新增供应商")!);
    const textarea = await view.findByRole("textbox", { name: "~/.codex/config.toml 配置模板" });
    fireEvent.change(textarea, { target: { value: 'model = "another-model"' } });
    fireEvent.click(view.getByRole("checkbox", { name: "参与候选池" }));
    await typeInto(inputsByPlaceholder("官方")!, "新供应商");
    await typeInto(inputsByPlaceholder("https://")!, "https://mock.example");
    await typeInto(inputsByPlaceholder("sk-")!, "sk-test");
    fireEvent.click(byText("创建")!);
    await waitFor(() => expect(runtime.calls.some((call) => call.command === "vendor:save")).toBe(true));
    const saved = runtime.calls.find((call) => call.command === "vendor:save")!.args;
    expect(saved.enabled).toBe(true);
    expect(saved.configs).toEqual([{ ...custom, content: 'model = "another-model"' }]);
    expect(runtime.calls.some((call) => call.command === "vendor:write-configs")).toBe(false);
  });

  it("编辑和复制都保留自定义模板，复制仍要求重新填写密钥", async () => {
    const { view, byText, runtime, typeInto, inputsByPlaceholder } = setup({ existing: [vendor({ configs: [custom] })] });
    await waitFor(() => expect(byText("编辑")).toBeTruthy());
    fireEvent.click(byText("编辑")!);
    expect((view.getByRole("textbox", { name: "~/.codex/config.toml 配置模板" }) as HTMLTextAreaElement).value).toBe(custom.content);
    fireEvent.click(byText("保存修改")!);
    await waitFor(() => expect(runtime.calls.find((call) => call.command === "vendor:save")?.args.configs).toEqual([custom]));
    await waitFor(() => expect(view.container.querySelector(".vendor-more-button")).toBeTruthy());
    expect(view.queryByRole("button", { name: "复制" })).toBeNull();
    fireEvent.click(view.getByRole("button", { name: "测试家 更多操作" }));
    fireEvent.click(view.getByRole("button", { name: "复制" }));
    expect((view.getByRole("textbox", { name: "~/.codex/config.toml 配置模板" }) as HTMLTextAreaElement).value).toBe(custom.content);
    expect(inputsByPlaceholder("sk-")!.value).toBe("");
    await typeInto(inputsByPlaceholder("sk-")!, "sk-copy");
    fireEvent.click(byText("创建")!);
    await waitFor(() => expect(runtime.calls.filter((call) => call.command === "vendor:save")).toHaveLength(2));
    const copied = runtime.calls.filter((call) => call.command === "vendor:save")[1]!.args;
    expect(copied.id).toBeUndefined();
    expect(copied.configs).toEqual([custom]);
    expect(copied.enabled).toBe(false);
  });

  it("更换厂商时换用对应模板，Qoder 明确显示无模板", async () => {
    const claude = { ...custom, providerId: "claude", targetPath: "~/.claude/settings.json", content: '{"env":{"ANTHROPIC_MODEL":"custom"}}' };
    const { view, byText, chip } = setup({}, { templates: { codex: [custom], claude: [claude] } });
    fireEvent.click(byText("新增供应商")!);
    await view.findByRole("textbox", { name: "~/.codex/config.toml 配置模板" });
    fireEvent.click(chip("Claude")!);
    await view.findByDisplayValue(claude.content);
    expect(view.queryByRole("textbox", { name: "~/.codex/config.toml 配置模板" })).toBeNull();
    fireEvent.click(chip("Qoder")!);
    await view.findByText("当前平台没有可用的兼容配置模板。");
    expect(view.queryByDisplayValue(claude.content)).toBeNull();
  });
});

describe("VendorsPanel：分页", () => {
  it("候选池切换后的慢速刷新保留当前页、表格及分页节点", async () => {
    const existing = Array.from({ length: 23 }, (_, index) =>
      vendor({ id: `v${index}`, name: `供应商 ${index + 1}` })
    );
    fakeRuntime(existing);
    const invoke = window.__TAURI__!.core!.invoke!;
    let reads = 0;
    let finish!: (value: unknown[]) => void;
    window.__TAURI__!.core!.invoke = async (command, args) => {
      const payload = args as { command?: string };
      if (payload.command === "vendor:list" && ++reads > 1) {
        return new Promise<unknown[]>((resolve) => { finish = resolve; });
      }
      return invoke(command, args);
    };
    const view = render(<VendorsPanel providerId="codex" providers={PROVIDERS} onError={vi.fn()} />);
    await waitFor(() => expect(view.container.querySelectorAll("tbody tr")).toHaveLength(10));
    fireEvent.click(view.getByRole("button", { name: "下一页" }));
    const table = view.container.querySelector("table")!;
    const pager = view.getByRole("navigation", { name: "供应商分页" });
    const poolSwitch = view.getByRole("switch", { name: "供应商 11已参与候选池" });
    fireEvent.click(poolSwitch);
    await waitFor(() => expect(finish).toBeTruthy());
    expect(view.container.querySelector("table")).toBe(table);
    expect(view.getByRole("navigation", { name: "供应商分页" })).toBe(pager);
    expect(table.querySelectorAll("tbody tr")).toHaveLength(10);
    expect(pager.textContent).toContain("第 2 / 3 页");
    expect(view.queryByText("正在读取供应商…")).toBeNull();
    await act(async () => {
      finish(existing.map((item) => item.id === "v10" ? { ...item, enabled: false } : item));
    });
    expect(view.getByRole("switch", { name: "供应商 11未参与候选池" })).toBe(poolSwitch);
    expect(view.container.querySelector("table")).toBe(table);
    expect(pager.textContent).toContain("第 2 / 3 页");
  });

  it("默认每页 10 条，支持首末页、上一页和下一页", async () => {
    const { view, rows } = setup({
      existing: Array.from({ length: 23 }, (_, index) => vendor({ id: `v${index}`, name: `供应商 ${index + 1}` }))
    });
    await waitFor(() => expect(rows()).toHaveLength(10));
    const pager = view.getByRole("navigation", { name: "供应商分页" });
    expect(pager.textContent).toContain("共 23 条");
    expect((view.getByRole("button", { name: "上一页" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(view.getByRole("button", { name: "下一页" }));
    expect(rows()[0]!.textContent).toContain("供应商 11");
    fireEvent.click(view.getByRole("button", { name: "最后一页" }));
    expect(rows()).toHaveLength(3);
    expect(rows()[0]!.textContent).toContain("供应商 21");
    expect((view.getByRole("button", { name: "下一页" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(view.getByRole("button", { name: "上一页" }));
    expect(rows()[0]!.textContent).toContain("供应商 11");
    fireEvent.click(view.getByRole("button", { name: "第一页" }));
    expect(rows()[0]!.textContent).toContain("供应商 1");
  });

  it("切换每页数量和厂商筛选都回到第一页，统计仍按完整筛选结果计算", async () => {
    const { view, rows, chip } = setup({
      existing: Array.from({ length: 26 }, (_, index) => vendor({
        id: `v${index}`, name: `供应商 ${index + 1}`, providerId: index < 23 ? "codex" : "claude"
      }))
    });
    await waitFor(() => expect(rows()).toHaveLength(10));
    fireEvent.click(view.getByRole("button", { name: "最后一页" }));
    fireEvent.change(view.getByRole("combobox", { name: "每页数量" }), { target: { value: "20" } });
    expect(rows()).toHaveLength(20);
    expect(view.container.textContent).toContain("第 1 / 2 页");
    fireEvent.click(view.getByRole("button", { name: "下一页" }));
    fireEvent.click(chip("Claude")!);
    expect(rows()).toHaveLength(3);
    expect(view.container.textContent).toContain("第 1 / 1 页");
    expect(view.container.textContent).toContain("3 个 · 3 个启用");
    expect(chip("Codex")!.textContent).toBe("Codex (23)");
  });

  it("删除最后一页的唯一记录后回到仍存在的末页", async () => {
    const existing = Array.from({ length: 11 }, (_, index) => vendor({ id: `v${index}`, name: `供应商 ${index + 1}` }));
    const { view, rows } = setup({ existing });
    vi.spyOn(window, "confirm").mockReturnValue(true);
    await waitFor(() => expect(rows()).toHaveLength(10));
    fireEvent.click(view.getByRole("button", { name: "下一页" }));
    expect(rows()).toHaveLength(1);
    // 下一次读取模拟后端删除后的清单。
    existing.pop();
    fireEvent.click(view.getByRole("button", { name: /更多操作/ }));
    fireEvent.click(view.getByRole("button", { name: "删除" }));
    await waitFor(() => expect(rows()).toHaveLength(10));
    expect(view.container.textContent).toContain("第 1 / 1 页");
  });
});

describe("VendorsPanel：面板头", () => {
  it("头部就是那两个动作，且都在面板头里", async () => {
    const { view, byText } = setup({ existing: [vendor()] });
    await waitFor(() => expect(byText("查余额")).toBeTruthy());
    for (const label of ["查余额", "新增供应商"]) {
      expect(byText(label), label).toBeTruthy();
    }
    const header = view.container.querySelector(".panel-header")!;
    expect(header.textContent).toContain("供应商管理");
    for (const label of ["查余额", "新增供应商"]) {
      expect(header.textContent, label).toContain(label);
    }
  });
});

describe("VendorsPanel：模型厂商选择", () => {
  it("表单里有「模型厂商」，四个厂商都在", async () => {
    const { byText } = setup();
    await act(async () => {
      byText("新增供应商")!.click();
    });
    const text = document.body.textContent ?? "";
    expect(text).toContain("模型厂商");
    for (const label of ["Codex", "Claude Code", "Gemini", "Qoder CN"]) {
      expect(text).toContain(label);
    }
  });

  it("「新增」时厂商**默认取当前平台**——多数情况就是它", async () => {
    const { byText, runtime, inputsByPlaceholder, typeInto } = setup({ providerId: "gemini" });
    await act(async () => {
      byText("新增供应商")!.click();
    });
    await typeInto(inputsByPlaceholder("官方")!, "测试家");
    await typeInto(inputsByPlaceholder("https://")!, "https://a/v1");
    await typeInto(inputsByPlaceholder("sk-")!, "sk-test");
    await act(async () => {
      byText("创建")!.click();
    });

    await waitFor(() => {
      const call = runtime.calls.find((item) => item.command === "vendor:save");
      expect(call?.args.providerId).toBe("gemini");
    });
  });

  it("**改了厂商之后，保存用的是改后的那一个**（不是外层的当前平台）", async () => {
    // 这一条是这个缺陷的核心：界面有选择器、保存时被外层覆盖，是最难发现的错。
    const { byText, runtime, inputsByPlaceholder, typeInto } = setup({ providerId: "codex" });
    await act(async () => {
      byText("新增供应商")!.click();
    });

    // 点「Claude Code」把厂商改成 claude。
    await act(async () => {
      byText("Claude Code")!.click();
    });

    await typeInto(inputsByPlaceholder("官方")!, "中转站");
    await typeInto(inputsByPlaceholder("https://")!, "https://b/v1");
    await typeInto(inputsByPlaceholder("sk-")!, "sk-test");
    await act(async () => {
      byText("创建")!.click();
    });

    await waitFor(() => {
      const call = runtime.calls.find((item) => item.command === "vendor:save");
      expect(call?.args.providerId).toBe("claude");
      // 而外层的当前平台仍然是 codex —— 两者不同，才说明真的用了草稿里的值。
      expect(call?.args.providerId).not.toBe("codex");
    });
  });

  it("没有 API 地址时点创建：就地报错、不提交（不再静默禁用）", async () => {
    const { byText, view, runtime, inputsByPlaceholder, typeInto } = setup();
    await act(async () => {
      byText("新增供应商")!.click();
    });
    await typeInto(inputsByPlaceholder("官方")!, "只有名字");
    // 按钮可点（点不动且无提示正是用户报的「没反应」）；报错落在字段旁边。
    expect(byText("创建")!.disabled).toBe(false);
    await act(async () => {
      byText("创建")!.click();
    });
    expect(view.container.textContent).toContain("请输入 API 请求地址。");
    expect(runtime.calls.some((item) => item.command === "vendor:save")).toBe(false);
  });
});

describe("VendorsPanel：跨平台列表与厂商过滤", () => {
  const mixed = [
    vendor({ id: "c1", providerId: "codex", name: "Codex 家" }),
    vendor({ id: "c2", providerId: "codex", name: "Codex 备用", enabled: false }),
    vendor({ id: "a1", providerId: "claude", name: "Claude 家" })
  ];

  it("默认列出**全部平台**的供应商（不传 providerId）", async () => {
    const { runtime } = setup();
    await waitFor(() => {
      const call = runtime.calls.find((item) => item.command === "vendor:list");
      expect(call).toBeTruthy();
      // 全部平台时**不带** providerId——带了就只有当前平台那几家。
      expect(call!.args).not.toHaveProperty("providerId");
    });
  });

  it("厂商过滤是 chips，默认选中「全部 (N)」，每家的条数按全量算", async () => {
    const { chip } = setup({ existing: mixed });
    // 等的是**算完条数**之后的那一份：chips 着一份空列表也会先渲染出来，
    // 只等「按钮在不在」会在取数落地之前就往下走（这条断言曾因此偶发失败）。
    await waitFor(() => expect(chip("全部")?.textContent).toBe("全部 (3)"));
    // 条数只有在**一次取全量**之后才算得出来——这正是这里不断言「筛选发给后端」的原因。
    expect(chip("Codex")!.textContent).toBe("Codex (2)");
    expect(chip("Claude Code")!.textContent).toBe("Claude Code (1)");
    expect(chip("Gemini")!.textContent).toBe("Gemini (0)");
    expect(chip("全部")!.getAttribute("aria-pressed")).toBe("true");
  });

  it("选一家厂商只显示它那几家；切回「全部」都回来", async () => {
    const { view, chip, rows } = setup({ existing: mixed });
    await waitFor(() => expect(rows()).toHaveLength(3));

    await act(async () => {
      chip("Claude Code")!.click();
    });
    expect(rows()).toHaveLength(1);
    expect(view.container.textContent).toContain("Claude 家");
    expect(view.container.textContent).not.toContain("Codex 备用");
    expect(chip("Claude Code")!.getAttribute("aria-pressed")).toBe("true");

    await act(async () => {
      chip("全部")!.click();
    });
    expect(rows()).toHaveLength(3);
  });

  it("厂商筛选那一组在无障碍树里有名字（旧平台的下拉带同一个 aria-label）", async () => {
    // chips 换掉了 `<select aria-label="按平台筛选供应商">`，这条信息不能跟着控件一起丢：
    // `role="group"` 没有名字时读屏只念出一串孤立的按钮。
    const { view } = setup({ existing: mixed });
    // 顺带等条数算完（空列表下这一组也已经在 DOM 里，只等「组在不在」会在取数落地前
    // 就继续往下走）。
    const group = await waitFor(() => {
      const found = view.container.querySelector('[role="group"][aria-label="按厂商筛选供应商"]');
      if (!found || !(found.textContent ?? "").includes("全部 (3)")) {
        throw new Error("还没有带名字、且算完条数的 chips 组");
      }
      return found;
    });
    expect(group.textContent).toContain("全部 (3)");
  });

  it("筛选结果为空时说清是**这个厂商**没有，而不是「一个都没有」", async () => {
    const { view, chip } = setup({ existing: mixed });
    await waitFor(() => expect(chip("Gemini (0)")).toBeTruthy());
    await act(async () => {
      chip("Gemini")!.click();
    });
    await waitFor(() => {
      expect(view.container.textContent).toContain("Gemini 下还没有供应商");
    });
  });

  it("表格里有「厂商」列，显示的是**平台名**而不是编号", async () => {
    // 列表是跨平台的，不标出来同名供应商分不清；显示编号则没人认得出。
    const { view } = setup({
      existing: [vendor({ id: "v1", providerId: "claude", name: "中转站" })]
    });
    const table = await waitFor(() => {
      const found = view.container.querySelector("table.data-table");
      if (!found) throw new Error("还没有表格");
      return found;
    });
    expect(table.textContent).toContain("厂商");
    expect(table.textContent).toContain("Claude Code");
  });

  it("编辑时**不回填密钥**——接口本来就不返回它", async () => {
    const { byText, view } = setup({ existing: [vendor({ name: "官方" })] });
    await waitFor(() => {
      expect(byText("编辑")).toBeTruthy();
    });
    await act(async () => {
      byText("编辑")!.click();
    });
    const key = [...view.container.querySelectorAll<HTMLInputElement>("input")].find((item) =>
      (item.placeholder ?? "").includes("不修改")
    );
    expect(key).toBeTruthy();
    expect(key!.value).toBe("");
  });

  it("编辑时带出这条自己的厂商，而不是当前平台", async () => {
    const { byText, runtime, inputsByPlaceholder, typeInto } = setup({
      providerId: "codex",
      existing: [vendor({ id: "v1", providerId: "qoder", name: "Qoder 官方" })]
    });
    await waitFor(() => expect(byText("编辑")).toBeTruthy());
    await act(async () => {
      byText("编辑")!.click();
    });
    await typeInto(inputsByPlaceholder("官方")!, "Qoder 官方改");
    await act(async () => {
      byText("保存修改")!.click();
    });
    await waitFor(() => {
      const call = runtime.calls.find((item) => item.command === "vendor:save");
      expect(call?.args.providerId).toBe("qoder");
      expect(call?.args.id).toBe("v1");
    });
  });
});

describe("VendorsPanel：表格列", () => {
  it("费率值：`$0.15`；未设置是 `-`（与 0 不是一回事）", () => {
    expect(rateText(undefined)).toBe("-");
    expect(rateText(0)).toBe("$0");
    expect(rateText(0.15)).toBe("$0.15");
  });

  it("列表移除费率列，计费设置仍在编辑表单", async () => {
    const { view, byText } = setup({ existing: [vendor({ pricing: { inputPerMillionUsd: 0.15, outputPerMillionUsd: 0.6 } })] });
    await waitFor(() => expect(view.container.querySelector("table")).toBeTruthy());
    const heads = [...view.container.querySelectorAll("thead th")].map((item) => item.textContent);
    expect(heads).not.toContain("输入费率");
    expect(heads).not.toContain("输出费率");
    fireEvent.click(byText("编辑")!);
    expect(view.getByDisplayValue("0.15")).toBeTruthy();
    expect(view.getByDisplayValue("0.6")).toBeTruthy();
  });

  it("列表保留名称和厂商，移除端点列；编辑仍能查看 API 地址", async () => {
    const { view } = setup({
      existing: [vendor({ name: "官方", apiBaseUrl: "https://api.example.com/v1" })]
    });
    await waitFor(() => expect(view.container.querySelector("table.data-table")).toBeTruthy());
    const heads = [...view.container.querySelectorAll("thead th")].map((item) =>
      item.textContent?.trim()
    );
    expect(heads).toContain("名称");
    expect(heads).not.toContain("端点");
    const cells = [...view.container.querySelectorAll("tbody tr td")];
    expect(cells[0]!.textContent).toBe("官方");
    expect(cells[1]!.textContent).toBe("Codex");
    expect(view.container.querySelector("table")!.textContent).not.toContain("https://api.example.com/v1");
    fireEvent.click(view.getByRole("button", { name: "编辑" }));
    expect(view.getByDisplayValue("https://api.example.com/v1")).toBeTruthy();
  });

  it("「余额」与「操作」分列且余额在左（2026-10-02 实机反馈：合并列拆开）", async () => {
    // 此前是一格「操作 / 余额」：余额状态机与三个操作按钮叠在同一个单元格里，
    // 读起来分不清哪块是哪块。拆成两列后，列头必须各自如实。
    const { view } = setup({ existing: [vendor()] });
    await waitFor(() => expect(view.container.querySelector("table.data-table")).toBeTruthy());
    const heads = [...view.container.querySelectorAll("thead th")].map((item) => item.textContent);
    const balanceAt = heads.indexOf("余额");
    const actionsAt = heads.indexOf("操作");
    expect(balanceAt).toBeGreaterThanOrEqual(0);
    expect(actionsAt).toBeGreaterThan(balanceAt);
  });

  it("**健康列不编造数据**：有快照才显示，没有就留空（后端根本不记延时）", async () => {
    const circuitUntil = new Date(Date.now() + 3 * 60_000).toISOString();
    const { view } = setup(
      {
        existing: [
          vendor({ id: "v1", name: "有健康" }),
          vendor({ id: "v2", name: "没健康" })
        ]
      },
      {
        health: [
          {
            vendorId: "v1",
            providerId: "codex",
            status: "open",
            failureCount: 3,
            successCount: 1,
            failureRate: 0.75,
            lastSuccessAt: new Date().toISOString(),
            circuitUntil
          }
        ]
      }
    );
    await waitFor(() => expect(view.container.querySelector("table.data-table")).toBeTruthy());
    const text = view.container.textContent ?? "";
    expect(text).toContain("已熔断");
    expect(text).toContain("失败率 75%");
    expect(text).toContain("最近成功");
    expect(text).toContain("熔断剩余 3 分钟");
    // 延时这类后端不提供的东西**不许出现**（demo 里那 142ms 是编的）。
    expect(text).not.toMatch(/\d+\s*ms/);
    // 没有快照的那一行，健康那一格是空的（列序：名称/厂商/健康/健康状况）。
    const cells = [...view.container.querySelectorAll("tbody tr")][1]!.querySelectorAll("td");
    expect(cells[2]!.textContent).toBe("");
  });

  it("候选池状态是**开关**；顺位最前的那家带「优先」标记", async () => {
    const { view } = setup({
      existing: [
        vendor({ id: "v1", name: "第一顺位", sort: 1 }),
        vendor({ id: "v2", name: "第二顺位", sort: 2 }),
        vendor({ id: "v3", name: "被禁用", sort: 0, enabled: false })
      ]
    });
    await waitFor(() => expect(view.container.querySelector("table.data-table")).toBeTruthy());
    const switches = [...view.container.querySelectorAll<HTMLButtonElement>('[role="switch"]')];
    expect(switches).toHaveLength(3);
    // 开关承载状态：无障碍语义与旧平台一致（role=switch + aria-checked + 可读名）。
    expect(switches[0]!.getAttribute("aria-checked")).toBe("true");
    expect(switches[1]!.getAttribute("aria-checked")).toBe("true");
    expect(switches[2]!.getAttribute("aria-checked")).toBe("false");
    expect(switches[0]!.getAttribute("aria-label")).toBe("第一顺位已参与候选池");
    expect(switches[2]!.getAttribute("aria-label")).toBe("被禁用未参与候选池");
    // 「优先」属于**启用且顺位最前**的那家：被禁用的那家排序更靠前也不算。
    const rows = [...view.container.querySelectorAll("tbody tr")];
    expect(rows[0]!.textContent).toContain("优先");
    expect(rows[1]!.textContent).not.toContain("优先");
    expect(rows[2]!.textContent).not.toContain("优先");
  });
});

describe("VendorsPanel：余额行内状态", () => {
  it("没查过时说「未获取」，不是 0 也不是空白", async () => {
    const { view } = setup({ existing: [vendor()] });
    await waitFor(() => expect(view.container.querySelector("table.data-table")).toBeTruthy());
    expect(view.container.textContent).toContain("未获取");
    expect(view.container.textContent).not.toContain("余额 未获取");
  });

  it("`balanceLabel` 的三态与「已过期」", () => {
    expect(balanceLabel(undefined)).toBe("未获取");
    expect(balanceLabel({ status: "loading" })).toBe("查询中…");
    expect(balanceLabel({ status: "ok", balance: { remaining: 12.5, unit: "USD" } })).toBe("12.5 USD");
    // 上游没提供余额与「这次查失败」都要说成查询失败——显示 0 会让人以为钱用完了。
    expect(balanceLabel({ status: "ok", balance: {} })).toBe("查询失败");
    expect(balanceLabel({ status: "error" })).toBe("查询失败");
    // 失败时留着上一次的数字，标成过期：抹掉它等于把「查不到」说成「没有」。
    expect(
      balanceLabel({ status: "error", balance: { remaining: 3, unit: "USD" }, error: "超时" })
    ).toBe("3 USD（已过期）");
  });

  it("**`isValid === false` 要单独说出来**（旧平台的「密钥可能无效」在重排里不能丢）", () => {
    // 它**不是**「查询失败」的同义词：金额查得到时也会带上它，而两句指向的下一步动作
    // 完全不同（一个去充值、一个去查密钥）。
    expect(
      balanceLabel({ status: "ok", balance: { remaining: 12.5, unit: "USD", isValid: false } })
    ).toBe("12.5 USD（密钥可能无效）");
    // 连数字都没有时，「查询失败」这句仍然要留着——两句话说的是两件事。
    expect(balanceLabel({ status: "ok", balance: { isValid: false } })).toBe(
      "查询失败（密钥可能无效）"
    );
    // 密钥验证过（`true`）或后端没发这个字段的，都不该多这一句。
    expect(balanceLabel({ status: "ok", balance: { remaining: 1, isValid: true } })).toBe("1");
    expect(balanceLabel({ status: "ok", balance: { remaining: 1 } })).toBe("1");
  });

  it("单家刷新：点「刷新」拿到金额", async () => {
    const { view, byText, onNotify } = setup(
      { existing: [vendor()] },
      { balance: { remaining: 12.5, unit: "USD" } }
    );
    await waitFor(() => expect(byText("刷新")).toBeTruthy());
    await act(async () => {
      byText("刷新")!.click();
    });
    await waitFor(() => {
      expect(view.container.textContent).toContain("余额 12.5 USD");
      expect(onNotify).toHaveBeenCalledWith(expect.stringContaining("余额查询成功"));
    });
  });

  it("**行内那格也要带「（密钥可能无效）」**（信号落到用户看得见的地方）", async () => {
    const { view, byText } = setup(
      { existing: [vendor()] },
      { balance: { remaining: 0.5, unit: "USD", isValid: false } }
    );
    await waitFor(() => expect(byText("刷新")).toBeTruthy());
    await act(async () => {
      byText("刷新")!.click();
    });
    await waitFor(() => {
      expect(view.container.textContent).toContain("余额 0.5 USD（密钥可能无效）");
    });
  });

  it("查询余额失败时报错，而不是安静地显示 0", async () => {
    const { view, byText, onError } = setup({ existing: [vendor()] }, { balance: "fail" });
    await waitFor(() => expect(byText("刷新")).toBeTruthy());
    await act(async () => {
      byText("刷新")!.click();
    });
    await waitFor(() => {
      expect(onError).toHaveBeenCalledWith(expect.stringContaining("余额查询失败"));
      expect(view.container.textContent).toContain("余额 查询失败");
    });
    expect(view.container.textContent).not.toContain("余额 0");
  });

  it("「查余额」刷全部，并把每一家的结果落到它自己那一行", async () => {
    const { view, byText, onNotify } = setup(
      {
        existing: [
          vendor({ id: "v1", name: "第一家" }),
          vendor({ id: "v2", name: "第二家" })
        ]
      },
      {
        allBalances: [
          { vendorId: "v1", balance: { remaining: 5, unit: "USD" } },
          { vendorId: "v2", error: "没有余额接口" }
        ]
      }
    );
    await waitFor(() => expect(byText("查余额")).toBeTruthy());
    await act(async () => {
      byText("查余额")!.click();
    });
    await waitFor(() => {
      const rows = [...view.container.querySelectorAll("tbody tr")];
      expect(rows[0]!.textContent).toContain("余额 5 USD");
      expect(rows[1]!.textContent).toContain("余额 查询失败");
      expect(onNotify).toHaveBeenCalledWith("1 家成功，1 家未提供或查询失败。");
    });
    expect(view.container.querySelector(".export-hint")).toBeNull();
  });
});

describe("VendorsPanel：删除需要确认", () => {
  it("**取消确认时什么都不发生**", async () => {
    const { byText, runtime, onNotify } = setup({ existing: [vendor()] });
    await waitFor(() => expect(byText("删除")).toBeTruthy());
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    await act(async () => {
      byText("删除")!.click();
    });
    expect(confirmSpy).toHaveBeenCalledTimes(1);
    // 确认文案要点名是哪一家，并说清「已写入的 CLI 配置不会还原」。
    expect(String(confirmSpy.mock.calls[0]![0])).toContain("测试家");
    expect(String(confirmSpy.mock.calls[0]![0])).toContain("不会还原已写入的 CLI 配置");
    expect(runtime.calls.some((item) => item.command === "vendor:delete")).toBe(false);
    expect(onNotify).not.toHaveBeenCalled();
  });

  it("确认后才删，并说一声", async () => {
    const { byText, runtime, onNotify } = setup({ existing: [vendor()] });
    await waitFor(() => expect(byText("删除")).toBeTruthy());
    vi.spyOn(window, "confirm").mockReturnValue(true);
    await act(async () => {
      byText("删除")!.click();
    });
    await waitFor(() => {
      expect(runtime.calls.some((item) => item.command === "vendor:delete")).toBe(true);
      expect(onNotify).toHaveBeenCalledWith(expect.stringContaining("测试家"));
    });
  });
});

describe("VendorsPanel：编辑中点「新增」先问一次", () => {
  // `startNew` 会整体覆盖草稿，与删除同一类不可逆动作（刚填的东西直接没了）。
  it("**取消时不丢草稿**——表单还停在原来那一条的编辑态", async () => {
    const { byText } = setup({ existing: [vendor({ name: "官方" })] });
    await waitFor(() => expect(byText("编辑")).toBeTruthy());
    await act(async () => {
      byText("编辑")!.click();
    });
    expect(byText("保存修改")).toBeTruthy();

    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    await act(async () => {
      byText("新增供应商")!.click();
    });
    expect(confirmSpy).toHaveBeenCalledTimes(1);
    expect(String(confirmSpy.mock.calls[0]![0])).toContain("放弃当前修改并新建");
    // 草稿没被换掉：按钮仍是「保存修改」而不是「创建」。
    expect(byText("保存修改")).toBeTruthy();
    expect(byText("创建")).toBeFalsy();
  });

  it("确认之后才换成一张新草稿", async () => {
    const { byText } = setup({ existing: [vendor({ name: "官方" })] });
    await waitFor(() => expect(byText("编辑")).toBeTruthy());
    await act(async () => {
      byText("编辑")!.click();
    });
    vi.spyOn(window, "confirm").mockReturnValue(true);
    await act(async () => {
      byText("新增供应商")!.click();
    });
    expect(byText("创建")).toBeTruthy();
    expect(byText("保存修改")).toBeFalsy();
  });
});

describe("VendorsPanel：启用与禁用的区别必须是说得出的话", () => {
  it("两个动作各自的后果写在**开关提示**里（一个写文件、一个只翻位）", async () => {
    const { view } = setup({
      existing: [
        vendor({ id: "v1", name: "开着的" }),
        vendor({ id: "v2", name: "关着的", enabled: false })
      ]
    });
    await waitFor(() => {
      expect(view.container.querySelectorAll('[role="switch"]')).toHaveLength(2);
    });
    const switches = [...view.container.querySelectorAll<HTMLButtonElement>('[role="switch"]')];
    // **开关只管候选池**：两个方向都不碰用户的 CLI 配置文件（写文件是表单里的显式勾选）。
    expect(switches[0]!.getAttribute("title")).toContain("移出候选池");
    expect(switches[0]!.getAttribute("title")).toContain("不改动你的 CLI 配置文件");
    expect(switches[1]!.getAttribute("title")).toContain("加入候选池");
  });

  it("表单底部的说明同样如实分开说（那句话此前与实现相反）", async () => {
    const { byText, view } = setup({ existing: [vendor()] });
    await waitFor(() => expect(byText("编辑")).toBeTruthy());
    await act(async () => {
      byText("编辑")!.click();
    });
    const note = view.container.querySelector(".editor .row-meta")!.textContent ?? "";
    expect(note).toContain("写进你的 CLI 配置文件");
    expect(note).toContain("只翻启用位");
  });
});

/* ── A3 费率 / A5 高级 JSON ─────────────────────────────
 *
 * 解析层单独测：它是「填错会静默算出错数字」的那一层。
 * 后端 `VendorPricing` 两个字段都可选，`BalanceQueryConfig` / `ModelQueryConfig`
 * 是结构化对象——JSON 文本框是旧平台的做法，照做。
 */

describe("parsePrice：费率", () => {
  it("空串 = 未设置（**不是 0**）", () => {
    expect(parsePrice("", "输入费率")).toEqual({ ok: true, value: undefined });
    expect(parsePrice("   ", "输入费率")).toEqual({ ok: true, value: undefined });
  });

  it("「0」是免费，与未设置是两回事", () => {
    expect(parsePrice("0", "输入费率")).toEqual({ ok: true, value: 0 });
  });

  it("正常小数", () => {
    expect(parsePrice("0.55", "输入费率")).toEqual({ ok: true, value: 0.55 });
  });

  it("**0.55 与 2.2 必须被接受**——二进制不精确，精确比较会把它们判成三位小数", () => {
    // 后端原来就是精确比较，实测把 0.55 拒了。这条与后端那条测试成对。
    for (const price of ["0.55", "2.2", "0.29", "1.15", "3.75"]) {
      const result = parsePrice(price, "输入费率");
      expect(result.ok, price).toBe(true);
    }
  });

  it("真的三位小数要拒（容差不是放开）", () => {
    for (const price of ["1.234", "0.555", "9.999"]) {
      const result = parsePrice(price, "输入费率");
      expect(result.ok, price).toBe(false);
      expect(result.ok === false && result.message, price).toContain("2 位小数");
    }
  });

  it("**非数字要报错，不能悄悄变成 0**", () => {
    // 悄悄归零会让成本估算凭空变成 0，而且用户完全看不出来。
    const result = parsePrice("abc", "输入费率");
    expect(result.ok).toBe(false);
    expect(result.ok === false && result.message).toContain("输入费率");
  });

  it("负数要报错", () => {
    expect(parsePrice("-1", "输出费率").ok).toBe(false);
  });
});

describe("parseAdvancedJson：高级 JSON", () => {
  it("空串 = 不设置", () => {
    expect(parseAdvancedJson("", "余额查询配置")).toEqual({ ok: true, value: undefined });
  });

  it("对象可用", () => {
    expect(parseAdvancedJson('{"endpoint":"/x"}', "余额查询配置")).toEqual({
      ok: true,
      value: { endpoint: "/x" }
    });
  });

  it("**格式错时文案是「JSON 格式无效」**（旧平台的原话）", () => {
    const result = parseAdvancedJson("{oops", "余额查询配置");
    expect(result.ok).toBe(false);
    expect(result.ok === false && result.message).toContain("JSON 格式无效");
  });

  it("**数组与字面量要报「必须是 JSON 对象」**——后端要的是对象", () => {
    for (const text of ["[]", "[1,2]", "null", "42", '"x"']) {
      const result = parseAdvancedJson(text, "模型查询配置");
      expect(result.ok, text).toBe(false);
      expect(result.ok === false && result.message, text).toContain("必须是 JSON 对象");
    }
  });

  it("文案里带上是哪一项，两个框分得清", () => {
    const result = parseAdvancedJson("{", "模型查询配置");
    expect(result.ok === false && result.message.startsWith("模型查询配置")).toBe(true);
  });
});

describe("VendorsPanel：费率与高级 JSON 接线", () => {
  it("**两条费率会按 pricing 结构提交**（后端字段名是 inputPerMillionUsd）", async () => {
    const { byText, runtime, inputsByPlaceholder, typeInto } = setup();
    await act(async () => {
      byText("新增供应商")!.click();
    });
    await typeInto(inputsByPlaceholder("官方")!, "带费率的");
    await typeInto(inputsByPlaceholder("https://")!, "https://a/v1");
    await typeInto(inputsByPlaceholder("sk-")!, "sk-test");
    await typeInto(inputsByPlaceholder("0.55")!, "0.55");
    await typeInto(inputsByPlaceholder("2.2")!, "2.2");
    await act(async () => {
      byText("创建")!.click();
    });
    await waitFor(() => {
      const call = runtime.calls.find((item) => item.command === "vendor:save");
      expect(call?.args.pricing).toEqual({
        inputPerMillionUsd: 0.55,
        outputPerMillionUsd: 2.2
      });
    });
  });

  it("**两个费率都留空时整个 pricing 不传**（空对象是「设置了但什么都没有」）", async () => {
    const { byText, runtime, inputsByPlaceholder, typeInto } = setup();
    await act(async () => {
      byText("新增供应商")!.click();
    });
    await typeInto(inputsByPlaceholder("官方")!, "无费率");
    await typeInto(inputsByPlaceholder("https://")!, "https://a/v1");
    await typeInto(inputsByPlaceholder("sk-")!, "sk-test");
    await act(async () => {
      byText("创建")!.click();
    });
    await waitFor(() => {
      const call = runtime.calls.find((item) => item.command === "vendor:save");
      expect(call?.args).not.toHaveProperty("pricing");
    });
  });

  it("JSON 填错时**整体不提交**，并给出是哪一项", async () => {
    const { byText, view, runtime, onError, inputsByPlaceholder, typeInto } = setup();
    await act(async () => {
      byText("新增供应商")!.click();
    });
    await typeInto(inputsByPlaceholder("官方")!, "JSON 错");
    await typeInto(inputsByPlaceholder("https://")!, "https://a/v1");
    // 必填先过（新增 KEY），这样下面测到的才是 JSON 那一道校验。
    await typeInto(inputsByPlaceholder("sk-")!, "sk-test");
    // **在本次渲染的容器里找**，不要在 `document` 里找：那个范围包含别的测试
    // 留下的节点，写进去的值不会进当前组件的 state——测试会以「没报错」的形式失败，
    // 看起来像产品缺陷，其实是测试读错了元素。
    const area = [...view.container.querySelectorAll("textarea")].find((item) =>
      (item.placeholder ?? "").includes("endpoint")
    )!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(
        HTMLTextAreaElement.prototype,
        "value"
      )!.set!;
      setter.call(area, "{oops");
      area.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {
      byText("创建")!.click();
    });
    expect(onError).toHaveBeenCalledWith(expect.stringContaining("JSON 格式无效"));
    // **没有发出去**：提交一半再报错，用户不知道哪一项存进去了。
    expect(runtime.calls.some((item) => item.command === "vendor:save")).toBe(false);
  });

  it("编辑时回填费率与高级 JSON（不回填就会被「保存修改」清掉）", async () => {
    const { byText, view, inputsByPlaceholder } = setup({
      existing: [
        vendor({
          name: "官方",
          pricing: { inputPerMillionUsd: 1.5 },
          balanceQuery: { endpoint: "/balance" }
        })
      ]
    });
    await waitFor(() => expect(byText("编辑")).toBeTruthy());
    await act(async () => {
      byText("编辑")!.click();
    });
    expect((inputsByPlaceholder("0.55") as HTMLInputElement).value).toBe("1.5");
    const area = [...view.container.querySelectorAll("textarea")].find((item) =>
      (item.placeholder ?? "").includes("endpoint")
    ) as HTMLTextAreaElement;
    expect(area.value).toContain("/balance");
  });
});

describe("VendorsPanel：保存时写入 CLI 配置文件（二轮：写文件与候选池解耦）", () => {
  it("**默认不勾选：只保存，不写文件**——写文件必须是一次显式动作", async () => {
    const { byText, runtime, inputsByPlaceholder, typeInto } = setup();
    await act(async () => {
      byText("新增供应商")!.click();
    });
    await typeInto(inputsByPlaceholder("官方")!, "不写文件");
    await typeInto(inputsByPlaceholder("https://")!, "https://a/v1");
    await typeInto(inputsByPlaceholder("sk-")!, "sk-test");
    await act(async () => {
      byText("创建")!.click();
    });
    await waitFor(() => {
      expect(runtime.calls.some((item) => item.command === "vendor:save")).toBe(true);
    });
    expect(runtime.calls.some((item) => item.command === "vendor:write-configs")).toBe(false);
  });

  it("**勾选后保存：写配置模板（带备份）并在提示里如实说**", async () => {
    const { byText, view, runtime, onNotify, inputsByPlaceholder, typeInto } = setup();
    await act(async () => {
      byText("新增供应商")!.click();
    });
    await typeInto(inputsByPlaceholder("官方")!, "要写文件");
    await typeInto(inputsByPlaceholder("https://")!, "https://a/v1");
    await typeInto(inputsByPlaceholder("sk-")!, "sk-test");
    const box = (view.getByRole("checkbox", { name: "保存时写入 CLI 配置文件（写前备份）" }) as HTMLInputElement)!;
    expect(box.checked).toBe(false);
    await act(async () => {
      box.click();
    });
    await act(async () => {
      byText("创建")!.click();
    });
    await waitFor(() => {
      const call = runtime.calls.find((item) => item.command === "vendor:write-configs");
      // 写文件带的是**刚保存出来的**那条记录。
      expect(call?.args.id).toBe("new");
    });
    await waitFor(() => {
      expect(onNotify.mock.calls.some((call) => String(call[0]).includes("备份"))).toBe(true);
    });
    expect(onNotify.mock.calls.some((call) => String(call[0]).includes("已写入 1 个配置文件"))).toBe(
      true
    );
  });

  it("写文件失败说明记录已保存，且不被成功提示覆盖", async () => {
    const { byText, view, onError, onNotify, inputsByPlaceholder, typeInto } = setup({}, {
      writeConfigs: "fail"
    });
    await act(async () => {
      byText("新增供应商")!.click();
    });
    await typeInto(inputsByPlaceholder("官方")!, "写失败");
    await typeInto(inputsByPlaceholder("https://")!, "https://a/v1");
    await typeInto(inputsByPlaceholder("sk-")!, "sk-test");
    await act(async () => {
      (view.getByRole("checkbox", { name: "保存时写入 CLI 配置文件（写前备份）" }) as HTMLInputElement)!.click();
    });
    await act(async () => {
      byText("创建")!.click();
    });
    await waitFor(() => {
      expect(onError).toHaveBeenCalledWith(
        expect.stringContaining("供应商已保存，但写入配置文件失败")
      );
    });
    expect(onNotify).not.toHaveBeenCalled();
  });
});

describe("balanceText：余额的一句话", () => {
  it("有剩余额度时显示数字与单位", () => {
    expect(balanceText({ remaining: 12.5, unit: "USD" })).toBe("12.5 USD");
  });

  it("只有已用量时显示已用", () => {
    expect(balanceText({ used: 3, unit: "USD" })).toBe("已用 3 USD");
  });

  it("只有套餐名时显示套餐名", () => {
    expect(balanceText({ planName: "Pro" })).toBe("Pro");
  });

  it("**上游没提供余额时说「未提供」，不是 0**", () => {
    // 显示 `0` 会让用户以为钱用完了，而事实是那家中转站根本不返回余额。
    expect(balanceText({})).toBe("未提供");
    expect(balanceText(undefined)).toBe("未提供");
  });
});

describe("VendorsPanel：必填标记与就地校验（2026-10-02 实机反馈）", () => {
  it("**必填字段带红标**：名称 / API 地址 / API Key（新建时）", async () => {
    const { byText, view } = setup();
    await act(async () => {
      byText("新增供应商")!.click();
    });
    const labeled = (label: string) =>
      [...view.container.querySelectorAll(".field")].find((item) =>
        (item.querySelector(".field-label")?.textContent ?? "").startsWith(label)
      );
    for (const label of ["名称", "API 地址", "API Key"]) {
      expect(labeled(label)?.querySelector(".field-required"), label).toBeTruthy();
    }
    // 标记不多不少：模型厂商/排序/费率不是必填。
    expect(view.container.querySelectorAll(".field-required")).toHaveLength(3);
  });

  it("编辑时 API Key 不是必填（留空=沿用原密钥），红标只出现两处", async () => {
    const { byText, view } = setup({ existing: [vendor({ name: "官方" })] });
    await waitFor(() => expect(byText("编辑")).toBeTruthy());
    await act(async () => {
      byText("编辑")!.click();
    });
    expect(view.container.querySelectorAll(".field-required")).toHaveLength(2);
  });

  it("**缺 API Key 点创建：就地报错、不提交**（用户实测的那条）", async () => {
    const { byText, view, runtime, inputsByPlaceholder, typeInto } = setup();
    await act(async () => {
      byText("新增供应商")!.click();
    });
    await typeInto(inputsByPlaceholder("官方")!, "没填 Key");
    await typeInto(inputsByPlaceholder("https://")!, "https://a/v1");
    await act(async () => {
      byText("创建")!.click();
    });
    expect(runtime.calls.some((item) => item.command === "vendor:save")).toBe(false);
    // 报错落在**字段旁边**（表单里就能看到），不是底部状态栏。
    expect(view.container.textContent).toContain("请输入 API Key。");
    expect(inputsByPlaceholder("sk-")!.getAttribute("aria-invalid")).toBe("true");
  });

  it("字段全空点创建：三条必填都就地报出来", async () => {
    const { byText, view, runtime } = setup();
    await act(async () => {
      byText("新增供应商")!.click();
    });
    await act(async () => {
      byText("创建")!.click();
    });
    expect(runtime.calls.some((item) => item.command === "vendor:save")).toBe(false);
    for (const message of ["请输入供应商名称。", "请输入 API 请求地址。", "请输入 API Key。"]) {
      expect(view.container.textContent, message).toContain(message);
    }
  });

  it("改动字段后它那条报错消失（旧结论过期）", async () => {
    const { byText, view, inputsByPlaceholder, typeInto } = setup();
    await act(async () => {
      byText("新增供应商")!.click();
    });
    await act(async () => {
      byText("创建")!.click();
    });
    expect(view.container.textContent).toContain("请输入 API Key。");
    await typeInto(inputsByPlaceholder("sk-")!, "sk-x");
    expect(view.container.textContent).not.toContain("请输入 API Key。");
  });
});

describe("VendorsPanel：余额与操作分列 + 复制（2026-10-02 实机反馈）", () => {
  const target = vendor({
    id: "v1",
    providerId: "claude",
    name: "官方",
    pricing: { inputPerMillionUsd: 1.5 },
    balanceQuery: { endpoint: "/balance" }
  });

  it("表头把「余额」「操作」分成两列，两格内容在不同单元格", async () => {
    const { view } = setup({ existing: [target] });
    await waitFor(() => expect(view.container.querySelector("tbody tr")).toBeTruthy());
    const headers = [...view.container.querySelectorAll("thead th")].map((item) =>
      (item.textContent ?? "").trim()
    );
    expect(headers).toContain("余额");
    expect(headers).toContain("操作");
    expect(headers.some((text) => text.includes("操作 / 余额"))).toBe(false);
    const balanceCell = view.container.querySelector(".vendor-balance-line")!.closest("td");
    const actionCell = view.container.querySelector(".vendor-row-actions")!.closest("td");
    expect(balanceCell).toBeTruthy();
    expect(actionCell).toBeTruthy();
    expect(balanceCell).not.toBe(actionCell);
  });

  it("「复制」把配置克隆进新建表单：地址/费率/查询保留、Key 留空待重填、不带 id", async () => {
    const { byText, runtime, inputsByPlaceholder, typeInto } = setup({ existing: [target] });
    await waitFor(() => expect(byText("复制")).toBeTruthy());
    await act(async () => {
      byText("复制")!.click();
    });

    // 新建（不是编辑）：按钮是「创建」，名称带「副本」。
    expect(byText("创建")).toBeTruthy();
    expect(byText("保存修改")).toBeFalsy();
    expect(inputsByPlaceholder("官方")!.value).toBe("官方 副本");
    expect(inputsByPlaceholder("https://")!.value).toBe("https://a/v1");
    expect(inputsByPlaceholder("0.55")!.value).toBe("1.5");
    // 密钥不回填：克隆出来必须重填（这也是「复制」与「编辑」的区别）。
    expect(inputsByPlaceholder("sk-")!.value).toBe("");

    await typeInto(inputsByPlaceholder("sk-")!, "sk-clone");
    await act(async () => {
      byText("创建")!.click();
    });
    await waitFor(() => {
      const call = runtime.calls.find((item) => item.command === "vendor:save");
      expect(call?.args.name).toBe("官方 副本");
      expect(call?.args.providerId).toBe("claude");
      // **不带 id**：这是新供应商，不是改原来那条。
      expect(call?.args).not.toHaveProperty("id");
    });
  });
});
