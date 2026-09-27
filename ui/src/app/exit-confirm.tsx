import { useCallback, useEffect, useState } from "react";

import { Button } from "../components";
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

  if (!open) {
    return null;
  }

  return (
    <div className="overlay" role="dialog" aria-modal="true" aria-label="退出确认">
      <div className="overlay-panel overlay-panel-confirm">
        <header className="overlay-head">
          <h2>退出 AI 可视化控制台</h2>
        </header>

        <div className="overlay-body">
          <p className="overlay-copy">
            最小化后应用会继续在后台运行，终端与本地 Gateway 都不受影响；
            直接退出会关闭所有终端和本地 Gateway。
          </p>
        </div>

        <footer className="overlay-foot">
          <Button small disabled={busy} onClick={() => void choose("cancel")}>
            取消
          </Button>
          <Button small disabled={busy} onClick={() => void choose("minimize")}>
            最小化到托盘
          </Button>
          <Button small variant="danger" disabled={busy} onClick={() => void choose("quit")}>
            直接退出
          </Button>
        </footer>
      </div>
    </div>
  );
}
