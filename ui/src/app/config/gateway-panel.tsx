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
  /** 成功反馈。见 `SettingsCenter` 同名字段的说明。 */
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
          /*
            没有地址时要说清**下一步做什么**，而不是只说「没有」。
            旧平台的原文是「本地 Gateway 正在监听后显示可用地址。」／
            「请先开启本地 Gateway」——用户看见「没有地址」时想知道的就是这个。
            按停在哪一步给不同的指引：关着就让他开，开着没监听就让他重启。
          */
          <span className="row-meta">
            {status.enabled
              ? "已启用但还没开始监听。重启应用后这里会出现可用地址。"
              : "尚未开启本地网关。请先在上面点「启用网关」，然后重启应用。"}
          </span>
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
      <ResilienceSection onError={onError} onNotify={onNotify} />
      <FailoverRulesSection onError={onError} onNotify={onNotify} />
      <ExternalApiSection onError={onError} onNotify={onNotify} />
    </>
  );
}

/**
 * 网关的失败转移与熔断阈值。
 *
 * # 这三项此前在界面上根本不存在
 *
 * 后端的三个 setter **一直是完整实现的**（边界校验与错误文案都在），
 * 只是从没登记进 `COMMANDS`——因此命令面里查不到、界面也就没法调。
 * 这一轮把它们接出来。
 *
 * # 三条说明文案照旧平台
 *
 * 这三个数字对使用者的含义并不直观（「异常切换阈值」尤其如此），
 * 旧平台为每一项配了一句解释。照抄它们，不要自己另写。
 *
 * # 取值范围与旧平台一致
 *
 * | 项 | 范围 | 旧平台的原话 |
 * | --- | --- | --- |
 * | 异常切换阈值 | 1–10 | `异常切换阈值必须是 1 到 10 之间的整数。` |
 * | 熔断次数 | 1–20 | `熔断次数必须是 1 到 20 之间的整数。` |
 * | 熔断持续时间 | 10–86400 秒 | `熔断持续时间必须是 10 到 86400 秒之间的整数。` |
 *
 * 界面**先校验一次**只是为了少一次往返与更早的提示；判定仍然以后端为准
 * （这里绝不自己写一套「大概对」的规则）。
 */
