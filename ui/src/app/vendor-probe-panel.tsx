import { useCallback, useState } from "react";

import { Button, Card, EmptyState, LoadingBlock, Toolbar, ToolbarSpacer, useToast } from "../components";
import * as api from "../ipc/commands";
import type { ApiVendor, VendorBalance, VendorConfigFile, VendorModel } from "../types";

/**
 * 供应商的「探活」三件事：**测连接 / 查余额 / 读配置**。
 *
 * # 为什么这三件要放在一起
 *
 * 它们回答的是同一个问题的三个侧面：「这家供应商我配对了吗」。
 *
 * | 动作 | 它真正验证的是什么 |
 * | --- | --- |
 * | 测连接（列模型） | 地址 + 密钥**真的能用**，而且对方确实在服务 |
 * | 查余额 | 这个中转站会不会告诉我还剩多少（很多不返回，那不是配错） |
 * | 读配置 | CLI 那边**实际读到的**是什么——界面里的表和它可能不一致 |
 *
 * 三条后端命令（`vendor:list-models` / `vendor:refresh-balance` /
 * `vendor:read-configs`）一直在，界面此前一个都没接。
 *
 * # 余额的三态
 *
 * 「查不到」与「余额为 0」必须分开：前者是上游没提供这个接口，后者是钱真的用完了。
 * 把前者显示成 `0` 会让用户以为该充值了。
 */

/** 余额的一句话。 */
export function balanceText(balance: VendorBalance | undefined): string {
  if (!balance) return "未提供";
  if (balance.remaining !== undefined) {
    return `${balance.remaining}${balance.unit ? ` ${balance.unit}` : ""}`;
  }
  if (balance.used !== undefined) return `已用 ${balance.used}${balance.unit ? ` ${balance.unit}` : ""}`;
  if (balance.planName) return balance.planName;
  // 上游返回了结构但没有任何数字——那不是 0，是「没这个信息」。
  return "未提供";
}

/**
 * CLI 会读的那几个配置文件。
 *
 * # 为什么是固定的三个
 *
 * 后端**强制**路径白名单（`assert_allowed_config_path`）：必须以 `~/` 或 `$HOME/` 开头，
 * 且落在 `.codex/`、`.gemini/`、`.claude/` 之下。这不是限制，是安全边界——
 * 不加限制的话这条命令就成了「读取任意文件」的入口，而配置文件里通常有 API Key。
 *
 * 所以界面**只提供这三个按钮**，不做一个可以随便填路径的输入框：
 * 那样用户填了别的路径只会收到一句「路径不允许」，还以为是功能坏了。
 */
const CONFIG_FILES = [
  { label: "Codex 配置", path: "~/.codex/config.toml" },
  { label: "Gemini 配置", path: "~/.gemini/settings.json" },
  { label: "Claude 配置", path: "~/.claude/settings.json" }
];

