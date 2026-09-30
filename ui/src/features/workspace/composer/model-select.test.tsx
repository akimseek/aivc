// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ModelSelect } from "./model-select";

/**
 * 模型选择。
 *
 * # 这里钉住的是「什么时候发请求」
 *
 * 取一次模型清单可能是一次真实的 HTTP 调用（供应商那一路）或一次 CLI 进程（Qoder
 * 那一路），**不能因为界面重渲染就发**。因此第一组断言是「渲染本身不发请求」，
 * 只有用户去碰那个下拉才算一次展开。
 *
 * 取数走的是 `vendor:list-models` / `models:list`，选中后写 `/model <id>` ——
 * 后者是旧平台的语义：CLI 收到就立刻切，不需要重启终端。
 */

/** 打桩的命令面。测试只断言「哪条命令被调了、带了什么」。 */
const ipc = vi.hoisted(() => ({
  listVendorModels: vi.fn(async () => [] as Array<{ id: string; name?: string }>),
  listModels: vi.fn(async () => [] as Array<{ id: string }>),
  writeTerminal: vi.fn(async () => undefined)
}));

vi.mock("../../../ipc/commands", () => ({
  listVendorModels: ipc.listVendorModels,
  listModels: ipc.listModels,
  writeTerminal: ipc.writeTerminal
}));

const select = (container: HTMLElement) =>
  container.querySelector<HTMLSelectElement>(".model-select");

beforeEach(() => {
  vi.clearAllMocks();
});

afterEach(cleanup);

