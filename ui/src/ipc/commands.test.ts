// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  listModels,
  openSessionFolder,
  windowClose,
  windowMinimize,
  windowStartDrag,
  windowToggleMaximize
} from "./commands";

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

describe("listModels", () => {
  const invoke = vi.fn(async () => null);

  beforeEach(() => {
    invoke.mockClear();
    (window as unknown as { __TAURI__: unknown }).__TAURI__ = {
      core: { invoke },
      event: { listen: vi.fn() }
    };
  });

  it("调用后端注册的 models:list，并带上 targetId", async () => {
    // 命令名写错与参数名写错一样：两边都编译得过，点下去才报「未知命令」/「缺少 targetId」。
    await listModels("qoder:local");
    expect(invoke).toHaveBeenCalledWith("aivc_invoke", {
      command: "models:list",
      args: { targetId: "qoder:local" }
    });
  });
});

/**
 * 窗口控制命令。
 *
 * 无边框之后这四个按钮是**唯一**的窗口操作入口（系统框已经没有了），命令名写错的
 * 后果就是窗口关不掉、也挪不动。后端那侧的名字登记在
 * `crates/aivc-ipc/src/commands.rs`，两边必须逐字一致。
 */
describe("窗口控制命令", () => {
  const invoke = vi.fn(async () => null);

  beforeEach(() => {
    invoke.mockClear();
    (window as unknown as { __TAURI__: unknown }).__TAURI__ = {
      core: { invoke },
      event: { listen: vi.fn() }
    };
  });

  it("windowMinimize 调 app:window-minimize", async () => {
    await windowMinimize();
    expect(invoke).toHaveBeenCalledWith("aivc_invoke", {
      command: "app:window-minimize",
      args: {}
    });
  });

  it("windowToggleMaximize 调 app:window-toggle-maximize", async () => {
    await windowToggleMaximize();
    expect(invoke).toHaveBeenCalledWith("aivc_invoke", {
      command: "app:window-toggle-maximize",
      args: {}
    });
  });

  it("windowStartDrag 调 app:window-start-drag", async () => {
    await windowStartDrag();
    expect(invoke).toHaveBeenCalledWith("aivc_invoke", {
      command: "app:window-start-drag",
      args: {}
    });
  });

  it("windowClose 调 app:window-close", async () => {
    await windowClose();
    expect(invoke).toHaveBeenCalledWith("aivc_invoke", {
      command: "app:window-close",
      args: {}
    });
  });
});
