import { useCallback, useEffect, useState } from "react";

import { Button, Card, EmptyState } from "../../components";
import { PanelHeader } from "../../features/settings/panel-header";
import * as api from "../../ipc/commands";
import type { AiProviderId, AiProviderSummary, AiTarget } from "../../types";

/**
 * 诊断：把「每一道门的结果」集中到一处。
 *
 * 这一页存在的理由很具体：**用户遇到问题时，界面上通常只有一句结论**，
 * 而真正有用的是「哪些目标可用、各自为什么不可用、这个平台到底有哪些能力」。
 * 这些信息本来就有（目标自带 `detail`、平台自带 `capabilities`），
 * 只是一直没有一个地方把它们并排列出来。
 *
 * # 与旧平台 `provider-status-dialog.tsx` 的对照
 *
 * 旧平台那一页有四张表：平台信息 / **路径（带存在性探测）** / **命令清单** / **能力矩阵**。
 * 这里做了两张（平台 + 能力矩阵），并给「配置目录」加了存在性探测。
 *
 * **没有做的那两样是有原因的，不是漏了**：
 *
 * - **历史目录 / 回收站目录**：目录名是 Rust 侧的约定，而且**四个平台各不相同**
 *   （`ClaudeLayout::projects_root` / `.visual-console-trash` 之类）。
 *   在界面里抄一份必然漂移——抄错的后果是「探测结果说目录不存在」，
 *   而用户会去查一个根本没配错的东西。要正确显示需要 provider 层暴露目录布局。
 * - **命令清单**：旧平台那张表是**硬编码在界面里的**，而且实测与后端不一致
 *   （后端既有 `--resume <id>`，也有 `resume --all <id>`）。照抄会写错命令，
 *   而这一页的用途恰恰是「照着它敲」。同样需要后端提供。
 */

/** 能探测存在性的路径。目前只有配置目录——它在目标的 DTO 里。 */
type PathProbe = {
  label: string;
  path: string;
  state: "checking" | "present" | "missing";
};

