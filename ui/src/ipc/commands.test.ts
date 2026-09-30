// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";

import { openSessionFolder } from "./commands";

/**
 * 命令名必须与后端注册表一致。
 *
 * 此前这里踩过一次：前端调 `codex:open-session-folder`，后端注册的是
 * `shell:open-session-folder`——**两边都编译得过**，按钮点下去报「未知命令」，
 * 只有人肉发现。因此对这条（以及同类）命令补一条断言。
 */
describe("openSessionFolder", () => {
  const invoke = vi.fn(async () => null);

  beforeEach(() => {
    invoke.mockClear();
    (window as unknown as { __TAURI__: unknown }).__TAURI__ = {
      core: { invoke },
      event: { listen: vi.fn() }
    };
  });

  it("调用后端注册的 shell:open-session-folder", async () => {
    await openSessionFolder("local", "sess-1");
    expect(invoke).toHaveBeenCalledWith("aivc_invoke", {
      command: "shell:open-session-folder",
      args: { targetId: "local", sessionId: "sess-1" }
    });
  });
});
