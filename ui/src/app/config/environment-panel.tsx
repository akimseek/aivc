import { useCallback, useEffect, useState } from "react";

import { Button, EmptyState, Field, Notice } from "../../components";
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

  const rows: { label: string; ok: boolean; value: string }[] = [
    { label: "Node", ok: status.hasNode, value: status.nodeVersion ?? "未找到" },
    {
      label: "Node 版本",
      ok: status.nodeMeetsMinimum,
      value: status.nodeMeetsMinimum ? "满足最低版本" : "低于最低版本"
    },
    { label: "npm", ok: status.hasNpm, value: status.hasNpm ? "已安装" : "未找到" },
    { label: "CLI", ok: status.hasCli, value: status.hasCli ? "已安装" : "未找到" }
  ];

  return (
    <>
      <div className="field-grid">
        <Field label="作用范围">
          <span className="value">
            {status.scope === "wsl" ? "WSL 发行版内" : "本机"} · {status.providerId}
          </span>
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
              {!status.hasCli && status.canInstallCli && (
                <Button small variant="primary" disabled={installing} onClick={() => void runInstall()}>
                  {installing ? "安装中…（可能要几分钟）" : "安装 CLI"}
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

