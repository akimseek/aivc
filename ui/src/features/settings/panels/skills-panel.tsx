import { useCallback, useEffect, useState } from "react";

import { Badge, Button, Card, ChipGroup, EmptyState, Notice, Toolbar, ToolbarSpacer, errorText } from "../../../components";
import * as api from "../../../ipc/commands";
import { PanelHeader } from "../panel-header";
import type { SkillEntry } from "../../../types";

export function SkillsPanel({
  targetId,
  onError,
  onNotify
}: {
  targetId: string;
  onError: (message: string) => void;
  /** 成功反馈。见 `SettingsCenter` 同名字段的说明。 */
  onNotify?: (message: string) => void;
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
      onError(errorText(error));
    } finally {
      setLoading(false);
    }
  }, [targetId, mode, onError]);

  useEffect(() => {
    void load();
  }, [load]);

  /**
   * 做一个会改技能状态的动作。
   *
   * `label` 是**做完之后要说的话**（旧平台每个动作都有一句：`Skill 已启用。` 等）。
   * 传进来而不是从 action 里推：一个匿名闭包看不出它做了什么，而结果提示正是给
   * 用户看的「刚才那一下生效了没有」。
   */
  const act = async (label: string, action: () => Promise<unknown>) => {
    try {
      await action();
      // 停用会**改目录名**（加 `.disabled` 后缀），因此必须重新拉列表——
      // 拿旧名字再操作会找不到。
      onNotify?.(label);
      await load();
    } catch (error) {
      onError(errorText(error));
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
      onNotify?.(`Skill 已导入：${source}`);
      await load();
    } catch (error) {
      // 重名是**预期内**的结果，不是异常：把它转成「要不要覆盖」的提问，
      // 而不是一句无从下手的错误。
      setConflict(source);
      if (overwrite) {
        onError(errorText(error));
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

  if (!targetId) {
    // 这个分支是可达的（没选目标时导航仍然能点进来）。面板头要跟着一起出来——
    // 只剩一句裸提示的话，用户会以为自己离开了设置中心。与 `gateway-panel.tsx`
    // 的等待态、`diagnostics-panel.tsx` 的空目标分支同一处理。
    return (
      <>
        <PanelHeader
          title="平台技能 (Skills)"
          description="管理注入到各 AI CLI 运行时的自定义 Skill 扩展包：导入、启停与回收站。"
        />
        <EmptyState message="请先选择目标。" />
      </>
    );
  }

  return (
    <div className="skills-panel">
      <PanelHeader
        title="平台技能 (Skills)"
        description="管理注入到各 AI CLI 运行时的自定义 Skill 扩展包：导入、启停与回收站。"
        actions={
          <>
            {/* 导入是这一页的主操作（demo 也放在面板头），因此给 primary；
                「打开目录」只是偶尔一用的旁路动作，保持默认档。 */}
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
          </>
        }
      />

      {/* 已安装 / 回收站的切换与计数：它们说的是「下面这张卡在列什么」。 */}
      <Toolbar>
        <ChipGroup
          value={mode}
          ariaLabel="技能列表来源"
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

      {/* 技能列表收进一张卡：一个目标的技能是一个整体，散着排会让人以为
          每一行都是独立的一块。空态也放进卡里，页面上只有这一块能是空的。
          加载中且还什么都没有时**不出空列表容器**：一个空的 `.list-rows` 会占着
          卡片的间距，看起来像「已经加载完，只是没有内容」。 */}
      <Card>
        {skills.length === 0 ? (
          loading ? null : (
            <EmptyState
              message={mode === "trash" ? "回收站是空的。" : "这个目标下还没有技能。"}
              hint={mode === "trash" ? undefined : "把技能目录放进目标的 skills/ 下即可。"}
            />
          )
        ) : (
          <div className="list-rows">
            {skills.map((skill) => (
              <div key={skill.path} className="list-row">
                <div className="list-row-main">
                  <span className="list-row-title">{skill.name}</span>
                  <span className="list-row-sub">{skill.description || "（无描述）"}</span>
                </div>
                {mode === "installed" ? (
                  <>
                    <Badge tone={skill.enabled ? "accent" : "neutral"}>
                      {/* 用词与旧平台一致：**禁用**，不是「停用」。按钮、标签、
                          结果提示三处必须是同一个词，否则同一件事在界面上有三个名字。 */}
                      {skill.enabled ? "已启用" : "已禁用"}
                    </Badge>
                    <Button
                      small
                      onClick={() =>
                        void act(
                          // 文案照旧平台：`Skill 已启用。` / `Skill 已禁用。`
                          skill.enabled
                            ? `Skill 已禁用：${skill.name}`
                            : `Skill 已启用：${skill.name}`,
                          () => api.setSkillEnabled(targetId, skill.name, !skill.enabled)
                        )
                      }
                    >
                      {skill.enabled ? "禁用" : "启用"}
                    </Button>
                    {/* 移入回收站（可还原）。销毁是回收站视图里的另一个动作——
                        两个动作、两个位置，避免在安装列表里误触不可逆的那一个。
                        与会话的同类操作一样走 danger 档：它是这一行里唯一会「删东西」的。 */}
                    <Button
                      small
                      variant="danger"
                      onClick={() =>
                        void act(`Skill 已移入回收站：${skill.name}`, () =>
                          api.deleteSkill(targetId, skill.name)
                        )
                      }
                    >
                      移入回收站
                    </Button>
                  </>
                ) : (
                  <>
                    <Button
                      small
                      onClick={() =>
                        void act(`Skill 已恢复：${skill.name}`, () =>
                          api.restoreSkill(targetId, skill.name)
                        )
                      }
                    >
                      还原
                    </Button>
                    {/* 彻底删除不可逆：它是这一行里唯一见红的操作。 */}
                    <Button
                      small
                      variant="danger"
                      onClick={() =>
                        void act(`Skill 已彻底删除：${skill.name}`, () =>
                          api.purgeSkill(targetId, skill.name)
                        )
                      }
                    >
                      彻底删除
                    </Button>
                  </>
                )}
              </div>
            ))}
          </div>
        )}
      </Card>
    </div>
  );
}
