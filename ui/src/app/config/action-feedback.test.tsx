// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { SkillsPanel } from "./skills-panel";
import { VendorsPanel } from "./vendors-panel";
import { EnvironmentPanel } from "./environment-panel";
import type { CliEnvironmentStatus } from "../../types";

/**
 * **操作结果反馈**。
 *
 * # 为什么值得单独一组
 *
 * 旧平台**每个动作都有一句结果回应**（改名、复制、删除、恢复、启用 Skill、装完 CLI…）。
 * 新平台此前只有零星几处——不是漏了某一个，而是每一处都要自己拿一个 `useToast`，
 * 散在十几个组件里，补齐过程必然漏掉几个，而且**漏掉不会有任何提示**：
 * 界面照样能用，只是「静默完成与没执行看起来一样」。
 *
 * 这一组因此钉的是「做完那一下，`onNotify` 被调了，而且说的是那件事」。
 * 文案不逐字钉死（改措辞不该让测试红），但**必须提到那个对象**。
 */

const PROVIDERS = [
  { id: "codex" as const, label: "Codex", capabilities: {} },
  { id: "claude" as const, label: "Claude Code", capabilities: {} }
];

function fakeRuntime(handlers: Record<string, (args: Record<string, unknown>) => unknown>) {
  window.__TAURI__ = {
    core: {
      invoke: async (_command: string, args?: unknown) => {
        const payload = args as { command?: string; args?: Record<string, unknown> };
        const name = payload?.command ?? "";
        const handler = handlers[name];
        return handler ? handler(payload?.args ?? {}) : null;
      }
    },
    event: { listen: async () => () => undefined }
  };
}

describe("VendorsPanel：供应商动作要有回音", () => {
  const vendor = {
    id: "v1",
    providerId: "codex" as const,
    name: "测试家",
    apiBaseUrl: "https://a/v1",
    sort: 0,
    enabled: true
  };

  it("**启用 / 禁用**会说一声，并带上是哪一家", async () => {
    const onNotify = vi.fn();
    fakeRuntime({ "vendor:list": () => [vendor], "vendor:set-enabled": () => vendor });
    const view = render(
      <VendorsPanel
        providerId="codex"
        providers={PROVIDERS}
        onError={vi.fn()}
        onNotify={onNotify}
      />
    );
    await waitFor(() => {
      expect([...view.container.querySelectorAll("button")].some((b) => b.textContent === "禁用"))
        .toBe(true);
    });
    const button = [...view.container.querySelectorAll("button")].find(
      (b) => b.textContent === "禁用"
    )!;
    await act(async () => {
      button.click();
    });
    await waitFor(() => expect(onNotify).toHaveBeenCalled());
    expect(onNotify.mock.calls[0][0]).toContain("测试家");
  });

  it("**删除**会说一声", async () => {
    const onNotify = vi.fn();
    fakeRuntime({ "vendor:list": () => [vendor], "vendor:delete": () => true });
    const view = render(
      <VendorsPanel
        providerId="codex"
        providers={PROVIDERS}
        onError={vi.fn()}
        onNotify={onNotify}
      />
    );
    await waitFor(() => {
      expect([...view.container.querySelectorAll("button")].some((b) => b.textContent === "删除"))
        .toBe(true);
    });
    const button = [...view.container.querySelectorAll("button")].find(
      (b) => b.textContent === "删除"
    )!;
    await act(async () => {
      button.click();
    });
    await waitFor(() => expect(onNotify).toHaveBeenCalled());
    expect(onNotify.mock.calls[0][0]).toContain("测试家");
  });

  it("**启用走 vendor:enable（写 CLI 配置），停用只翻启用位**", async () => {
    // 旧平台「启用」的含义是「CLI 的配置指向它」——只翻标志位会留下
    // 「界面说已启用、CLI 还用着旧配置」这种最难排查的不一致。
    const onNotify = vi.fn();
    const calls: string[] = [];
    // 列表要**跟着状态变**：否则停用之后重新拉列表仍然回到「已启用」，
    // 界面上就永远看不到「启用」按钮（第一版测试就是这么失败的）。
    let enabled = true;
    fakeRuntime({
      "vendor:list": () => [{ ...vendor, enabled }],
      "vendor:set-enabled": () => {
        calls.push("set-enabled");
        enabled = false;
        return { ...vendor, enabled };
      },
      "vendor:enable": () => {
        calls.push("enable");
        enabled = true;
        return {
          vendorId: "v1",
          written: ["~/.codex/auth.json", "~/.codex/config.toml"],
          backupRoot: "data/vendor-backups/测试家/2026"
        };
      }
    });
    const view = render(
      <VendorsPanel
        providerId="codex"
        providers={PROVIDERS}
        onError={vi.fn()}
        onNotify={onNotify}
      />
    );
    const button = (text: string) =>
      [...view.container.querySelectorAll("button")].find((b) => b.textContent === text);

    await waitFor(() => expect(button("禁用")).toBeTruthy());
    // 停用：只翻启用位，**不动用户的文件**。
    await act(async () => {
      button("禁用")!.click();
    });
    await waitFor(() => expect(button("启用")).toBeTruthy());
    expect(calls).toEqual(["set-enabled"]);
    expect(onNotify.mock.calls[0][0]).toContain("已禁用");

    // 启用：走 vendor:enable（写文件 + 备份）。
    await act(async () => {
      button("启用")!.click();
    });
    await waitFor(() => expect(calls).toContain("enable"));
    // 启用那条必须点名写了哪些文件、备份在哪——它动了用户 HOME 下的东西。
    await waitFor(() => {
      expect(onNotify.mock.calls.some((call) => String(call[0]).includes("备份"))).toBe(true);
    });
  });

  it("**新建默认不启用**——启用会写用户 CLI 配置，不该由「新增」顺手完成", async () => {
    const onNotify = vi.fn();
    const calls: Array<Record<string, unknown>> = [];
    fakeRuntime({
      "vendor:list": () => [],
      "vendor:save": (args) => {
        calls.push(args);
        return vendor;
      }
    });
    const view = render(
      <VendorsPanel
        providerId="codex"
        providers={PROVIDERS}
        onError={vi.fn()}
        onNotify={onNotify}
      />
    );
    await act(async () => {
      [...view.container.querySelectorAll("button")]
        .find((b) => b.textContent === "新增供应商")!
        .click();
    });
    const setValue = async (element: HTMLInputElement, value: string) => {
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(
          element,
          value
        );
        element.dispatchEvent(new Event("input", { bubbles: true }));
      });
    };
    const inputs = [...view.container.querySelectorAll<HTMLInputElement>("input")];
    await setValue(inputs.find((i) => i.placeholder.includes("官方"))!, "新家");
    await setValue(inputs.find((i) => i.placeholder.includes("https://"))!, "https://n/v1");
    await act(async () => {
      [...view.container.querySelectorAll("button")].find((b) => b.textContent === "创建")!.click();
    });
    await waitFor(() => expect(calls.length).toBe(1));
    expect(calls[0].enabled).toBe(false);
  });

  it("**新增**会说一声", async () => {
    const onNotify = vi.fn();
    fakeRuntime({ "vendor:list": () => [], "vendor:save": () => vendor });
    const view = render(
      <VendorsPanel
        providerId="codex"
        providers={PROVIDERS}
        onError={vi.fn()}
        onNotify={onNotify}
      />
    );
    const newButton = [...view.container.querySelectorAll("button")].find(
      (b) => b.textContent === "新增供应商"
    )!;
    await act(async () => {
      newButton.click();
    });
    const setValue = async (element: HTMLInputElement, value: string) => {
      await act(async () => {
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
        setter.call(element, value);
        element.dispatchEvent(new Event("input", { bubbles: true }));
      });
    };
    const inputs = [...view.container.querySelectorAll<HTMLInputElement>("input")];
    await setValue(inputs.find((i) => i.placeholder.includes("官方"))!, "新家");
    await setValue(inputs.find((i) => i.placeholder.includes("https://"))!, "https://n/v1");
    const create = [...view.container.querySelectorAll("button")].find(
      (b) => b.textContent === "创建"
    )!;
    await act(async () => {
      create.click();
    });
    await waitFor(() => expect(onNotify).toHaveBeenCalled());
    expect(onNotify.mock.calls[0][0]).toContain("新家");
  });
});

