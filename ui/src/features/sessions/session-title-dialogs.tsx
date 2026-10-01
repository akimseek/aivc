import { useState } from "react";

import { Button, Dialog, Field, TextInput } from "../../components";
import type { AiSession } from "../../types";

/**
 * 重命名 / 复制会话的两个对话框。
 *
 * # 为什么复制**要问名字**
 *
 * 旧平台的 `session-duplicate-dialog.tsx` 有一个「会话名称（可选）」字段，
 * 提示是「留空则使用原会话标题」。不问名字直接复制出一个同名会话，列表里会立刻
 * 出现两条一模一样的标题——而用户复制它的**目的**通常正是为了改出个不同的名字。
 *
 * # 为什么重命名是弹层而不是内联输入框
 *
 * 会话头那处内联输入框保留（改当前会话很快）；右键菜单走这条弹层路径——
 * 那一处的上下文是**列表里的某一条**，不是「我正在看的那条」，
 * 内联到列表行里会挤坏排版。
 */
export function SessionRenameDialog({
  session,
  onConfirm,
  onClose
}: {
  session: AiSession;
  onConfirm: (title: string) => void;
  onClose: () => void;
}) {
  const [draft, setDraft] = useState(session.title ?? "");
  const trimmed = draft.trim();

  return (
    <Dialog
      title="重命名会话"
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>取消</Button>
          <Button variant="primary" disabled={!trimmed} onClick={() => onConfirm(trimmed)}>
            保存
          </Button>
        </>
      }
    >
      <Field label="会话名称" hint="这会写进会话的自定义标题，不改动文件内容。">
        <TextInput
          value={draft}
          autoFocus
          placeholder="请输入会话名称"
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && trimmed) onConfirm(trimmed);
          }}
        />
      </Field>
    </Dialog>
  );
}

export function SessionDuplicateDialog({
  onConfirm,
  onClose
}: {
  onConfirm: (title: string | undefined) => void;
  onClose: () => void;
}) {
  const [draft, setDraft] = useState("");
  const trimmed = draft.trim();

  return (
    <Dialog
      title="复制会话"
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>取消</Button>
          <Button variant="primary" onClick={() => onConfirm(trimmed || undefined)}>
            复制
          </Button>
        </>
      }
    >
      <Field label="会话名称（可选）" hint="留空则使用原会话标题。">
        <TextInput
          value={draft}
          autoFocus
          placeholder="留空则使用原会话标题"
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") onConfirm(trimmed || undefined);
          }}
        />
      </Field>
    </Dialog>
  );
}
