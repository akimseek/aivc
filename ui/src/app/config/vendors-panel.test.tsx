// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  VendorsPanel,
  balanceLabel,
  parseAdvancedJson,
  parsePrice,
  priceText
} from "./vendors-panel";
import type { AiProviderId } from "../../types";

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
        if (name === "vendor:save") return { id: "new" };
        if (name === "vendor:set-enabled") return {};
        if (name === "vendor:delete") return true;
        if (name === "vendor:refresh-balance") {
          const vendorId = String(payload?.args?.vendorId ?? "");
          if (extra.balance === "fail") return { vendorId, error: "连不上上游" };
          return { vendorId, balance: extra.balance ?? {} };
        }
        if (name === "vendor:refresh-balances") return extra.allBalances ?? [];
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
    const found = [...view.container.querySelectorAll("button")].find(
      (item) => item.textContent?.trim() === label
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
  vi.restoreAllMocks();
});

describe("VendorsPanel：面板头", () => {
  it("头部就是那四个动作，且都在面板头里", async () => {
    const { view, byText } = setup({ existing: [vendor()] });
    await waitFor(() => expect(byText("查余额")).toBeTruthy());
    for (const label of ["查余额", "测连接（列模型）", "读本地配置", "新增供应商"]) {
      expect(byText(label), label).toBeTruthy();
    }
    const header = view.container.querySelector(".panel-header")!;
    expect(header.textContent).toContain("供应商管理");
    for (const label of ["查余额", "测连接（列模型）", "读本地配置", "新增供应商"]) {
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

  it("没有 API 地址时创建按钮禁用（地址是必填）", async () => {
    const { byText, inputsByPlaceholder, typeInto } = setup();
    await act(async () => {
      byText("新增供应商")!.click();
    });
    await typeInto(inputsByPlaceholder("官方")!, "只有名字");
    expect(byText("创建")!.disabled).toBe(true);
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
  it("费率一格：`$输入 / $输出`，未设置的那一边是 `-`（与 0 不是一回事）", () => {
    expect(priceText(undefined)).toBe("-");
    expect(priceText({ inputPerMillionUsd: 0 })).toBe("$0 / -");
    expect(priceText({ inputPerMillionUsd: 0.15, outputPerMillionUsd: 0.6 })).toBe("$0.15 / $0.6");
  });

  it("费率列显示真实值；未设置时是 `-` 而不是 `$0`", async () => {
    const { view } = setup({
      existing: [
        vendor({ id: "v1", name: "带费率", pricing: { inputPerMillionUsd: 0.15, outputPerMillionUsd: 0.6 } }),
        vendor({ id: "v2", name: "没费率" })
      ]
    });
    await waitFor(() => expect(view.container.querySelector("table.data-table")).toBeTruthy());
    expect(view.container.textContent).toContain("$0.15 / $0.6");
    // 两行都写「没费率」的那一格是 `-`。
    expect(view.container.querySelectorAll("tbody tr")).toHaveLength(2);
  });

  it("「操作 / 余额」列头如实说明这一格装着两样东西（余额不在列清单里）", async () => {
    // 余额按行内状态机呈现 + 每行带单家刷新按钮，于是这一格里既有余额又有三个操作。
    // 列头只写「操作」的话，用户找不到「余额」在哪一列。
    const { view } = setup({ existing: [vendor()] });
    await waitFor(() => expect(view.container.querySelector("table.data-table")).toBeTruthy());
    const heads = [...view.container.querySelectorAll("thead th")].map((item) => item.textContent);
    expect(heads).toContain("操作 / 余额");
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
    // 没有快照的那一行，健康那一格是空的。
    const cells = [...view.container.querySelectorAll("tbody tr")][1]!.querySelectorAll("td");
    expect(cells[3]!.textContent).toBe("");
  });

  it("候选池状态：同厂商里顺位最前的那家是「已启用 · 优先」", async () => {
    const { view } = setup({
      existing: [
        vendor({ id: "v1", name: "第一顺位", sort: 1 }),
        vendor({ id: "v2", name: "第二顺位", sort: 2 }),
        vendor({ id: "v3", name: "被禁用", sort: 0, enabled: false })
      ]
    });
    await waitFor(() => expect(view.container.querySelector("table.data-table")).toBeTruthy());
    const text = view.container.textContent ?? "";
    expect(text).toContain("已启用 · 优先");
    expect(text).toContain("已启用");
    expect(text).toContain("未启用");
    // 「优先」属于**启用且顺位最前**的那家：被禁用的那家排序更靠前也不算。
    const rows = [...view.container.querySelectorAll("tbody tr")];
    expect(rows[0]!.textContent).toContain("已启用 · 优先");
    expect(rows[1]!.textContent).not.toContain("优先");
    expect(rows[2]!.textContent).toContain("未启用");
  });
});

describe("VendorsPanel：余额行内状态", () => {
  it("没查过时说「未获取」，不是 0 也不是空白", async () => {
    const { view } = setup({ existing: [vendor()] });
    await waitFor(() => expect(view.container.querySelector("table.data-table")).toBeTruthy());
    expect(view.container.textContent).toContain("余额 未获取");
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
    const { view, byText } = setup(
      { existing: [vendor()] },
      { balance: { remaining: 12.5, unit: "USD" } }
    );
    await waitFor(() => expect(byText("刷新")).toBeTruthy());
    await act(async () => {
      byText("刷新")!.click();
    });
    await waitFor(() => {
      expect(view.container.textContent).toContain("余额 12.5 USD");
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
    const { view, byText } = setup(
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
    });
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
  it("两个动作各自的后果写在按钮提示里（一个写文件、一个只翻位）", async () => {
    const { byText } = setup({
      existing: [
        vendor({ id: "v1", name: "开着的" }),
        vendor({ id: "v2", name: "关着的", enabled: false })
      ]
    });
    await waitFor(() => expect(byText("禁用")).toBeTruthy());
    // 禁用只翻数据库标志位；启用会**写用户的 CLI 配置文件**（写前备份）。
    expect(byText("禁用")!.getAttribute("title")).toContain("只翻启用位");
    expect(byText("启用")!.getAttribute("title")).toContain("写进 CLI 配置文件");
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

describe("VendorsPanel：工具区块在表下方，按需展开", () => {  it("「测连接（列模型）」展开测连接卡片，再点一次收起", async () => {
    const { view, byText } = setup({ existing: [vendor()] });
    await waitFor(() => expect(byText("测连接（列模型）")).toBeTruthy());
    expect(view.container.textContent).not.toContain("查询模型");

    await act(async () => {
      byText("测连接（列模型）")!.click();
    });
    await waitFor(() => expect(byText("查询模型")).toBeTruthy());

    await act(async () => {
      byText("测连接（列模型）")!.click();
    });
    expect(byText("查询模型")).toBeFalsy();
  });

  it("「读本地配置」展开白名单里的三个入口", async () => {
    const { view, byText } = setup({ existing: [vendor()] });
    await waitFor(() => expect(byText("读本地配置")).toBeTruthy());
    await act(async () => {
      byText("读本地配置")!.click();
    });
    await waitFor(() => {
      expect(view.container.textContent).toContain("CLI 实际读到的配置");
      expect(byText("Codex 配置")).toBeTruthy();
      expect(byText("Claude 配置")).toBeTruthy();
      expect(byText("Gemini 配置")).toBeTruthy();
    });
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