export function ResilienceSection({
  onError,
  onNotify
}: {
  onError: (message: string) => void;
  onNotify?: (message: string) => void;
}) {
  const [status, setStatus] = useState<GatewayPortStatus | null>(null);
  /** 三个输入的草稿。数字用字符串存——「空」与「0」在输入框里不是一回事。 */
  const [draft, setDraft] = useState({ failure: "", circuit: "", duration: "" });

  const load = useCallback(async () => {
    try {
      const next = await api.getGatewayPort();
      setStatus(next);
      setDraft({
        failure: String(next.failureThreshold ?? ""),
        circuit: String(next.circuitFailureThreshold ?? ""),
        duration: String(next.circuitDurationSeconds ?? "")
      });
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    }
  }, [onError]);

  useEffect(() => {
    void load();
  }, [load]);

  if (!status) return null;

  /**
   * 保存一项。
   *
   * `label` 是这一项的显示名，用于成功提示里点名改的是哪一个——
   * 三个数字长得很像，只说「已保存」用户不知道哪一格生效了。
   */
  const save = async (
    label: string,
    text: string,
    range: { min: number; max: number; message: string },
    call: (value: number) => Promise<GatewayPortStatus>
  ) => {
    const value = Number(text.trim());
    if (!Number.isInteger(value) || value < range.min || value > range.max) {
      onError(range.message);
      return;
    }
    try {
      // 三个 setter 返回的都是**完整状态**，因此直接替换即可。
      setStatus(await call(value));
      onNotify?.(`${label}已保存：${value}`);
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    }
  };

  return (
    <>
      <h3 className="section-title">失败转移与熔断</h3>
      <div className="field-grid">
        <Field
          label="异常切换阈值（次）"
          hint="同一供应商连续请求失败达到该次数后，才切换到候选池中的下一个供应商。"
        >
          <div className="gateway-address">
            <TextInput
              value={draft.failure}
              inputMode="numeric"
              aria-label="异常切换阈值"
              onChange={(event) => setDraft({ ...draft, failure: event.target.value })}
            />
            <Button
              small
              onClick={() =>
                void save(
                  "异常切换阈值",
                  draft.failure,
                  { min: 1, max: 10, message: "异常切换阈值必须是 1 到 10 之间的整数。" },
                  api.setGatewayFailureThreshold
                )
              }
            >
              保存
            </Button>
          </div>
        </Field>

        <Field
          label="熔断次数"
          hint="供应商在该时间窗口内累计失败达到次数后进入熔断状态。"
        >
          <div className="gateway-address">
            <TextInput
              value={draft.circuit}
              inputMode="numeric"
              aria-label="熔断次数"
              onChange={(event) => setDraft({ ...draft, circuit: event.target.value })}
            />
            <Button
              small
              onClick={() =>
                void save(
                  "熔断次数",
                  draft.circuit,
                  { min: 1, max: 20, message: "熔断次数必须是 1 到 20 之间的整数。" },
                  api.setGatewayCircuitFailureThreshold
                )
              }
            >
              保存
            </Button>
          </div>
        </Field>

        <Field
          label="熔断持续时间（秒）"
          hint="熔断期间不会选择该供应商；到期后进入半开状态并允许一次试探请求。"
        >
          <div className="gateway-address">
            <TextInput
              value={draft.duration}
              inputMode="numeric"
              aria-label="熔断持续时间"
              onChange={(event) => setDraft({ ...draft, duration: event.target.value })}
            />
            <Button
              small
              onClick={() =>
                void save(
                  "熔断持续时间",
                  draft.duration,
                  {
                    min: 10,
                    max: 86400,
                    message: "熔断持续时间必须是 10 到 86400 秒之间的整数。"
                  },
                  api.setGatewayCircuitDurationSeconds
                )
              }
            >
              保存
            </Button>
          </div>
        </Field>
      </div>
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
  const [testing, setTesting] = useState(false);
  /**
   * 上一次试规则的结果。
   *
   * `null` = 还没试过（与「试了没命中」是两件事，因此不能用一个布尔量表达）。
   */
  const [testResult, setTestResult] = useState<
    { matched: boolean; matchedBy: "status" | "pattern" | "none" } | null
  >(null);

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
    // 自定义返回先解析：任何一处不合法就整体不提交——提交一半再报错，
    // 用户不知道哪一项存进去了。
    const custom = parseCustomResponse(draft.customStatus, draft.customBody);
    if (!custom.ok) {
      onError(custom.message);
      return;
    }
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
        ...(custom.status === undefined ? {} : { customResponseStatus: custom.status }),
        ...(custom.body === undefined ? {} : { customResponseBody: custom.body }),
        enabled: draft.enabled
      });
      setDraft(null);
      await load();
      onNotify?.(draft.id ? "故障规则已保存。" : "已新增故障规则。");
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    }
  };

  /** 试当前这条草稿会不会命中。**不落库**，后端也只做纯计算。 */
  const test = async () => {
    if (!draft) return;
    const custom = parseCustomResponse(draft.customStatus, draft.customBody);
    if (!custom.ok) {
      onError(custom.message);
      return;
    }
    setTesting(true);
    try {
      const result = await api.testFailoverRule({
        rule: {
          scope: draft.scope,
          pattern: draft.pattern.trim(),
          ...(draft.statusCodes.trim()
            ? {
                statusCodes: draft.statusCodes
                  .split(",")
                  .map((part) => Number.parseInt(part.trim(), 10))
                  .filter((value) => Number.isInteger(value) && value > 0)
              }
            : {}),
          ...(custom.status === undefined ? {} : { customResponseStatus: custom.status }),
          ...(custom.body === undefined ? {} : { customResponseBody: custom.body })
        },
        ...(draft.testStatus.trim()
          ? { statusCode: Number.parseInt(draft.testStatus.trim(), 10) }
          : {}),
        ...(draft.testMessage.trim() ? { message: draft.testMessage.trim() } : {})
      });
      setTestResult(result);
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    } finally {
      setTesting(false);
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
          onClick={() =>
            setDraft({
              id: "",
              scope: "global",
              pattern: "",
              statusCodes: "",
              enabled: true,
              customStatus: "",
              customBody: "",
              // 试规则的输入给一个**能立刻命中**的默认值：用户点「试一下」就想看到
              // 反馈，默认空着会一律得到「没命中」，看起来像功能坏了。
              testStatus: "429",
              testMessage: ""
            })
          }
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
            {/*
              自定义返回：命中时**由网关自己应答**，而不是把上游的错误原样透传给客户端。
              两项必须同时配置（后端校验），因此中间画一条线，让它们看起来是一组。
            */}
            <Field
              label="自定义返回状态码"
              hint="命中时返回给客户端的状态码，100–599。留空表示透传上游。"
            >
              <TextInput
                value={draft.customStatus}
                inputMode="numeric"
                placeholder="例如 503"
                onChange={(event) => setDraft({ ...draft, customStatus: event.target.value })}
              />
            </Field>
            <Field label="自定义返回内容（JSON）" hint="与状态码必须同时填；不超过 32 KB。">
              <TextInput
                value={draft.customBody}
                placeholder='例如 {"error":"上游不可用，请稍后重试"}'
                onChange={(event) => setDraft({ ...draft, customBody: event.target.value })}
              />
            </Field>
          </div>

          {/*
            试规则。
            
            匹配是「状态码 **或** 错误文本」，只有规则引擎说了算——因此这件事**必须**
            问后端（`gateway:test-failover-rule`），不能在前端重写一遍匹配逻辑。
          */}
          <div className="rule-test">
            <div className="field-grid">
              <Field label="试：上游状态码" hint="模拟上游返回的状态码，可留空。">
                <TextInput
                  value={draft.testStatus}
                  inputMode="numeric"
                  placeholder="例如 429"
                  onChange={(event) => setDraft({ ...draft, testStatus: event.target.value })}
                />
              </Field>
              <Field label="试：上游错误文本" hint="粘贴供应商返回的错误信息，可留空。">
                <TextInput
                  value={draft.testMessage}
                  placeholder="例如 credit insufficient balance"
                  onChange={(event) => setDraft({ ...draft, testMessage: event.target.value })}
                />
              </Field>
            </div>
            <Toolbar>
              <Button small disabled={testing} onClick={() => void test()}>
                {testing ? "测试中…" : "试一下"}
              </Button>
              {testResult && (
                // **说清是被哪一项命中的**：只说「命中」时用户不知道是哪一条生效了，
                // 而两条都写错时会以为都对。
                <span className={testResult.matched ? "tag ok" : "tag off"}>
                  {testResult.matchedBy === "status"
                    ? "会命中：状态码匹配"
                    : testResult.matchedBy === "pattern"
                      ? "会命中：错误文本包含"
                      : "不会命中"}
                </span>
              )}
            </Toolbar>
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
              {/*
                「自定义返回」单列一栏：它是**命中之后客户端实际看到什么**，
                与「匹配什么」是两个问题。配置了却看不出来，用户会怀疑自己没保存成功。
              */}
              <th>自定义返回</th>
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
                <td className="mono" title={rule.customResponseBody}>
                  {rule.customResponseStatus === undefined
                    ? // 没配就是「透传上游」——用 `-` 表示「未设置」，
                      // 而不是留空（留空看不出是没配还是没加载出来）。
                      "透传上游"
                    : `${rule.customResponseStatus}${rule.customResponseBody ? " · JSON" : ""}`}
                </td>
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
                        enabled: rule.enabled,
                        // **必须回填**：不回填的话，点一次「保存修改」就会把用户配好的
                        // 自定义返回当成「没填」而清掉（供应商那三项踩过同一个坑）。
                        customStatus:
                          rule.customResponseStatus === undefined
                            ? ""
                            : String(rule.customResponseStatus),
                        customBody: rule.customResponseBody ?? "",
                        testStatus: (rule.statusCodes?.[0] ?? 429).toString(),
                        testMessage: ""
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
  /**
   * 命中时网关自己返回给客户端的**状态码与正文**（而不是把上游的错原样透传）。
   *
   * 两项**必须同时配置**（后端校验：「自定义返回状态码和内容必须同时配置。」），
   * 因此这里存成字符串、提交时一起判断。
   */
  customStatus: string;
  customBody: string;
  /** 试规则用的模拟输入。**不是规则的一部分**，因此不参与提交。 */
  testStatus: string;
  testMessage: string;
};

/**
 * 解析「自定义返回」两项。
 *
 * 四条校验照旧平台：
 * - 状态码必须是 100–599 的整数；
 * - 正文必须是合法 JSON；
 * - 正文不超过 32 KB；
 * - **状态码与正文必须同时配置**。
 *
 * 放在提交之前统一判，返回第一处错误——一次只报一条比堆一串更难读。
 * 后端也有同样的校验（这里是提前告知，不是替代）。
 */
export function parseCustomResponse(
  status: string,
  body: string
): { ok: true; status?: number; body?: string } | { ok: false; message: string } {
  const statusText = status.trim();
  const bodyText = body.trim();
  if (statusText === "" && bodyText === "") return { ok: true };
  if (statusText === "" || bodyText === "") {
    return { ok: false, message: "自定义返回状态码和内容必须同时配置。" };
  }
  const code = Number(statusText);
  if (!Number.isInteger(code) || code < 100 || code > 599) {
    return { ok: false, message: "自定义返回状态码需为 100 到 599 之间的整数。" };
  }
  // 32 KB 按**字节**算（UTF-8）：中文字符一个占三字节，按字符数算会放过三倍的量。
  const size = new TextEncoder().encode(bodyText).length;
  if (size > 32 * 1024) {
    return { ok: false, message: "自定义返回内容不能超过 32 KB。" };
  }
  try {
    JSON.parse(bodyText);
  } catch {
    return { ok: false, message: "自定义返回内容必须是合法 JSON。" };
  }
  return { ok: true, status: code, body: bodyText };
}

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
          {/* 只显示「是否已设置」，**绝不回显令牌本身**——后端只存哈希。 */}
          <span className="value">{status.hasToken ? "已设置" : "未设置"}</span>
        </Field>
      </div>
      {/*
        说清当前这一步该做什么（旧平台原文：「尚未生成访问令牌。」／
        「已配置；轮换后旧令牌立即失效。」）。外部 API 开着却没有令牌是**半配置状态**：
        请求会全部被拒，而界面上只看得出「开着」。
      */}
      <p className="field-hint">
        {!status.hasToken
          ? "尚未生成访问令牌。外部客户端现在连不上——请先点「轮换令牌」生成一个。"
          : status.enabled
            ? "已配置；轮换后旧令牌立即失效，用它的客户端需要同步更新。"
            : "已配置令牌，但外部 API 处于关闭状态。"}
      </p>
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
