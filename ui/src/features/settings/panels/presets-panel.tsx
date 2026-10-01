import { useCallback, useEffect, useState } from "react";

import { Button, Card, EmptyState, Field, TextInput, Toolbar } from "../../../components";
import { PanelHeader } from "../panel-header";
import * as api from "../../../ipc/commands";
import type { WorkspacePreset } from "../../../types";

/**
 * 工作区预设：把「用哪个目录、带什么提示词、附加什么 CLI 参数」存下来复用。
 *
 * 去重键是 `cwd` + 目标类型——同一个目录配两次没有意义，后端会替换旧的那条。
 */
export function PresetsPanel({ onError }: { onError: (message: string) => void }) {
  const [presets, setPresets] = useState<WorkspacePreset[]>([]);
  const [name, setName] = useState("");
  const [cwd, setCwd] = useState("");

  const load = useCallback(async () => {
    try {
      setPresets(await api.listPresets());
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    }
  }, [onError]);

  useEffect(() => {
    void load();
  }, [load]);

  const save = async () => {
    if (!cwd.trim()) return;
    try {
      await api.savePreset({ name: name.trim(), cwd: cwd.trim() });
      setName("");
      setCwd("");
      await load();
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    }
  };

  return (
    <div className="presets-panel">
      <PanelHeader
        title="工作区预设"
        description="保存常用的工作目录。同一个目录重复保存时会替换旧的那条。"
      />

      {/* 表单一张卡、列表一张卡：可写的那一块与只读列表分开，改错了不会看串行。 */}
      <Card title="新增预设">
        <div className="field-grid">
          <Field label="名称" hint="留空时用工作目录作为名称。">
            <TextInput value={name} onChange={(event) => setName(event.target.value)} />
          </Field>
          <Field label="工作目录" hint="必填：空目录的预设没有意义。">
            <TextInput
              value={cwd}
              placeholder="例如 /home/akim/code/ai"
              onChange={(event) => setCwd(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") void save();
              }}
            />
          </Field>
        </div>
        {/* 「新增」是这一页唯一的主操作：填了目录它才可点，与 `Field` 的「必填」呼应。 */}
        <Toolbar>
          <Button small variant="primary" disabled={!cwd.trim()} onClick={() => void save()}>
            新增预设
          </Button>
        </Toolbar>
      </Card>

      <Card title="已保存的预设">
        {presets.length === 0 ? (
          <EmptyState message="还没有工作区预设。" />
        ) : (
          <div className="list-rows">
            {presets.map((preset) => (
              <div key={preset.id} className="list-row">
                <div className="list-row-main">
                  <span className="list-row-title">{preset.name}</span>
                  <span className="list-row-sub">{preset.cwd}</span>
                </div>
                {/* 删除是这一行唯一的动作，也是不可逆的那个：它是这一行里唯一见红的操作。 */}
                <Button
                  small
                  variant="danger"
                  onClick={() =>
                    void api
                      .deletePreset(preset.id)
                      .then(load)
                      .catch((error: unknown) =>
                        onError(error instanceof Error ? error.message : String(error))
                      )
                  }
                >
                  删除
                </Button>
              </div>
            ))}
          </div>
        )}
      </Card>
    </div>
  );
}