export function VendorProbePanel({
  vendors,
  onError
}: {
  vendors: ApiVendor[];
  onError: (message: string) => void;
}) {
  const [vendorId, setVendorId] = useState("");
  const [models, setModels] = useState<VendorModel[] | null>(null);
  const [balance, setBalance] = useState<VendorBalance | null>(null);
  const [busy, setBusy] = useState<"" | "models" | "balance" | "all">("");
  const [file, setFile] = useState<VendorConfigFile | null>(null);
  const [configError, setConfigError] = useState("");
  const toast = useToast();

  const selected = vendors.find((item) => item.id === vendorId) ?? null;

  /** 列模型 = 测连接。地址或密钥不对，这一步会给出后端那句原始错误。 */
  const probe = useCallback(async () => {
    if (!vendorId) return;
    setBusy("models");
    setModels(null);
    try {
      setModels(await api.listVendorModels(vendorId));
    } catch (problem) {
      onError(problem instanceof Error ? problem.message : String(problem));
    } finally {
      setBusy("");
    }
  }, [vendorId, onError]);

  const refreshOne = useCallback(async () => {
    if (!vendorId) return;
    setBusy("balance");
    setBalance(null);
    try {
      const result = await api.refreshVendorBalance(vendorId);
      if (result.error) {
        onError(`余额查询失败：${result.error}`);
        setBalance(null);
      } else {
        setBalance(result.balance ?? null);
      }
    } catch (problem) {
      onError(problem instanceof Error ? problem.message : String(problem));
    } finally {
      setBusy("");
    }
  }, [vendorId, onError]);

  /**
   * 刷新全部。
   *
   * 后端**按批次并发**（避免给中转站造成瞬时压力），因此这一步会花几秒。
   * 返回的每一项各自成功或失败——一家失败不影响其他家，所以这里逐条报，
   * 而不是整体成功/失败。
   */
  const refreshAll = useCallback(async () => {
    setBusy("all");
    try {
      const results = await api.refreshAllVendorBalances();
      const ok = results.filter((item) => item.balance).length;
      const failed = results.length - ok;
      toast.show(
        failed === 0
          ? `已刷新 ${ok} 家供应商的余额。`
          : `${ok} 家成功，${failed} 家未提供或查询失败。`
      );
    } catch (problem) {
      onError(problem instanceof Error ? problem.message : String(problem));
    } finally {
      setBusy("");
    }
  }, [toast, onError]);

  const readConfig = useCallback(
    async (path: string) => {
      setConfigError("");
      setFile(null);
      try {
        const files = await api.readVendorConfigs([path]);
        setFile(files[0] ?? null);
      } catch (problem) {
        setConfigError(problem instanceof Error ? problem.message : String(problem));
      }
    },
    []
  );

  if (vendors.length === 0) {
    return (
      <Card title="测试连接与用量" description="先配一家供应商，这里才有可测的对象。">
        <EmptyState message="这个平台还没有配置供应商。" />
      </Card>
    );
  }

  return (
    <>
      {toast.message && (
        <div className="export-hint" role="status">
          {toast.message}
          <Button small onClick={toast.dismiss}>
            知道了
          </Button>
        </div>
      )}

      <Card
        title="测试连接"
        description="向供应商的 HTTP 接口查可用模型。这一步会真的发出请求，因此地址或密钥不对时会失败——那正是它的用途。"
        actions={
          <Toolbar>
            <Button small onClick={() => void refreshAll()} disabled={busy !== ""}>
              刷新全部余额
            </Button>
            <ToolbarSpacer />
          </Toolbar>
        }
      >
        <Toolbar>
          <label className="field">
            <span className="field-label">供应商</span>
            <select
              className="control control-select"
              value={vendorId}
              onChange={(event) => {
                setVendorId(event.target.value);
                // 换了供应商就把上一家的结果清掉：留着会让人以为那是新家的。
                setModels(null);
                setBalance(null);
              }}
            >
              <option value="">请选择</option>
              {vendors.map((vendor) => (
                <option key={vendor.id} value={vendor.id}>
                  {vendor.name}
                  {vendor.enabled ? "" : "（已禁用）"}
                </option>
              ))}
            </select>
          </label>

          <ToolbarSpacer />

          <Button small disabled={!selected || busy !== ""} onClick={() => void probe()}>
            查询模型
          </Button>
          <Button small disabled={!selected || busy !== ""} onClick={() => void refreshOne()}>
            查询余额
          </Button>
        </Toolbar>

        {busy === "models" && <LoadingBlock label="正在查询模型…" />}
        {busy === "balance" && <LoadingBlock label="正在查询余额…" />}
        {busy === "all" && <LoadingBlock label="正在刷新全部余额（按批次并发）…" />}

        {models && (
          <p className="value">
            可用模型 <b>{models.length}</b> 个：
            <span className="mono">{models.map((item) => item.id).join("、") || "（接口返回了空列表）"}</span>
          </p>
        )}

        {/* 余额的**三态**：未查询 / 已查询但上游未提供 / 有数字。
            把「未提供」显示成 `0` 会让用户以为该充值了。 */}
        {balance !== null && (
          <p className="value">
            余额 <b>{balanceText(balance)}</b>
            {balance.isValid === false && <span className="tag off">密钥可能无效</span>}
          </p>
        )}
        {busy === "" && selected && balance === null && models !== null && (
          <p className="value">余额：未查询</p>
        )}
      </Card>

      <Card
        title="CLI 实际读到的配置"
        description="界面里的供应商表是应用自己的记录；CLI 真正读的是这几个文件。两者不一致时，问题就在这里。"
      >
        <Toolbar>
          {CONFIG_FILES.map((item) => (
            <Button key={item.path} small onClick={() => void readConfig(item.path)} title={item.path}>
              {item.label}
            </Button>
          ))}
        </Toolbar>

        {configError && <EmptyState message="读不到这个配置文件。" hint={configError} />}

        {file && (
          <>
            <p className="value mono" title={file.path}>
              {file.path}
            </p>
            {file.content.trim() ? (
              <pre className="config-view">{file.content}</pre>
            ) : (
              <EmptyState
                message="这个文件是空的，或不存在。"
                hint="不存在时后端返回空内容而不是报错——那说明 CLI 还没写过它。"
              />
            )}
          </>
        )}
      </Card>
    </>
  );
}
