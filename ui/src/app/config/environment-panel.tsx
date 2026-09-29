import { useCallback, useEffect, useState } from "react";

import { Button, EmptyState, Field, Notice, Select } from "../../components";
import * as api from "../../ipc/commands";
import type { AiProviderId, CliEnvironmentStatus } from "../../types";

export function EnvironmentPanel({
  providerId,
  targetId,
  onError
}: {
  providerId: AiProviderId | "";
  targetId: string;
  onError: (message: string) => void;
}) {
  const [status, setStatus] = useState<CliEnvironmentStatus | null>(null);
  const [version, setVersion] = useState("");
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
      await load();
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
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
      onError(error instanceof Error ? error.message : String(error));
    }
  }, [providerId, targetId, onError]);

  useEffect(() => {
    void load();
    void api
      .getAppVersion()
      .then(setVersion)
      .catch(() => undefined);
  }, [load]);

  if (!providerId) return <EmptyState message="请先选择平台。" />;
  if (!status) return <EmptyState message="检测中…" />;

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
    <>
      <p className="field-hint">
        选择已集成的 AI 厂商，在当前目标环境执行 npm 全局安装。Node 缺失或版本不足时，
        会优先使用已安装的 nvm 安装 Node。
      </p>

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
          <span className="value">{version || "-"}</span>
        </Field>
      </div>

      <table className="data-table">
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
                <span className={`tag ${row.ok ? "ok" : "off"}`}>{row.value}</span>
              </td>
            </tr>
          ))}
        </tbody>
      </table>

      {/* Node 由 nvm 管的时候，换版本会影响其他项目——说一句比不说好。 */}
      {status.nodeManagedByNvm && (
        <p className="field-hint">
          当前 Node 由 nvm 管理。切换版本会影响同一台机器上其他用 nvm 的项目。
        </p>
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
    </>
  );
}

