import { useCallback, useEffect, useState } from "react";

import { Button, ChipGroup, EmptyState, Notice, Toolbar, ToolbarSpacer } from "../../components";
import * as api from "../../ipc/commands";
import type { SkillEntry } from "../../types";

export function SkillsPanel({
  targetId,
  onError
}: {
  targetId: string;
  onError: (message: string) => void;
}) {
  const [skills, setSkills] = useState<SkillEntry[]>([]);
  const [loading, setLoading] = useState(false);
  /** 已安装 / 回收站。 */
  const [mode, setMode] = useState<"installed" | "trash">("installed");
  /** 上一次导入因重名被拒时记住源路径，供「覆盖导入」用。 */
  const [conflict, setConflict] = useState("");

  const load = useCallback(async () => {
    if (!targetId) {
      setSkills([]);
      return;
    }
    setLoading(true);
    try {
      setSkills(mode === "trash" ? await api.listTrashSkills(targetId) : await api.listSkills(targetId));
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    } finally {
      setLoading(false);
    }
  }, [targetId, mode, onError]);

  useEffect(() => {
    void load();
  }, [load]);

  const act = async (action: () => Promise<unknown>) => {
    try {
      await action();
      // 停用会**改目录名**（加 `.disabled` 后缀），因此必须重新拉列表——
      // 拿旧名字再操作会找不到。
      await load();
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    }
  };

  /**
   * 导入一个技能。
   *
   * 源路径用**原生目录选择框**取；`overwrite` 默认 `false`——覆盖不可逆，
   * 因此先按不覆盖试一次，被拒之后再让用户明确选择。
   */
  const importFrom = async (source: string, overwrite: boolean) => {
    try {
      await api.importSkill(targetId, source, overwrite);
      setConflict("");
      setMode("installed");
      await load();
    } catch (error) {
      // 重名是**预期内**的结果，不是异常：把它转成「要不要覆盖」的提问，
      // 而不是一句无从下手的错误。
      setConflict(source);
      if (overwrite) {
        onError(error instanceof Error ? error.message : String(error));
      }
    }
  };

  const pickAndImport = async () => {
    const source = await api.chooseDirectory("选择技能目录");
    // `null` 表示用户取消——正常操作，什么都不做。
    if (source) await importFrom(source, false);
  };

  /** 导入单个 `.md`（例如别人发来的 `SKILL.md`）。 */
  const pickAndImportFile = async () => {
    const source = await api.chooseFile("选择技能文件（SKILL.md）");
    if (source) await importFrom(source, false);
  };

  if (!targetId) return <EmptyState message="请先选择目标。" />;

  return (
    <>
      <Toolbar>
        <ChipGroup
          value={mode}
          onChange={(value) => setMode(value === "trash" ? "trash" : "installed")}
          options={[
            { value: "installed", label: "已安装" },
            { value: "trash", label: "回收站" }
          ]}
        />
        <ToolbarSpacer />
        <span className="row-meta">
          {loading ? "加载中…" : `${skills.length} 个`}
        </span>
        {mode === "installed" && (
          <>
            <Button small variant="primary" onClick={() => void pickAndImport()}>
              导入技能目录
            </Button>
            {/* 单个 `SKILL.md` 也是合法来源——后端两种都支持，此前只有目录入口。 */}
            <Button small onClick={() => void pickAndImportFile()}>
              导入 .md
            </Button>
          </>
        )}
        <Button small onClick={() => void api.openSkillFolder(targetId).catch(() => undefined)}>
          打开目录
        </Button>
      </Toolbar>

      {conflict && (
        <Notice
          message={`目标里已经有一个同名技能（来源：${conflict}）。`}
          actions={
            <>
              <Button small variant="danger" onClick={() => void importFrom(conflict, true)}>
                覆盖导入
              </Button>
              <Button small onClick={() => setConflict("")}>
                取消
              </Button>
            </>
          }
        />
      )}

      {!loading && skills.length === 0 && (
        <EmptyState
          message={mode === "trash" ? "回收站是空的。" : "这个目标下还没有技能。"}
          hint={mode === "trash" ? undefined : "把技能目录放进目标的 skills/ 下即可。"}
        />
      )}

      {skills.map((skill) => (
        <div key={skill.path} className="list-row">
          <div className="list-row-main">
            <span className="list-row-title">{skill.name}</span>
            <span className="list-row-sub">{skill.description || "（无描述）"}</span>
          </div>
          {mode === "installed" ? (
            <>
              <span className={`tag ${skill.enabled ? "ok" : "off"}`}>
                {skill.enabled ? "已启用" : "已停用"}
              </span>
              <Button
                small
                onClick={() =>
                  void act(() => api.setSkillEnabled(targetId, skill.name, !skill.enabled))
                }
              >
                {skill.enabled ? "停用" : "启用"}
              </Button>
            </>
          ) : (
            <>
              <Button
                small
                onClick={() => void act(() => api.restoreSkill(targetId, skill.name))}
              >
                还原
              </Button>
              <Button
                small
                variant="danger"
                onClick={() => void act(() => api.purgeSkill(targetId, skill.name))}
              >
                彻底删除
              </Button>
            </>
          )}
        </div>
      ))}
    </>
  );
}