export function DiagnosticsPanel({
  targets,
  selectedTargetId,
  providers,
  selectedProviderId
}: {
  targets: AiTarget[];
  selectedTargetId: string;
  /** 全部平台：能力矩阵从它取。 */
  providers: AiProviderSummary[];
  /** 当前平台：能力矩阵要显示它的那一行。 */
  selectedProviderId: AiProviderId | "";
}) {
  /** 路径探测结果，按路径存。 */
  const [probes, setProbes] = useState<Record<string, PathProbe["state"]>>({});

  /**
   * 探测一条路径是否存在。
   *
   * 只在**本机**目标上探：`shell:path-exists` 对 WSL 目标会显式拒绝
   * （它按本机路径检查会把「WSL 里有」误判成「没有」），因此 WSL 目标直接标成
   * 「无法检查」而不是给出一个错的结论。
   */
  const probe = useCallback(
    async (target: AiTarget, path: string) => {
      setProbes((current) => ({ ...current, [path]: "checking" }));
      try {
        const exists = await api.pathExists(path, target.id);
        setProbes((current) => ({ ...current, [path]: exists ? "present" : "missing" }));
      } catch {
        // 探测不了（WSL / 权限）时不写结论：留着「检查中」比写一个错的更诚实。
        setProbes((current) => ({ ...current, [path]: "checking" }));
      }
    },
    []
  );

  const target = targets.find((item) => item.id === selectedTargetId) ?? null;
  const provider = providers.find((item) => item.id === selectedProviderId) ?? null;

  /** 配置目录。只有本机目标能直接探。 */
  const configPath = target?.codexHome ?? "";

  useEffect(() => {
    if (!target || !configPath || target.kind !== "local") return;
    void probe(target, configPath);
  }, [target, configPath, probe]);

  if (targets.length === 0) {
    return (
      <>
        <PanelHeader
          title="系统诊断"
          description="列出每个目标的 CLI 命令探测结果、配置路径与平台能力矩阵。"
        />
        <EmptyState message="还没有探测到任何目标。" hint="先在上方选择平台。" />
      </>
    );
  }

  const pathState = (path: string): PathProbe["state"] => probes[path] ?? "checking";
  const pathLabel = (state: PathProbe["state"]) =>
    state === "present" ? "存在" : state === "missing" ? "不存在" : "未检查";

  return (
    <div className="diagnostics-panel">
      <PanelHeader
        title="系统诊断"
        description="列出每个目标的 CLI 命令探测结果、配置路径与平台能力矩阵。"
      />

      {/* 目标表：一行一个目标，行里带后端给的 `detail`。计数从散行收进卡片头——
          它与表说的是同一件事。 */}
      <Card
        title="目标状态"
        description={`共 ${targets.length} 个目标 · ${targets.filter((item) => item.available).length} 个可用`}
      >
        <table className="data-table dense">
          <thead>
            <tr>
              <th>目标</th>
              <th>类型</th>
              <th>命令状态</th>
              <th>探测结果</th>
            </tr>
          </thead>
          <tbody>
            {targets.map((item) => (
              <tr key={item.id}>
                <td>
                  {item.label}
                  {item.id === selectedTargetId && <span className="tag ok">当前</span>}
                </td>
                <td>{item.kind === "wsl" ? `WSL · ${item.distro ?? "-"}` : "本机"}</td>
                <td>
                  {/* 旧平台这一格写的是「可用 / 未找到命令」，与 `available` 同义。 */}
                  <span className={`tag ${item.available ? "ok" : "off"}`}>
                    {item.available ? "可用" : "未找到命令"}
                  </span>
                </td>
                {/* `detail` 是后端给出的四态说明，**原样显示**——它已经是最好的解释。 */}
                <td className="mono ui-num">{item.detail ?? "-"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </Card>

      {/* 路径：旧平台那一页也有，并逐条做存在性探测。 */}
      {target && configPath && (
        <Card
          title="路径"
          description="只列后端告诉我们的目录；历史目录与回收站需要 provider 暴露布局才能显示。"
          actions={
            /*
              「重新检测路径」只在**本机**目标出现（与探测本身同一个条件）：
              WSL 的路径在本机看来毫无意义，探测不做，重试入口也不该有。
            */
            target.kind === "local" ? (
              <Button small onClick={() => void probe(target, configPath)}>
                重新检测路径
              </Button>
            ) : undefined
          }
        >
          <table className="data-table dense">
            <thead>
              <tr>
                <th>用途</th>
                <th>路径</th>
                <th>存在性</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td>配置目录</td>
                <td className="mono ui-num">{configPath}</td>
                <td>
                  {target.kind === "local" ? (
                    <span className={`tag ${pathState(configPath) === "present" ? "ok" : "off"}`}>
                      {pathLabel(pathState(configPath))}
                    </span>
                  ) : (
                    // WSL 目标的路径在本机看来毫无意义，**不猜**。
                    <span className="row-meta">WSL 目标，不做本机探测</span>
                  )}
                </td>
              </tr>
            </tbody>
          </table>
        </Card>
      )}

      {/* 能力矩阵：哪些入口在这个平台上是有意义的。 */}
      {provider && (
        <Card title="能力矩阵" description={provider.label}>
          <table className="data-table dense">
            <thead>
              <tr>
                <th>能力</th>
                <th>结果</th>
              </tr>
            </thead>
            <tbody>
              {capabilityRows(provider).map((row) => (
                <tr key={row.label}>
                  <td>{row.label}</td>
                  <td>
                    <span className={`tag ${row.enabled ? "ok" : "off"}`}>
                      {row.enabled ? "支持" : "不支持"}
                    </span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
      )}
    </div>
  );
}

/**
 * 能力矩阵的行。
 *
 * 键名与后端的 `ProviderCapabilities`（camelCase）一致——写错就是「永远是假的」，
 * 而这种错在界面上只表现为「这个平台什么都不支持」。
 */
export function capabilityRows(provider: AiProviderSummary): { label: string; enabled: boolean }[] {
  const has = (key: string) => provider.capabilities[key] === true;
  return [
    { label: "Skill", enabled: has("skills") },
    { label: "分支", enabled: has("branch") },
    { label: "Usage", enabled: has("usage") },
    { label: "回收站", enabled: has("trash") },
    { label: "批量", enabled: has("batchActions") },
    { label: "工作目录", enabled: has("customCwd") },
    { label: "导出", enabled: has("export") },
    { label: "会话设置", enabled: has("sessionSettings") },
    { label: "复制", enabled: has("duplicate") },
    { label: "供应商管理", enabled: has("vendorManagement") }
  ];
}
