import { useState } from "react";

import { Button, TextInput } from "../../components";

/**
 * 手动指定 WSL 内的 Codex 目录。
 *
 * # 为什么它必须出现在「目标不可用」的告警里
 *
 * 发行版进不去时，自动探测**注定失败**；而用户往往知道自己的 Codex 装在哪。
 * 这条路径此前只藏在菜单里（甚至压根没有入口），于是「目标不可用」变成了死路。
 *
 * 保存后必须**重新探测**——否则界面还停在旧结果上，用户会以为没生效。
 * 重探这一步在调用方（`use-console` 的 `configureWslCodexHome`）里，
 * 这里只负责把「要保存什么」交出去。
 */
export function WslHomeForm({
  distro,
  onSave,
  onClear
}: {
  distro: string;
  onSave: (codexHome: string) => void;
  onClear: () => void;
}) {
  const [draft, setDraft] = useState("");
  return (
    <div className="wsl-home-form">
      <TextInput
        value={draft}
        placeholder={`${distro} 内的路径，例如 /home/你/.codex`}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && draft.trim()) onSave(draft.trim());
        }}
      />
      <div className="wsl-home-form-actions">
        <Button small disabled={!draft.trim()} onClick={() => onSave(draft.trim())}>
          指定并重新探测
        </Button>
        <Button small onClick={onClear} title="清除人工指定，回到自动探测">
          清除
        </Button>
      </div>
    </div>
  );
}
