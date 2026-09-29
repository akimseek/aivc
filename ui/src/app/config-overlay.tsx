import { useCallback, useEffect, useState } from "react";

import { Button, ChipGroup, EmptyState, Field, Notice, Select, TextInput, Toolbar, ToolbarSpacer } from "../components";
import * as api from "../ipc/commands";
import { GatewayInsights } from "./gateway-insights";
import { GatewayFailurePanel } from "./gateway-failure-panel";
import { GatewayLogPanel } from "./gateway-log-panel";
import { GatewayUsagePanel } from "./gateway-usage-panel";
import { VendorsPanel } from "./config/vendors-panel";
import type {
  AiProviderId,
  AiProviderSummary,
  AiTarget,
  CliEnvironmentStatus,
  ExternalApiStatus,
  FailoverRule,
  FailoverRuleScope,
  GatewayPortStatus,
  SkillEntry,
  WorkspacePreset
} from "../types";

/**
 * 配置覆盖层。
 *
 * # 为什么是覆盖层而不是侧栏的一个区域
 *
 * 这四类东西（供应商 / 技能 / 网关 / 环境）**都不是「我正在看的会话」**，把它们放进
 * 侧栏会和会话列表抢同一块空间，而且每切一次都要重新布局。旧平台也是这个形状。
 *
 * 覆盖层还带来一个好处：它是**临时**的——关掉就回到原来的视图，不需要「返回」。
 */

/**
 * 配置面的分页。导出给命令面板用——它要能直接打开某一页。
 *
 * 分成「网关」与「用量 / 异常 / 日志」三页而不是全塞进「网关」一页：
 *
 * - 「网关」是**设置**（开关、端口、故障规则、外部接入），改的是行为；
 * - 另外三页是**观测**（花在哪、为什么失败、有哪些日志），只看不改。
 *
 * 混在一页里的后果是实测过的：`config-overlay.tsx` 一度到 1380 行，
 * 打开「网关」要同时渲染设置表单与三张只读表格，滚动条拉到一半就找不到开关了。
 */
export type ConfigTab =
  | "vendors"
  | "skills"
  | "gateway"
  | "usage"
  | "failures"
  | "logs"
  | "environment"
  | "presets"
  | "diagnostics";


export function ConfigOverlay({
  providerId,
  providers,
  targetId,
  targets,
  initialTab = "vendors",
  onClose,
  onError
}: {
  providerId: AiProviderId | "";
  /**
   * 全部平台。
   *
   * 供应商页要它——那里管的是**所有厂商**的供应商，厂商的下拉与筛选都从这里取，
   * 因此显示名与侧栏逐字一致。缺了它那一页只能拿编号当名字。
   */
  providers: AiProviderSummary[];
  targetId: string;
  /** 全部目标：诊断页要把每个目标的状态列出来。 */
  targets: AiTarget[];
  initialTab?: ConfigTab;
  onClose: () => void;
  onError: (message: string) => void;
}) {
  const [tab, setTab] = useState<ConfigTab>(initialTab);

  return (
    <div className="overlay" role="dialog" aria-modal="true" aria-label="配置">
      <div className="overlay-panel">
        <header className="overlay-head">
          <h2>配置</h2>
          <ToolbarSpacer />
          <Button small onClick={onClose}>
            关闭
          </Button>
        </header>

        <div className="overlay-tabs">
          <ChipGroup
            value={tab}
            onChange={(value) => setTab(value as ConfigTab)}
            options={[
              { value: "vendors", label: "供应商" },
              { value: "skills", label: "技能" },
              { value: "gateway", label: "网关" },
              { value: "usage", label: "用量" },
              { value: "failures", label: "异常" },
              { value: "logs", label: "日志" },
              { value: "environment", label: "环境" },
              { value: "presets", label: "工作区预设" },
              { value: "diagnostics", label: "诊断" }
            ]}
          />
        </div>

        <div className="overlay-body">
          {tab === "vendors" && (
            <VendorsPanel providerId={providerId} providers={providers} onError={onError} />
          )}
          {tab === "skills" && <SkillsPanel targetId={targetId} onError={onError} />}
          {tab === "gateway" && <GatewayPanel onError={onError} />}
          {/* 观测三页：看，不改。设置仍然在「网关」那一页。 */}
          {tab === "usage" && <GatewayUsagePanel onError={onError} />}
          {tab === "failures" && (
            <GatewayFailurePanel providerId={providerId} onError={onError} />
          )}
          {tab === "logs" && <GatewayLogPanel onError={onError} />}
          {tab === "environment" && (
            <EnvironmentPanel providerId={providerId} targetId={targetId} onError={onError} />
          )}
          {tab === "presets" && <PresetsPanel onError={onError} />}
          {tab === "diagnostics" && (
            <DiagnosticsPanel targets={targets} selectedTargetId={targetId} />
          )}
        </div>
      </div>
    </div>
  );
}

