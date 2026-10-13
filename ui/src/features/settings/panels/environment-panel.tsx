import { useCallback, useEffect, useState } from "react";

import { Badge, Button, Card, EmptyState, Field, Notice, Select, errorText } from "../../../components";
import { PanelHeader } from "../panel-header";
import * as api from "../../../ipc/commands";
import type { AiProviderId, AiTarget, CliEnvironmentStatus } from "../../../types";

export function EnvironmentPanel({
  providerId,
  targetId,
  selectedTarget,
  supportsSessionSettings,
  version,
  onOpenSessionSettings,
  onError,
  onNotify
}: {
  providerId: AiProviderId | "";
  targetId: string;
  /**
   * 当前选中的目标。
   *
   * 「WSL 会话目录」卡只在 WSL 目标上出现：本机目标的会话目录就是用户目录，
   * 手动指定没有意义（后端也只认 WSL 的 `CODEX_HOME` 覆盖）。
   */
  selectedTarget?: AiTarget | null;
  /**
   * 当前平台是否支持会话设置（WSL 的 `CODEX_HOME` 覆盖）。
   *
   * 「WSL 会话目录」卡是**三重**条件之一：平台支持、目标是 WSL、入口被接上。
   * 少了能力这一项，claude/qoder 的 WSL 目标也会出现这张卡——而后端对这两个平台
   * `sessionSettings=false`，保存写的是 Codex 的全局设置，点了必然失败。
   */
  supportsSessionSettings: boolean;
  /**
   * 应用版本。
   *
   * 由 `app.tsx` 取一次（`app:get-version`）后经 `SettingsCenter` 传下来：
   * 版本是**进程级常量**，面板自己再取一次只是多一次 IPC，而且「读不到」的兜底
   * 会分散在两处。此前这一页与 `app.tsx` 各取了一次，读到的还是同一个值。
   */
  version: string;
  /**
   * 打开「设置会话」对话框。
   *
   * 这是被移除的菜单栏「设置会话」的新入口（spec §11）：自动探测受发行版、
   * 用户、nvm/home 布局影响，探测不到时没有这个入口那台目标就等于用不了。
   * 对话框本体不在这个面板里——它由 `app.tsx` 统一渲染，这里只回传意图。
   */
  onOpenSessionSettings?: () => void;
  onError: (message: string) => void;
  /** 成功反馈。见 `SettingsCenter` 同名字段的说明。 */
  onNotify?: (message: string) => void;
}) {
  const [status, setStatus] = useState<CliEnvironmentStatus | null>(null);
  const [installing, setInstalling] = useState(false);
  /**
   * 要装的 Node 主版本。
   *
   * 初值是后端给的**推荐**版本（`recommendedNodeMajor`），而不是界面写死 24——
   * 写死的话后端换了推荐版本，界面还在装旧的。
   */
  const [nodeMajor, setNodeMajor] = useState<number | null>(null);

  /** 实际用的版本：用户没选过就用后端推荐的那个。 */
  const chosenNodeMajor = nodeMajor ?? status?.recommendedNodeMajor ?? 24;

  /**
   * 安装 CLI。
   *
   * **这一步可能很久**（下载 + npm 安装，分钟级），因此按钮必须给出「进行中」，
   * 否则用户会以为点了没反应而重复点击。
   */
  const runInstall = async () => {
    if (!status) return;
    setInstalling(true);
    try {
      await api.installCli({
        providerId,
        ...(targetId ? { targetId } : {}),
        // 把用户选的版本递下去。后端本来就有这个参数（默认是它的推荐值），
        // 此前界面没传，于是「24 LTS 推荐 / 22 LTS 兼容」这个选择在界面上不存在。
        nodeMajor: chosenNodeMajor,
        // 后端会据此决定要不要先装 Node：能直接装 CLI 就不必动 Node。
        ensureNode: !status.canInstallCli && status.canInstallNodeWithNvm
      });
      // 装 CLI 是**分钟级**的操作，用户会去看别的东西；不说一句，
      // 回来时既看不出装完没有、也看不出是不是点了没反应。文案照旧平台。
      onNotify?.("CLI 安装完成。");
      await load();
    } catch (error) {
      onError(errorText(error));
    } finally {
      setInstalling(false);
    }
  };

  const load = useCallback(async () => {
    if (!providerId) {
      setStatus(null);
      return;
    }
    try {
      // `targetId` 为空时后端检测本机环境。
      setStatus(await api.checkCliEnvironment(providerId, targetId || undefined));
    } catch (error) {
      onError(errorText(error));
    }
  }, [providerId, targetId, onError]);

  useEffect(() => {
    void load();
  }, [load]);

  // 两个早返回分支都要带面板头：它们都会真的出现（没选平台时、检测结果还没回来时），
  // 只剩一句裸提示的话，用户会以为自己离开了设置中心。与 `gateway-panel.tsx` 的
  // 与 `gateway-panel.tsx` 的「读取网关状态…」等待态同一处理。
  if (!providerId) {
    return (
      <>
        <PanelHeader
          title="运行时环境"
          description="选择已集成的 AI 厂商，在当前目标环境执行 npm 全局安装。Node 缺失或版本不足时，会优先使用已安装的 nvm 安装 Node。"
        />
        <EmptyState message="请先选择平台。" />
      </>
    );
  }
  if (!status) {
    return (
      <>
        <PanelHeader
          title="运行时环境"
          description="选择已集成的 AI 厂商，在当前目标环境执行 npm 全局安装。Node 缺失或版本不足时，会优先使用已安装的 nvm 安装 Node。"
        />
        <EmptyState message="检测中…" />
      </>
    );
  }

  /**
   * 状态行。与旧平台 `cli-installer-dialog.tsx` 的五项一致：
   * **官方 Node（最低版本）/ Node / npm / nvm / CLI**。
   *
   * nvm 那一行不能省：Node 版本不够时靠它兜底装 Node，
   * 用户看不到它就无法判断「能不能自动补 Node」。
   */
  const rows: { label: string; ok: boolean; value: string }[] = [
    {
      label: "官方 Node",
      ok: true,
      value: `${status.minimumNodeMajor}+`
    },
    {
      label: "Node",
      ok: status.nodeMeetsMinimum,
      value: status.nodeVersion ?? "未检测到"
    },
    {
      label: "npm",
      ok: status.hasNpm,
      value: status.npmVersion ?? "未检测到"
    },
    {
      label: "nvm",
      ok: status.hasNvm,
      value: status.nvmVersion ?? "未检测到"
    },
    {
      label: "CLI",
      ok: status.hasCli,
      value: status.cliVersion ?? "未安装"
    }
  ];

  return (
    <div className="environment-panel">
      {/* 面板头统一：原来那段开场说明搬到这里（同一句话，只是位置从正文变到头部）。
          「重新检测」**留在提示条里**（两条分支都有）：`Notice` 的约定是有问题就要给出口，
          抽到面板头要么让「环境正常」那条没有出口，要么同一页出现两个同名的按钮。 */}
      <PanelHeader
        title="运行时环境"
        description="选择已集成的 AI 厂商，在当前目标环境执行 npm 全局安装。Node 缺失或版本不足时，会优先使用已安装的 nvm 安装 Node。"
      />

      {/* 这次检测的是谁、用什么参数（Node 版本选择与应用版本都在这里）。 */}
      <Card title="目标与版本">
        <div className="field-grid">
          <Field label="目标环境">
            <span className="value">
              {status.scope === "wsl" ? "WSL 发行版内" : "本机"} · {status.providerId}
              {status.targetId ? ` · ${status.targetId}` : ""}
            </span>
          </Field>
          <Field label="平台">
            <span className="value">{status.platform || "-"}</span>
          </Field>
          {/*
            Node 版本选择。旧平台的两个选项就是「推荐」与「兼容」两个 LTS，
            因此这里也按主版本号给这两项，而不是让用户填一个任意数字——
            填 25 这种还没进 LTS 的版本不在支持范围内。
          */}
          <Field label="Node 版本" hint="CLI 装不上时会先用 nvm 装这个版本。">
            <Select
              value={String(chosenNodeMajor)}
              disabled={installing}
              onChange={(event) => setNodeMajor(Number(event.target.value))}
            >
              <option value={String(status.recommendedNodeMajor)}>
                {status.recommendedNodeMajor} LTS 推荐
              </option>
              {/* 推荐版本与最低版本相同时不重复列出。 */}
              {status.minimumNodeMajor !== status.recommendedNodeMajor && (
                <option value={String(status.minimumNodeMajor)}>
                  {status.minimumNodeMajor} LTS 兼容
                </option>
              )}
            </Select>
          </Field>
          <Field label="应用版本">
            {/* 版本来自 `SettingsCenter` 的 prop（`app.tsx` 取一次），不再自己发一次 IPC。 */}
            <span className="value ui-num">{version || "-"}</span>
          </Field>
        </div>

        {/* Node 由 nvm 管的时候，换版本会影响其他项目——说一句比不说好。 */}
        {status.nodeManagedByNvm && (
          <p className="field-hint">
            当前 Node 由 nvm 管理。切换版本会影响同一台机器上其他用 nvm 的项目。
          </p>
        )}
      </Card>

      {/* 五项探测结果是**同一次检测的读数**，散着排会让人以为是五件不相干的事；
          `dense` 收一档行距，五项在一屏内一眼扫完。 */}
      <Card title="工具链状态" description="Node 一行要求不低于官方 Node 的最低版本。">
        <table className="data-table dense">
          <thead>
            <tr>
              <th>检查项</th>
              <th>结果</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr key={row.label}>
                <td>{row.label}</td>
                <td>
                  {/* 版本号成列出现：`ui-num` 让数位对齐。 */}
                  <Badge small mono tone={row.ok ? "accent" : "neutral"}>
                    {row.value}
                  </Badge>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </Card>

      {/* WSL 目标的会话目录入口。三重条件缺一不可：**平台支持会话设置**
          （claude/qoder 为假——放行保存写的是 Codex 的全局设置，点了必然失败）、
          目标是 WSL、入口已接上。放在工具状态之后：它属于这台环境的一部分，
          但只有 WSL 目标才有——本机目标上出现它只会让人以为漏了什么。 */}
      {supportsSessionSettings && selectedTarget?.kind === "wsl" && onOpenSessionSettings && (
        <Card
          title="WSL 会话目录"
          description={`当前发行版：${selectedTarget.distro ?? "-"}（Codex 的 CODEX_HOME 覆盖）`}
        >
          <Button small onClick={onOpenSessionSettings} title="手动指定 WSL 内的 Codex 目录">
            设置会话目录…
          </Button>
        </Card>
      )}

      {/* 建议由后端算好，界面**直接显示**——在前后端各写一份判断逻辑，两份一定会分叉。 */}
      {status.messages.length > 0 && (
        <Notice
          message={
            <ul className="message-list">
              {status.messages.map((message) => (
                <li key={message}>{message}</li>
              ))}
            </ul>
          }
          actions={
            <>
              <Button small onClick={() => void load()}>
                重新检测
              </Button>
              {/* CLI 缺失时直接给安装入口——否则用户看到「未找到 codex」之后
                  仍然不知道该做什么。 */}
              {/*
                旧平台的条件是 `canInstallCli || canInstallNodeWithNvm`。
                此前这里只判 `canInstallCli`，于是**「没有 Node 但有 nvm」那种环境
                根本拿不到安装入口**——而那正是最需要这个按钮的情况。
              */}
              {!status.hasCli && (status.canInstallCli || status.canInstallNodeWithNvm) && (
                <Button small variant="primary" disabled={installing} onClick={() => void runInstall()}>
                  {installing
                    ? "安装中…（可能要几分钟）"
                    : status.canInstallCli
                      ? "安装 CLI"
                      : // 文案照旧平台：**说清这一步会顺带装 Node**，
                        // 否则用户以为只是装个 CLI，结果机器上的 Node 被换了。
                        `安装 Node ${chosenNodeMajor} 并安装 CLI`}
                </Button>
              )}
            </>
          }
        />
      )}
      {status.messages.length === 0 && (
        <Notice message="环境正常，没有需要注意的问题。" actions={<Button small onClick={() => void load()}>重新检测</Button>} />
      )}
    </div>
  );
}