describe("ModelSelect", () => {
  it("首次展开才取数，且只取一次", async () => {
    ipc.listVendorModels.mockResolvedValue([{ id: "gpt-5", name: "GPT-5" }]);
    const view = render(
      <ModelSelect terminalId="t1" providerId="codex" vendorId="v1" onError={vi.fn()} />
    );

    // 渲染（含重渲染）本身不该发请求——每开一个终端都发一次是不可接受的。
    expect(select(view.container)).not.toBeNull();
    expect(ipc.listVendorModels).not.toHaveBeenCalled();

    fireEvent.pointerDown(select(view.container)!);
    await waitFor(() => expect(ipc.listVendorModels).toHaveBeenCalledWith("v1"));
    await waitFor(() => expect(select(view.container)!.textContent).toContain("GPT-5"));

    // 再碰一次不该重复取：清单在一次会话里不会变，重复取只是白等。
    fireEvent.pointerDown(select(view.container)!);
    expect(ipc.listVendorModels).toHaveBeenCalledTimes(1);
  });

  it("选中后向终端写 `/model <id>`", async () => {
    ipc.listVendorModels.mockResolvedValue([{ id: "gpt-5" }, { id: "gpt-5-mini" }]);
    const onSent = vi.fn();
    const view = render(
      <ModelSelect
        terminalId="t1"
        providerId="codex"
        vendorId="v1"
        onSent={onSent}
        onError={vi.fn()}
      />
    );

    fireEvent.pointerDown(select(view.container)!);
    // 1 个占位项 + 2 个模型。
    await waitFor(() => expect(select(view.container)!.options).toHaveLength(3));

    fireEvent.change(select(view.container)!, { target: { value: "gpt-5-mini" } });
    await waitFor(() =>
      expect(ipc.writeTerminal).toHaveBeenCalledWith("t1", "/model gpt-5-mini\r")
    );
    expect(onSent).toHaveBeenCalledWith("/model gpt-5-mini");
  });

  it("取数失败显示「无法获取模型」，重新展开即重试", async () => {
    ipc.listVendorModels.mockRejectedValueOnce(new Error("网络不通"));
    ipc.listVendorModels.mockResolvedValueOnce([{ id: "gpt-5" }]);
    const view = render(
      <ModelSelect terminalId="t1" providerId="codex" vendorId="v1" onError={vi.fn()} />
    );

    fireEvent.pointerDown(select(view.container)!);
    await waitFor(() => expect(view.container.textContent).toContain("无法获取模型"));

    // 重新展开 = 再试一次。失败一次就永久坏掉的下拉比没有更糟。
    fireEvent.pointerDown(select(view.container)!);
    await waitFor(() => expect(ipc.listVendorModels).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(select(view.container)!.textContent).toContain("gpt-5"));
  });

  it("Qoder 走 `models:list`（带 targetId）", async () => {
    ipc.listModels.mockResolvedValue([{ id: "qwen3-coder" }]);
    const view = render(
      <ModelSelect terminalId="t1" providerId="qoder" targetId="qoder:local" onError={vi.fn()} />
    );

    fireEvent.pointerDown(select(view.container)!);
    await waitFor(() => expect(ipc.listModels).toHaveBeenCalledWith("qoder:local"));
    expect(ipc.listVendorModels).not.toHaveBeenCalled();
  });

  it("没有模型来源时什么都不渲染", () => {
    // 非 Qoder 又没有供应商：这一路终端没有可列的模型，宁可没有这一项。
    const view = render(
      <ModelSelect terminalId="t1" providerId="codex" onError={vi.fn()} />
    );
    expect(select(view.container)).toBeNull();
  });

  it("写入失败走 onError，且不抛出去", async () => {
    ipc.listVendorModels.mockResolvedValue([{ id: "gpt-5" }]);
    ipc.writeTerminal.mockRejectedValueOnce(new Error("终端已关闭"));
    const onError = vi.fn();
    const view = render(
      <ModelSelect terminalId="t1" providerId="codex" vendorId="v1" onError={onError} />
    );

    fireEvent.pointerDown(select(view.container)!);
    await waitFor(() => expect(select(view.container)!.options).toHaveLength(2));
    fireEvent.change(select(view.container)!, { target: { value: "gpt-5" } });

    await waitFor(() => expect(onError).toHaveBeenCalledWith("终端已关闭"));
  });

  it("写入失败后选中项复位：同一个模型再选一次仍然发得出去", async () => {
    // 不复位的话下拉停在刚选的那一项上，同值再选不触发 `onChange`——
    // 那一次失败之后用户**再也切不过去**（除非先选一个别的再选回来）。
    ipc.listVendorModels.mockResolvedValue([{ id: "gpt-5" }]);
    ipc.writeTerminal.mockRejectedValueOnce(new Error("终端已关闭"));
    const view = render(
      <ModelSelect terminalId="t1" providerId="codex" vendorId="v1" onError={vi.fn()} />
    );

    fireEvent.pointerDown(select(view.container)!);
    await waitFor(() => expect(select(view.container)!.options).toHaveLength(2));
    fireEvent.change(select(view.container)!, { target: { value: "gpt-5" } });
    await waitFor(() => expect(ipc.writeTerminal).toHaveBeenCalledTimes(1));
    // 回到占位项（值不再是 gpt-5）。
    await waitFor(() => expect(select(view.container)!.value).toBe(""));

    fireEvent.change(select(view.container)!, { target: { value: "gpt-5" } });
    await waitFor(() => expect(ipc.writeTerminal).toHaveBeenCalledTimes(2));
  });

  it("右键 / 中键不触发取数（Qoder 那一路会真起一次 CLI）", async () => {
    const view = render(
      <ModelSelect terminalId="t1" providerId="qoder" targetId="qoder:local" onError={vi.fn()} />
    );

    fireEvent.pointerDown(select(view.container)!, { button: 2 });
    fireEvent.pointerDown(select(view.container)!, { button: 1 });
    expect(ipc.listModels).not.toHaveBeenCalled();

    // 主键才算「要去选」。
    fireEvent.pointerDown(select(view.container)!, { button: 0 });
    await waitFor(() => expect(ipc.listModels).toHaveBeenCalledWith("qoder:local"));
  });

  it("来源变了（供应商被自动换掉）就把清单作废，不再列旧家的模型", async () => {
    // 自动故障转移全程没有用户操作：沿用旧清单会让用户把 `/model` 发给一家
    // 已经不再服务这一路的供应商。
    ipc.listVendorModels.mockResolvedValue([{ id: "gpt-5" }]);
    const view = render(
      <ModelSelect terminalId="t1" providerId="codex" vendorId="v1" onError={vi.fn()} />
    );
    fireEvent.pointerDown(select(view.container)!);
    await waitFor(() => expect(select(view.container)!.options).toHaveLength(2));
    fireEvent.change(select(view.container)!, { target: { value: "gpt-5" } });
    await waitFor(() => expect(select(view.container)!.value).toBe("gpt-5"));

    ipc.listVendorModels.mockResolvedValue([{ id: "claude-sonnet" }]);
    view.rerender(
      <ModelSelect terminalId="t1" providerId="codex" vendorId="v2" onError={vi.fn()} />
    );

    // 清单没了、选中项也清了：只剩余占位项那一条。
    await waitFor(() => expect(select(view.container)!.options).toHaveLength(1));
    expect(select(view.container)!.value).toBe("");
    expect(select(view.container)!.textContent).toContain("选择模型");

    // 重新展开取的是**新那一家**的清单。
    fireEvent.pointerDown(select(view.container)!);
    await waitFor(() => expect(ipc.listVendorModels).toHaveBeenCalledWith("v2"));
    await waitFor(() => expect(select(view.container)!.textContent).toContain("claude-sonnet"));
  });

  it("在飞的旧来源请求返回时被丢弃（不把旧家的模型填回来）", async () => {
    // 为什么这条要紧：Qoder 那一路取一次模型要起一次 CLI（最长 20 秒），而供应商
    // 被自动故障转移换掉是**瞬间**发生的。旧家的响应晚一步回来就把复位清掉的清单
    // 又填上了——用户会在新来源下看到并选中旧家的模型。
    let settle: (models: Array<{ id: string; name?: string }>) => void = () => {};
    ipc.listVendorModels.mockImplementationOnce(
      () => new Promise((resolve) => (settle = resolve))
    );
    const view = render(
      <ModelSelect terminalId="t1" providerId="codex" vendorId="v1" onError={vi.fn()} />
    );

    // 向 v1 发起加载，请求挂住不返回。
    fireEvent.pointerDown(select(view.container)!);
    await waitFor(() => expect(ipc.listVendorModels).toHaveBeenCalledWith("v1"));
    expect(select(view.container)!.textContent).toContain("加载中…");

    // 自动故障转移：来源换成 v2，状态复位（清清单、清选中）。
    view.rerender(
      <ModelSelect terminalId="t1" providerId="codex" vendorId="v2" onError={vi.fn()} />
    );
    await waitFor(() => expect(select(view.container)!.textContent).toContain("选择模型"));

    // 旧请求这时才回来：必须整条丢弃——仍停在复位后的「没取过」，而不是列出 v1 的模型。
    await act(async () => {
      settle([{ id: "gpt-5", name: "旧家的模型" }]);
    });
    expect(select(view.container)!.options).toHaveLength(1);
    expect(select(view.container)!.textContent).not.toContain("旧家的模型");
    expect(select(view.container)!.textContent).toContain("选择模型");
  });

  it("在飞的旧来源请求失败时也不写状态（新来源仍是「没取过」）", async () => {
    // 与上一条同一个道理的另一半：旧家的失败若写进来，新来源会凭空显示
    // 「无法获取模型」——那会把「重新展开即重试」的入口也一并藏掉（状态不是 failed）。
    let fail: (problem: Error) => void = () => {};
    ipc.listVendorModels.mockImplementationOnce(
      () => new Promise((_resolve, reject) => (fail = reject))
    );
    const view = render(
      <ModelSelect terminalId="t1" providerId="codex" vendorId="v1" onError={vi.fn()} />
    );

    fireEvent.pointerDown(select(view.container)!);
    await waitFor(() => expect(ipc.listVendorModels).toHaveBeenCalledWith("v1"));

    view.rerender(
      <ModelSelect terminalId="t1" providerId="codex" vendorId="v2" onError={vi.fn()} />
    );
    await waitFor(() => expect(select(view.container)!.textContent).toContain("选择模型"));

    await act(async () => {
      fail(new Error("旧家的请求超时"));
    });
    expect(select(view.container)!.textContent).toContain("选择模型");
    expect(select(view.container)!.textContent).not.toContain("无法获取模型");
  });
});
