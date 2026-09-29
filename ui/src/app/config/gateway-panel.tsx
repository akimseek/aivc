import { useCallback, useEffect, useState } from "react";

import { Button, EmptyState, Field, Notice, Select, TextInput, Toolbar, ToolbarSpacer } from "../../components";
import * as api from "../../ipc/commands";
import { GatewayInsights } from "../gateway-insights";
import type { ExternalApiStatus, FailoverRule, FailoverRuleScope, GatewayPortStatus } from "../../types";

export function GatewayPanel({
  onError,
  onNotify
}: {
  onError: (message: string) => void;
  /** 成功反馈。见 `ConfigOverlay` 同名字段的说明。 */
  onNotify?: (message: string) => void;
}) {
  const [status, setStatus] = useState<GatewayPortStatus | null>(null);
  const [portDraft, setPortDraft] = useState("");
  /** 复制成功的一次性提示——复制是「做完没有任何反馈」的典型动作。 */
  const [copied, setCopied] = useState(false);

  const load = useCallback(async () => {
    try {
      const next = await api.getGatewayPort();
      setStatus(next);
      // **`configuredPort` 现在是后端真的会发的字段**。此前它永远是 `undefined`，
      // `String(undefined)` 让这个输入框里躺着字符串 `"undefined"`——
      // 用户点一下「保存端口」就会把它解析成 NaN。
      setPortDraft(String(next.configuredPort));
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    }
  }, [onError]);

  const copyAddress = useCallback(async () => {
    if (!status?.baseUrl) return;
    try {
      await api.copyText(status.baseUrl);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    }
  }, [status?.baseUrl, onError]);

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

      {/*
        平台 Base URL：**别的客户端该连哪里**。
        这是「把网关给外部用」这件事唯一要抄的值，因此必须有，而且要能一键复制。
        值由后端给（它读的就是注入给 CLI 的那个地址）——界面自己拼的话，
        端口为 0（自动分配）时只能拼出 `http://127.0.0.1:0`。
      */}
      <Field label="平台 Base URL">
        {status.baseUrl ? (
          <div className="gateway-address">
            <code className="mono" title={status.baseUrl}>
              {status.baseUrl}
            </code>
            <Button small onClick={() => void copyAddress()}>
              {copied ? "已复制" : "复制"}
            </Button>
          </div>
        ) : (
          <span className="row-meta">当前没有在监听。开启网关并重启应用后这里会出现地址。</span>
        )}
      </Field>
      {status.baseUrl && (
        // 端口是自动分配还是固定的，决定这个地址能不能长期用。
        // 不写清楚的话，用户可能拿一个「这次有效」的地址去配外部客户端，
        // 下次重启连不上，看起来像客户端坏了。
        //
        // 用普通提示段落而不是 `Notice`：那个组件要求给一个动作按钮，
        // 而这里没有可做的动作——为了用它而塞一个假按钮更糟。
        <p className="field-hint">
          {status.configuredPort === 0
            ? "端口由系统自动分配，每次启动都可能不同。要长期给别的客户端用，请在上面填一个固定端口。"
            : `端口已固定在 ${status.configuredPort}，重启后地址不变。`}
        </p>
      )}

      <Toolbar>
        <Button
          small
          onClick={() =>
            void api
              .setGatewayEnabled(!status.enabled)
              .then((next) => {
                setStatus(next);
                // 关网关会**立刻切断终端路由**（旧平台专门写了这句警告），
                // 因此这里必须说清是哪一种结果。
                onNotify?.(
                  next.enabled
                    ? "已启用本地网关。重启应用后开始监听。"
                    : "已关闭本地网关。使用网关的终端路由将立即停止。"
                );
              })
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
              .then((next) => {
                setStatus(next);
                // 端口改了但**不会立刻生效**（要重启才重绑），这句话不说出来
                // 用户会以为点了没反应。
                onNotify?.(
                  next.configuredPort === 0
                    ? "端口已保存：改为自动分配。重启应用后生效。"
                    : `端口已保存：${next.configuredPort}。重启应用后生效。`
                );
              })
              .catch((error: unknown) =>
                onError(error instanceof Error ? error.message : String(error))
              )
          }
        >
          保存端口
        </Button>
      </Toolbar>

      <GatewayInsights onError={onError} />
      <FailoverRulesSection onError={onError} onNotify={onNotify} />
      <ExternalApiSection onError={onError} onNotify={onNotify} />
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
export function FailoverRulesSection({
  onError,
  onNotify
}: {
  onError: (message: string) => void;
  onNotify?: (message: string) => void;
}) {
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
      onNotify?.(draft.id ? "故障规则已保存。" : "已新增故障规则。");
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
export function ExternalApiSection({
  onError,
  onNotify
}: {
  onError: (message: string) => void;
  onNotify?: (message: string) => void;
}) {
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
              .then((next) => {
                setStatus(next);
                onNotify?.(next.enabled ? "外部 API 已启用。" : "外部 API 已关闭。");
              })
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
                // 旧令牌**立即失效**，这句不说用户不会知道别的客户端已经断了。
                onNotify?.("已生成新令牌；旧令牌立即失效。");
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