/* ── 工作区预设 ──────────────────────────────────────── */

/**
 * 工作区预设：把「用哪个目录、带什么提示词、附加什么 CLI 参数」存下来复用。
 *
 * 去重键是 `cwd` + 目标类型——同一个目录配两次没有意义，后端会替换旧的那条。
 */
function PresetsPanel({ onError }: { onError: (message: string) => void }) {
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
    <>
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
      <Toolbar>
        <Button small disabled={!cwd.trim()} onClick={() => void save()}>
          新增预设
        </Button>
      </Toolbar>

      {presets.length === 0 && <EmptyState message="还没有工作区预设。" />}
      {presets.map((preset) => (
        <div key={preset.id} className="list-row">
          <div className="list-row-main">
            <span className="list-row-title">{preset.name}</span>
            <span className="list-row-sub">{preset.cwd}</span>
          </div>
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
    </>
  );
}

/* ── 诊断 ────────────────────────────────────────────── */

/**
 * 诊断：把「每一道门的结果」集中到一处。
 *
 * 这一页存在的理由很具体：**用户遇到问题时，界面上通常只有一句结论**，
 * 而真正有用的是「哪些目标可用、各自为什么不可用」。这些信息本来就有
 * （目标自带 `detail`），只是一直没有一个地方把**全部目标**并排列出来。
 */
function DiagnosticsPanel({
  targets,
  selectedTargetId
}: {
  targets: AiTarget[];
  selectedTargetId: string;
}) {
  if (targets.length === 0) {
    return <EmptyState message="还没有探测到任何目标。" hint="先在上方选择平台。" />;
  }
  return (
    <>
      <div className="row-meta">
        共 {targets.length} 个目标 · {targets.filter((target) => target.available).length} 个可用
      </div>
      <table className="data-table">
        <thead>
          <tr>
            <th>目标</th>
            <th>类型</th>
            <th>状态</th>
            <th>探测结果</th>
          </tr>
        </thead>
        <tbody>
          {targets.map((target) => (
            <tr key={target.id}>
              <td>
                {target.label}
                {target.id === selectedTargetId && <span className="tag ok">当前</span>}
              </td>
              <td>{target.kind === "wsl" ? `WSL · ${target.distro ?? "-"}` : "本机"}</td>
              <td>
                <span className={`tag ${target.available ? "ok" : "off"}`}>
                  {target.available ? "可用" : "不可用"}
                </span>
              </td>
              {/* `detail` 是后端给出的四态说明，**原样显示**——它已经是最好的解释。 */}
              <td className="mono">{target.detail ?? "-"}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}

/* ── 技能 ────────────────────────────────────────────── */

function SkillsPanel({
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

/* ── 网关 ────────────────────────────────────────────── */

function GatewayPanel({ onError }: { onError: (message: string) => void }) {
  const [status, setStatus] = useState<GatewayPortStatus | null>(null);
  const [portDraft, setPortDraft] = useState("");

  const load = useCallback(async () => {
    try {
      const next = await api.getGatewayPort();
      setStatus(next);
      setPortDraft(String(next.configuredPort));
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    }
  }, [onError]);

  useEffect(() => {
    void load();
  }, [load]);

  if (!status) return <EmptyState message="读取网关状态…" />;

  return (
    <>
      <div className="field-grid">
        <Field label="状态">
          <span className={`tag ${status.enabled ? "ok" : "off"}`}>
            {status.enabled ? "已启用" : "已关闭"}
          </span>
        </Field>
        <Field label="当前端口">
          <span className="value">
            {status.activePort > 0 ? status.activePort : "未启动"}
          </span>
        </Field>
        <Field
          label="配置端口"
          hint="填 0 表示自动分配。改完需要重启应用才会生效。"
        >
          <TextInput
            value={portDraft}
            inputMode="numeric"
            onChange={(event) => setPortDraft(event.target.value)}
          />
        </Field>
      </div>

      <Toolbar>
        <Button
          small
          onClick={() =>
            void api
              .setGatewayEnabled(!status.enabled)
              .then(setStatus)
              .catch((error: unknown) =>
                onError(error instanceof Error ? error.message : String(error))
              )
          }
        >
          {status.enabled ? "关闭网关" : "启用网关"}
        </Button>
        <Button
          small
          onClick={() =>
            void api
              .setGatewayPort(Number.parseInt(portDraft, 10) || 0)
              .then(setStatus)
              .catch((error: unknown) =>
                onError(error instanceof Error ? error.message : String(error))
              )
          }
        >
          保存端口
        </Button>
      </Toolbar>

      <GatewayInsights onError={onError} />
      <FailoverRulesSection onError={onError} />
      <ExternalApiSection onError={onError} />
    </>
  );
}

/**
 * 故障识别规则。
 *
 * # 一个必须写进界面的语义
 *
 * `pattern` 是**不区分大小写的包含匹配，不是正则**。用户几乎一定会先按正则去写，
 * 因此提示必须显眼——写错正则不会报错，只会静默地匹配不到。
 */
function FailoverRulesSection({ onError }: { onError: (message: string) => void }) {
  const [rules, setRules] = useState<FailoverRule[]>([]);
  const [draft, setDraft] = useState<RuleDraft | null>(null);

  const load = useCallback(async () => {
    try {
      setRules(await api.listFailoverRules());
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    }
  }, [onError]);

  useEffect(() => {
    void load();
  }, [load]);

  const save = async () => {
    if (!draft || !draft.pattern.trim()) return;
    try {
      await api.saveFailoverRule({
        ...(draft.id ? { id: draft.id } : {}),
        scope: draft.scope,
        pattern: draft.pattern.trim(),
        // 状态码用逗号分隔；空表示「不按状态码判定」，因此**不传**这个字段，
        // 而不是传空数组——两者在后端不是同一件事。
        ...(draft.statusCodes.trim()
          ? {
              statusCodes: draft.statusCodes
                .split(",")
                .map((part) => Number.parseInt(part.trim(), 10))
                .filter((value) => Number.isInteger(value) && value > 0)
            }
          : {}),
        enabled: draft.enabled
      });
      setDraft(null);
      await load();
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    }
  };

  return (
    <>
      <h3 className="section-title">失败转移规则</h3>
      <Toolbar>
        <span className="row-meta">
          {rules.length} 条 · {rules.filter((rule) => rule.enabled).length} 条启用
        </span>
        <ToolbarSpacer />
        <Button
          small
          onClick={() => setDraft({ id: "", scope: "global", pattern: "", statusCodes: "", enabled: true })}
        >
          新增规则
        </Button>
      </Toolbar>

      {draft && (
        <div className="editor">
          <div className="field-grid">
            <Field label="作用域">
              <Select
                value={draft.scope}
                onChange={(event) =>
                  setDraft({ ...draft, scope: event.target.value as FailoverRuleScope })
                }
              >
                <option value="global">全局</option>
                <option value="provider">按平台</option>
                <option value="vendor">按供应商</option>
              </Select>
            </Field>
            <Field
              label="错误文本模式"
              hint="不区分大小写的包含匹配，不是正则。"
            >
              <TextInput
                value={draft.pattern}
                placeholder="例如 insufficient_quota"
                onChange={(event) => setDraft({ ...draft, pattern: event.target.value })}
              />
            </Field>
            <Field label="HTTP 状态码" hint="逗号分隔，例如 429,500。留空表示不按状态码判定。">
              <TextInput
                value={draft.statusCodes}
                placeholder="429,500"
                onChange={(event) => setDraft({ ...draft, statusCodes: event.target.value })}
              />
            </Field>
          </div>
          <Toolbar>
            <Button
              small
              variant="primary"
              disabled={!draft.pattern.trim()}
              onClick={() => void save()}
            >
              {draft.id ? "保存修改" : "创建"}
            </Button>
            <Button small onClick={() => setDraft(null)}>
              取消
            </Button>
          </Toolbar>
        </div>
      )}

      {rules.length === 0 ? (
        <div className="group-empty">还没有规则。没有规则时网关不会切换供应商。</div>
      ) : (
        <table className="data-table">
          <thead>
            <tr>
              <th>作用域</th>
              <th>匹配</th>
              <th>状态码</th>
              <th>状态</th>
              <th />
              <th />
            </tr>
          </thead>
          <tbody>
            {rules.map((rule) => (
              <tr key={rule.id}>
                <td>{scopeLabel(rule.scope)}</td>
                <td className="mono">{rule.pattern}</td>
                <td>{rule.statusCodes?.join(", ") ?? "-"}</td>
                <td>
                  <span className={`tag ${rule.enabled ? "ok" : "off"}`}>
                    {rule.enabled ? "已启用" : "已停用"}
                  </span>
                </td>
                <td>
                  <Button
                    small
                    onClick={() =>
                      setDraft({
                        id: rule.id,
                        scope: rule.scope,
                        pattern: rule.pattern,
                        statusCodes: rule.statusCodes?.join(",") ?? "",
                        enabled: rule.enabled
                      })
                    }
                  >
                    编辑
                  </Button>
                </td>
                <td>
                  <Button
                    small
                    onClick={() =>
                      void api
                        .setFailoverRuleEnabled(rule.id, !rule.enabled)
                        .then(load)
                        .catch((error: unknown) =>
                          onError(error instanceof Error ? error.message : String(error))
                        )
                    }
                  >
                    {rule.enabled ? "停用" : "启用"}
                  </Button>
                  <Button
                    small
                    variant="danger"
                    onClick={() =>
                      void api
                        .deleteFailoverRule(rule.id)
                        .then(load)
                        .catch((error: unknown) =>
                          onError(error instanceof Error ? error.message : String(error))
                        )
                    }
                  >
                    删除
                  </Button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </>
  );
}

function scopeLabel(scope: FailoverRuleScope): string {
  if (scope === "provider") return "按平台";
  if (scope === "vendor") return "按供应商";
  return "全局";
}

/** 规则编辑草稿。状态码用字符串存——「空」与「空数组」在后端不是同一件事。 */
type RuleDraft = {
  id: string;
  scope: FailoverRuleScope;
  pattern: string;
  statusCodes: string;
  enabled: boolean;
};

/**
 * 外部 API。
 *
 * 令牌**只显示这一次**：后端只存哈希，之后再也取不回来。因此轮换后必须让用户
 * 当场复制走，并明确告诉他这一点。
 */
function ExternalApiSection({ onError }: { onError: (message: string) => void }) {
  const [status, setStatus] = useState<ExternalApiStatus | null>(null);
  const [revealedToken, setRevealedToken] = useState("");

  const load = useCallback(async () => {
    try {
      setStatus(await api.getExternalApiStatus());
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    }
  }, [onError]);

  useEffect(() => {
    void load();
  }, [load]);

  if (!status) return null;

  return (
    <>
      <h3 className="section-title">外部 API</h3>
      <div className="field-grid">
        <Field label="状态">
          <span className={`tag ${status.enabled ? "ok" : "off"}`}>
            {status.enabled ? "已启用" : "已关闭"}
          </span>
        </Field>
        <Field label="令牌">
          <span className="value">{status.hasToken ? "已设置" : "未设置"}</span>
        </Field>
      </div>
      <Toolbar>
        <Button
          small
          onClick={() =>
            void api
              .setExternalApiEnabled(!status.enabled)
              .then(setStatus)
              .catch((error: unknown) =>
                onError(error instanceof Error ? error.message : String(error))
              )
          }
        >
          {status.enabled ? "关闭外部 API" : "启用外部 API"}
        </Button>
        <Button
          small
          onClick={() =>
            void api
              .rotateExternalApiToken()
              .then((result) => {
                // 令牌只在这里出现一次，界面必须留着它让用户复制。
                setRevealedToken(result.token);
                void load();
              })
              .catch((error: unknown) =>
                onError(error instanceof Error ? error.message : String(error))
              )
          }
        >
          轮换令牌
        </Button>
      </Toolbar>

      {revealedToken && (
        <Notice
          message={
            <span>
              新令牌：<code>{revealedToken}</code>
              <br />
              关闭后<b>再也看不到</b>（后端只存哈希），请现在复制走。
            </span>
          }
          actions={
            <>
              <Button small variant="primary" onClick={() => void api.copyText(revealedToken)}>
                复制令牌
              </Button>
              <Button small onClick={() => setRevealedToken("")}>
                我已保存
              </Button>
            </>
          }
        />
      )}
    </>
  );
}

/* ── 环境 ────────────────────────────────────────────── */

function EnvironmentPanel({
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
