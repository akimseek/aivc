// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  listModels,
  openSessionFolder,
  queryGatewayLogs,
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
 * 日志分页参数必须**真的传下去**。
 *
 * 这条命令此前踩过一次：前端传 `{ page, pageSize }`，后端读的是另一套筛选参数名，
 * 于是那两个参数被**静默忽略**——函数看起来在分页，实际每次都拿全量，
 * 而且两边都编译得过。现在后端真的支持分页了，参数名再写错（少个 `s`、换成下划线）
 * 会退回同一个坑，因此在这里钉住。
 */
describe("queryGatewayLogs", () => {
  const invoke = vi.fn(async () => null);

  beforeEach(() => {
    invoke.mockClear();
    (window as unknown as { __TAURI__: unknown }).__TAURI__ = {
      core: { invoke },
      event: { listen: vi.fn() }
    };
  });

  it("分页调用带上 page/pageSize", async () => {
    await queryGatewayLogs({
      page: 2,
      pageSize: 20,
      outcome: "error",
      periodStart: "2026-09-01T00:00:00.000Z",
      periodEnd: "2026-09-30T00:00:00.000Z"
    });
    expect(invoke).toHaveBeenCalledWith("aivc_invoke", {
      command: "gateway:query-logs",
      args: {
        page: 2,
        pageSize: 20,
        outcome: "error",
        periodStart: "2026-09-01T00:00:00.000Z",
        periodEnd: "2026-09-30T00:00:00.000Z"
      }
    });
  });

  it("不带分页参数的旧调用：同一个命令，参数原样（返回裸数组那条路）", async () => {
    await queryGatewayLogs({ periodStart: "s", periodEnd: "e" });
    expect(invoke).toHaveBeenCalledWith("aivc_invoke", {
      command: "gateway:query-logs",
      args: { periodStart: "s", periodEnd: "e" }
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
