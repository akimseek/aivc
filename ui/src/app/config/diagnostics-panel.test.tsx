// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { DiagnosticsPanel, capabilityRows } from "./diagnostics-panel";
import type { AiProviderSummary, AiTarget } from "../../types";

/**
 * 诊断页。
 *
 * # 这一页此前只有一个目标表
 *
 * 旧平台 `provider-status-dialog.tsx` 有四张表（平台 / 路径 / 命令清单 / 能力矩阵）。
 * 这一轮补了**能力矩阵**与**配置目录的存在性探测**；命令清单与历史/回收站目录
 * 没做，原因是数据源在后端而界面不该抄一份（见面板头部注释）。
 *
 * 因此这里钉住的是：能力矩阵的**键名**必须与后端一致、路径探测**只对本机目标做**。
 */

function target(over: Partial<AiTarget> = {}): AiTarget {
  return {
    id: "codex:local",
    provider: "codex",
    label: "本机 Codex",
    kind: "local",
    codexHome: "C:\\Users\\x\\.codex",
    available: true,
    ...over
  } as AiTarget;
}

function provider(over: Partial<AiProviderSummary> = {}): AiProviderSummary {
  return {
    id: "codex",
    label: "Codex",
    capabilities: {
      skills: true,
      branch: true,
      usage: true,
      trash: true,
      batchActions: true,
      customCwd: true,
      export: true,
      sessionSettings: true,
      duplicate: true,
      vendorManagement: true
    },
    ...over
  } as AiProviderSummary;
}

function setup(options: { targets?: AiTarget[]; providers?: AiProviderSummary[]; exists?: boolean } = {}) {
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  const exists = options.exists ?? true;
  window.__TAURI__ = {
    core: {
      invoke: async (_command: string, args?: unknown) => {
        const payload = args as { command?: string; args?: Record<string, unknown> };
        calls.push({ command: payload?.command ?? "", args: payload?.args ?? {} });
        if (payload?.command === "shell:path-exists") return exists;
        return null;
      }
    },
    event: { listen: async () => () => undefined }
  };
  const targets = options.targets ?? [target()];
  const view = render(
    <DiagnosticsPanel
      targets={targets}
      selectedTargetId={targets[0]?.id ?? ""}
      providers={options.providers ?? [provider()]}
      selectedProviderId="codex"
    />
  );
  return { view, calls };
}

describe("capabilityRows：键名必须与后端一致", () => {
  it("十项俱全，顺序与旧平台一致", () => {
    const labels = capabilityRows(provider()).map((row) => row.label);
    expect(labels).toEqual([
      "Skill",
      "分支",
      "Usage",
      "回收站",
      "批量",
      "工作目录",
      "导出",
      "会话设置",
      "复制",
      "供应商管理"
    ]);
  });

  it("**键名写错就是「永远不支持」**，因此按 camelCase 逐个钉住", () => {
    // `batchActions` / `customCwd` / `sessionSettings` / `vendorManagement`
    // 写成 snake_case 不会报错，只会让那一行恒为「不支持」。
    const rows = capabilityRows(
      provider({
        capabilities: {
          skills: true,
          branch: true,
          usage: true,
          trash: true,
          batchActions: true,
          customCwd: true,
          export: true,
          sessionSettings: true,
          duplicate: true,
          vendorManagement: true
        }
      })
    );
    expect(rows.every((row) => row.enabled)).toBe(true);
  });

  it("Qoder 的真实能力差异要如实反映", () => {
    const rows = capabilityRows(
      provider({
        id: "qoder",
        label: "Qoder",
        capabilities: {
          skills: false,
          branch: false,
          usage: true,
          trash: true,
          batchActions: true,
          customCwd: true,
          export: true,
          sessionSettings: false,
          duplicate: false,
          vendorManagement: false
        }
      })
    );
    const byLabel = new Map(rows.map((row) => [row.label, row.enabled]));
    expect(byLabel.get("分支")).toBe(false);
    expect(byLabel.get("复制")).toBe(false);
    // 回收站对 Qoder 是**支持**的——按旧平台的说法写会写反。
    expect(byLabel.get("回收站")).toBe(true);
  });

  it("缺字段按不支持处理，而不是默认支持", () => {
    const rows = capabilityRows(provider({ capabilities: {} }));
    expect(rows.every((row) => !row.enabled)).toBe(true);
  });
});

describe("DiagnosticsPanel：路径探测", () => {
  it("本机目标会去探配置目录是否存在", async () => {
    const { calls } = setup({ exists: true });
    await waitFor(() => {
      expect(calls.some((call) => call.command === "shell:path-exists")).toBe(true);
    });
    const probe = calls.find((call) => call.command === "shell:path-exists")!;
    expect(probe.args.path).toBe("C:\\Users\\x\\.codex");
    // 带上 targetId：后端按目标分流，不传就按本机处理。
    expect(probe.args.targetId).toBe("codex:local");
  });

  it("探测结果渲染成「存在」", async () => {
    const { view } = setup({ exists: true });
    await waitFor(() => {
      expect(view.container.textContent).toContain("存在");
    });
  });

  it("目录不存在时如实显示「不存在」", async () => {
    const { view } = setup({ exists: false });
    await waitFor(() => {
      expect(view.container.textContent).toContain("不存在");
    });
  });

  it("**WSL 目标不做本机探测**——按本机路径检查会把「有」误判成「没有」", async () => {
    const wsl = target({
      id: "codex:wsl:Ubuntu",
      kind: "wsl",
      distro: "Ubuntu",
      label: "WSL · Ubuntu",
      codexHome: "/home/u/.codex"
    });
    const { view, calls } = setup({ targets: [wsl] });
    await act(async () => {
      await Promise.resolve();
    });
    expect(calls.some((call) => call.command === "shell:path-exists")).toBe(false);
    expect(view.container.textContent).toContain("WSL 目标，不做本机探测");
  });

  it("命令状态用旧平台的措辞「未找到命令」", async () => {
    const { view } = setup({ targets: [target({ available: false })] });
    await waitFor(() => {
      expect(view.container.textContent).toContain("未找到命令");
    });
  });
});

describe("DiagnosticsPanel：空状态", () => {
  it("没有目标时说清先做什么", () => {
    const view = render(
      <DiagnosticsPanel
        targets={[]}
        selectedTargetId=""
        providers={[provider()]}
        selectedProviderId="codex"
      />
    );
    expect(view.container.textContent).toContain("还没有探测到任何目标");
  });
});