describe("SkillsPanel：Skill 动作要有回音", () => {
  const skill = { name: "demo", description: "", path: "/s/demo", enabled: true };

  it("**禁用**会说一声，并带上 Skill 名", async () => {
    // 按钮、标签、提示三处必须是同一个词（旧平台用的是「禁用」）。
    // 这一条最初写成「停用」而失败——那正是它要抓的不一致。
    const onNotify = vi.fn();
    fakeRuntime({ "skill:list": () => [skill], "skill:set-enabled": () => ({}) });
    const view = render(
      <SkillsPanel targetId="codex:local" onError={vi.fn()} onNotify={onNotify} />
    );
    await waitFor(() => {
      expect([...view.container.querySelectorAll("button")].some((b) => b.textContent === "禁用"))
        .toBe(true);
    });
    const button = [...view.container.querySelectorAll("button")].find(
      (b) => b.textContent === "禁用"
    )!;
    await act(async () => {
      button.click();
    });
    await waitFor(() => expect(onNotify).toHaveBeenCalled());
    expect(onNotify.mock.calls[0][0]).toContain("demo");
    expect(onNotify.mock.calls[0][0]).toContain("禁用");
  });
});

describe("EnvironmentPanel：装完 CLI 要说一声", () => {
  function status(over: Partial<CliEnvironmentStatus> = {}): CliEnvironmentStatus {
    return {
      providerId: "codex",
      targetId: "codex:local",
      scope: "local",
      platform: "windows",
      minimumNodeMajor: 22,
      recommendedNodeMajor: 24,
      nodeVersion: "v22.11.0",
      npmVersion: "10.9.0",
      nvmVersion: "1.2.2",
      cliVersion: undefined,
      hasNode: true,
      hasNpm: true,
      hasNvm: true,
      hasCli: false,
      nodeManagedByNvm: false,
      nodeMeetsMinimum: true,
      canInstallCli: true,
      canInstallNodeWithNvm: true,
      messages: ["未检测到 codex。"],
      ...over
    };
  }

  it("安装成功后 `onNotify` 被调用——**这是分钟级的操作，不说一声用户不知道装完没有**", async () => {
    const onNotify = vi.fn();
    let installed = false;
    fakeRuntime({
      "cli:check-environment": () => status(),
      "cli:install": () => {
        installed = true;
        return {};
      },
      "app:get-version": () => "0.1.0"
    });
    const view = render(
      <EnvironmentPanel
        providerId="codex"
        targetId="codex:local"
        onError={vi.fn()}
        onNotify={onNotify}
      />
    );
    await waitFor(() => {
      expect(
        [...view.container.querySelectorAll("button")].some((b) =>
          (b.textContent ?? "").startsWith("安装")
        )
      ).toBe(true);
    });
    const button = [...view.container.querySelectorAll("button")].find((b) =>
      (b.textContent ?? "").startsWith("安装")
    )!;
    await act(async () => {
      button.click();
    });
    await waitFor(() => expect(onNotify).toHaveBeenCalled());
    expect(installed).toBe(true);
    expect(onNotify.mock.calls[0][0]).toContain("CLI");
  });
});
