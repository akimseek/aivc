/**
 * 与 Rust 后端通信的**唯一入口**。
 *
 * # 为什么只有一个命令
 *
 * Rust 侧只注册了一个 Tauri 命令 `aivc_invoke`，它收 `{ command, args }` 再转给内部的
 * 命令路由器。好处是前端不需要维护「几十个 invoke 名字」，只需要维护**命令名**这一层。
 *
 * 注意：`crates/aivc-app/src/tauri_app.rs` 里那个函数的文档注释写的是
 * `invoke("aivc:invoke", …)`，而**实际注册名是 `aivc_invoke`**（Tauri 用函数名）。
 * 按注释写会得到一个「命令不存在」的运行时错误，因此这里以函数名为准。
 *
 * # 为什么不让每个模块自己调 `invoke`
 *
 * 那样「命令名」会散落在整个前端里，改一个后端命令名要全局搜字符串。收敛到这一层之后，
 * 命令面只有一个地方要维护，也才谈得上类型化。
 */

/** Tauri 注入的全局对象；`withGlobalTauri: true` 时才存在。 */
declare global {
  interface Window {
    __TAURI__?: {
      core?: {
        invoke?: (command: string, args?: unknown) => Promise<unknown>;
      };
      event?: {
        listen?: (
          event: string,
          handler: (event: { payload: unknown }) => void
        ) => Promise<() => void>;
      };
    };
  }
}

/** Rust 侧唯一注册的命令名。 */
const INVOKE_COMMAND = "aivc_invoke";

/**
 * 订阅后端推送的事件。
 *
 * 返回一个取消订阅函数——**必须调用**：组件卸载后仍留着的监听会让已销毁的终端
 * 继续收数据，表现为「关掉的终端还在后台跑」。
 *
 * 与命令调用不同，事件是单向的（后端 → 前端），没有返回值。
 */
export async function listen<T>(
  channel: string,
  handler: (payload: T) => void
): Promise<() => void> {
  const event = window.__TAURI__?.event;
  if (typeof event?.listen !== "function") {
    // 收不到事件是「功能缺失」，抛异常是「启动失败」——这里选择前者：
    // 事件没接上不该让整个界面打不开。
    return () => undefined;
  }
  const unlisten = await event.listen(channel, (message) => {
    handler(message.payload as T);
  });
  return unlisten;
}

/** 后端返回的业务错误。 */
export class IpcError extends Error {
  constructor(
    message: string,
    /** 出错的命令名——没有它，报错只说「失败了」，查不出是哪一步。 */
    readonly command: string
  ) {
    super(message);
    this.name = "IpcError";
  }
}

/**
 * 调一条后端命令。
 *
 * 后端把错误以字符串形式返回（Tauri 命令的 `Err(String)`），因此这里统一包成
 * [`IpcError`] 并带上命令名：调试时「哪条命令失败」比「失败」本身有用得多。
 */
export async function invoke<T>(command: string, args?: unknown): Promise<T> {
  const core = window.__TAURI__?.core;
  if (typeof core?.invoke !== "function") {
    throw new IpcError(
      "Tauri 运行时不可用：window.__TAURI__.core.invoke 不存在。\
       若在浏览器里直接打开页面，这是预期的——界面必须由应用窗口加载。",
      command
    );
  }
  try {
    return (await core.invoke(INVOKE_COMMAND, {
      command,
      // 后端签名是 `args: Option<Value>`；显式传 `null` 比省略字段更明确。
      args: args ?? null
    })) as T;
  } catch (error) {
    throw new IpcError(
      error instanceof Error ? error.message : String(error),
      command
    );
  }
}
