import { useCallback, useEffect, useState } from "react";

import { Button, Dialog } from "../components";
import * as api from "../ipc/commands";

/**
 * 退出确认框。
 *
 * # 为什么是后端发起的
 *
 * 用户点窗口关闭按钮时，Tauri 里只有**后端**拿得到「可以阻止关闭」的那个钩子，
 * 而这里的关闭**必须**能阻止：应用里可能跑着终端和本地网关，直接关掉会留下一堆
 * 没写盘的日志和没结束的进程。
 *
 * 因此方向是反的：后端拦住关闭 → 推 `app:request-exit` → 这里弹框 → 用户选完
 * 回调 `app:exit-choice`。
 *
 * # 不回答会怎样
 *
 * 后端**不会**因为等太久就自己退出——那会在用户还在读这段文字的时候杀掉应用。
 * 它会一直等；用户如果干脆再点一次窗口关闭按钮，后端就把那次当成「坚持要退」放行。
 * 见 `crates/aivc-app/src/exit.rs` 的失败安全一节。
 *
 * # 为什么现在套 `Dialog` 而不是自绘
 *
 * 这个框此前自带一套 `.overlay` 骨架（遮罩 + 头 + 正文 + 页脚），于是它是唯一
 * **没有 Escape** 的弹层：键盘用户除了鼠标点那三个按钮没有别的出口，而后端会一直
 * 等着回答。骨架换成共享的 `Dialog` 之后，Escape / `aria-modal` / 焦点进面板
 * 三件事都跟着来，尺寸档也用确认类的 `sm`（内容少，不该占满整屏）。
 */
export function ExitConfirm({ onError }: { onError: (message: string) => void }) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void api
      .listen<void>(api.CHANNEL_REQUEST_EXIT, () => setOpen(true))
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const choose = useCallback(
    async (choice: api.ExitChoice) => {
      setBusy(true);
      try {
        await api.chooseExitAction(choice);
        // 成功之后一律收起对话框。最小化时窗口会被隐藏，下次从托盘回来不该还挂着
        // 一个「要不要退出」的框；退出时应用本来就在关。
        // 失败时**保持打开**，用户可以重试——收起会让这次询问看起来从未发生过。
        setOpen(false);
      } catch (error) {
        onError(error instanceof Error ? error.message : String(error));
      } finally {
        setBusy(false);
      }
    },
    [onError]
  );

  /**
   * Escape（与点遮罩同一条路）等价于「取消」。
   *
   * 忙碌时忽略：请求已经在路上，再按一次只是重复询问后端；三个按钮此时也都是禁用的。
   */
  const cancel = useCallback(() => {
    if (!busy) void choose("cancel");
  }, [busy, choose]);

  if (!open) {
    return null;
  }

  return (
    <Dialog
      // 标题即无障碍名称（`Dialog` 默认取它）：可见文字与读屏念出的一致，符合
      // 「标签包含名称」，不再另写一个更短的 `aria-label`。
      title="退出 AI 可视化控制台"
      size="sm"
      onClose={cancel}
      // 点遮罩不关闭用 `Dialog` 的默认值：这个框的每个选项都会改变应用状态，
      // 误点一下遮罩不该被当成回答。
      footer={
        <>
          <Button disabled={busy} onClick={() => void choose("cancel")}>
            取消
          </Button>
          <Button disabled={busy} onClick={() => void choose("minimize")}>
            最小化到托盘
          </Button>
          <Button variant="danger" disabled={busy} onClick={() => void choose("quit")}>
            直接退出
          </Button>
        </>
      }
    >
      <p className="dialog-copy">
        最小化后应用会继续在后台运行，终端与本地 Gateway 都不受影响；
        直接退出会关闭所有终端和本地 Gateway。
      </p>
    </Dialog>
  );
}
