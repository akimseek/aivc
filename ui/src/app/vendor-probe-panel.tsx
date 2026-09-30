import { useCallback, useState } from "react";

import { Button, Card, EmptyState, LoadingBlock, Select, Toolbar, ToolbarSpacer } from "../components";
import * as api from "../ipc/commands";
import type { ApiVendor, VendorBalance, VendorConfigFile, VendorModel } from "../types";

/**
 * 供应商的「探活」两件事：**测连接 / 读配置**。
 *
 * # 为什么这两件要放在一起
 *
 * 它们回答的是同一个问题的两个侧面：「这家供应商我配对了吗」。
 *
 * | 动作 | 它真正验证的是什么 |
 * | --- | --- |
 * | 测连接（列模型） | 地址 + 密钥**真的能用**，而且对方确实在服务 |
 * | 读配置 | CLI 那边**实际读到的**是什么——界面里的表和它可能不一致 |
 *
 * 两件都**不再是常驻卡片**：批次二把供应商页重排成「上面是主表、下面是工具」，
 * 由面板头的两个按钮把这里切出来（见 `config/vendors-panel.tsx`）。
 *
 * # 余额去哪了
 *
 * 余额查询不再在这里显示：它按供应商行内呈现（每一行有自己的状态与刷新入口），
 * 因为「哪一家还有多少钱」是看表时要能一眼比出来的信息，塞在另一张卡里对不上号。
 * 这里只留 `balanceText` —— 行内状态与它共用同一套文案，不能有两份。
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

/**
 * 测连接：选一家供应商，列它的可用模型。
 *
 * **「选供应商 → 查模型」这个形态不能丢**：这条命令要的是某一个供应商的地址与密钥，
 * 没有「默认那一家」——列表里的第一行未必是你要测的那家。
 */
export function VendorModelProbe({
  vendors,
  onError
}: {
  vendors: ApiVendor[];
  onError: (message: string) => void;
}) {
  const [vendorId, setVendorId] = useState("");
  const [models, setModels] = useState<VendorModel[] | null>(null);
  const [busy, setBusy] = useState(false);

  const selected = vendors.find((item) => item.id === vendorId) ?? null;

  /** 列模型 = 测连接。地址或密钥不对，这一步会给出后端那句原始错误。 */
  const probe = useCallback(async () => {
    if (!vendorId) return;
    setBusy(true);
    setModels(null);
    try {
      setModels(await api.listVendorModels(vendorId));
    } catch (problem) {
      onError(problem instanceof Error ? problem.message : String(problem));
    } finally {
      setBusy(false);
    }
  }, [vendorId, onError]);

  if (vendors.length === 0) {
    return (
      <Card title="测连接（列模型）" description="先配一家供应商，这里才有可测的对象。">
        <EmptyState message="这个平台还没有配置供应商。" />
      </Card>
    );
  }

  return (
    <Card
      title="测连接（列模型）"
      description="向供应商的 HTTP 接口查可用模型。这一步会真的发出请求，因此地址或密钥不对时会失败——那正是它的用途。"
    >
      <Toolbar>
        <label className="field">
          <span className="field-label">供应商</span>
          <Select
            value={vendorId}
            onChange={(event) => {
              setVendorId(event.target.value);
              // 换了供应商就把上一家的结果清掉：留着会让人以为那是新家的。
              setModels(null);
            }}
          >
            <option value="">请选择</option>
            {vendors.map((vendor) => (
              <option key={vendor.id} value={vendor.id}>
                {vendor.name}
                {vendor.enabled ? "" : "（已禁用）"}
              </option>
            ))}
          </Select>
        </label>

        <ToolbarSpacer />

        <Button small disabled={!selected || busy} onClick={() => void probe()}>
          查询模型
        </Button>
      </Toolbar>

      {busy && <LoadingBlock label="正在查询模型…" />}

      {models && (
        <p className="value">
          可用模型 <b>{models.length}</b> 个：
          <span className="mono">{models.map((item) => item.id).join("、") || "（接口返回了空列表）"}</span>
        </p>
      )}
    </Card>
  );
}

/** 读配置：CLI 那边**实际读到的**是什么。界面里的表与它可能不一致。 */
export function VendorConfigReader() {
  const [file, setFile] = useState<VendorConfigFile | null>(null);
  const [error, setError] = useState("");

  const readConfig = useCallback(
    async (path: string) => {
      setError("");
      setFile(null);
      try {
        const files = await api.readVendorConfigs([path]);
        setFile(files[0] ?? null);
      } catch (problem) {
        setError(problem instanceof Error ? problem.message : String(problem));
      }
    },
    []
  );

  return (
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

      {/* 白名单要在界面上说得出理由，而不是只留三个按钮让人猜。
          写成一句提示：为什么这里没有「自己填路径」的输入框。 */}
      <p className="field-hint">
        只读上面三个固定路径：后端强制白名单（路径必须落在 <code>~/.codex/</code>、
        <code>~/.gemini/</code>、<code>~/.claude/</code> 之下）。
        不加这一道，这条命令就成了「读取任意文件」的入口——配置文件里通常有 API Key。
      </p>

      {error && <EmptyState message="读不到这个配置文件。" hint={error} />}

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
  );
}
