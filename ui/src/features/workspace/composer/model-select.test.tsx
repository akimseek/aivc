// @vitest-environment jsdom
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
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
});
