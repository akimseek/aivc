import { useState } from "react";

import { Button } from "../components";
import * as api from "../ipc/commands";
import type { AiSession } from "../types";

/**
 * 会话头的动作条。
 *
 * # 为什么这些动作放在内容上方而不是右键菜单里
 *
 * 它们是**常用**动作：重命名、复制、导出。旧平台把它们放在详情面板顶部，
 * 藏进右键菜单会让它们难以发现——右键菜单的定位是「偶尔用一次的那几个」。
 *
 * # 「另存到…」与「一键导出」并存
 *
 * 一键导出用后端默认落点，立即可用；另存到…先用**原生目录选择框**问用户放哪。
 * 两条路径都要，因为「快速拿一份」和「放到指定位置」是两种真实需求。
 *
 * # 重命名是**内联输入框**
 *
 * 不用弹层：改个名字弹一层会打断视线，而用户往往正看着这条会话的内容想标题。
 */
export function SessionActions({
  session,
  onRename,
  onDuplicate,
  onExport,
  onOpenDetail
}: {
  session: AiSession | null;
  onRename: (title: string) => void;
  onDuplicate: () => void;
  onExport: (format: "markdown" | "json" | "html", toDirectory?: boolean) => void;
  /** 打开详情弹层（旧平台在会话头右侧有「详情」按钮）。 */
  onOpenDetail: () => void;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  const disabled = !session;

  if (draft !== null) {
    return (
      <input
        className="rename-input"
        autoFocus
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            onRename(draft);
            setDraft(null);
          }
          if (event.key === "Escape") setDraft(null);
        }}
        onBlur={() => setDraft(null)}
      />
    );
  }

  return (
    <>
      <Button small disabled={disabled} onClick={onOpenDetail} title="打开会话详情">
        详情
      </Button>
      <Button
        small
        disabled={disabled}
        onClick={() => setDraft(session?.title ?? "")}
        title="重命名这条会话"
      >
        重命名
      </Button>
      <Button small disabled={disabled} onClick={onDuplicate} title="复制出一条新会话">
        复制
      </Button>
      <Button small disabled={disabled} onClick={() => onExport("markdown")}>
        导出 MD
      </Button>
      <Button small disabled={disabled} onClick={() => onExport("json")}>
        导出 JSON
      </Button>
      <Button
        small
        disabled={disabled}
        onClick={() => onExport("markdown", true)}
        title="选择导出目录"
      >
        另存到…
      </Button>
    </>
  );
}

/**
 * 导出到指定目录，返回实际落点（用户取消时返回 `null`）。
 *
 * 文件名用会话编号前 8 位：标题可能是中文、可能带斜杠，直接当文件名在 Windows 上
 * 会失败；编号是安全且稳定唯一的。这件事**必须在这里做**，而不是留给用户去改文件名。
 */
export async function exportToDirectory(
  format: "markdown" | "json" | "html",
  sessionId: string,
  exportTo: (format: "markdown" | "json" | "html", path: string) => Promise<unknown>
): Promise<string | null> {
  const directory = await api.chooseDirectory("选择导出到的目录");
  // `null` 表示用户取消——取消是正常操作，什么都不做。
  if (!directory) return null;
  const separator = directory.includes("\\") ? "\\" : "/";
  const extension = format === "markdown" ? "md" : format;
  const path = `${directory}${directory.endsWith(separator) ? "" : separator}session-${sessionId.slice(0, 8)}.${extension}`;
  await exportTo(format, path);
  return path;
}
