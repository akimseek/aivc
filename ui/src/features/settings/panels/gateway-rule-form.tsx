import { Badge, Button, Field, Select, TextInput, Toolbar, ToolbarSpacer } from "../../../components";
import type { AiProviderId, AiProviderSummary, ApiVendor, FailoverRuleScope } from "../../../types";
import { providerIds, providerLabel } from "./gateway-external-card";
import type { RuleDraft } from "./gateway-rule-model";

/**
 * 规则表单（demo 第 1255-1367 行）。
 *
 * # 纯受控
 *
 * 草稿、测试结果与三个动作全部由卡片（`gateway-rules-card.tsx`）传进来——
 * 这样「试」和「存」用的必然是同一份草稿，不存在两份状态分叉的可能。
 * 草稿模型、四条校验与载荷拼装在 `gateway-rule-model.ts`。
 */

export function RuleForm({
  draft,
  vendors,
  providers,
  testing,
  testResult,
  onChange,
  onSubmit,
  onCancel,
  onTest
}: {
  draft: RuleDraft;
  /** 全部供应商：作用域选到「按供应商」时用来出选项。 */
  vendors: ApiVendor[];
  /**
   * 全部平台：清单与平台名都从它派生（与端点清单同一份推导）。拿不到时退回
   * 本地穷举兜底表——名称见 `providerLabel`。
   */
  providers?: AiProviderSummary[];
  testing: boolean;
  testResult: { matched: boolean; matchedBy: "status" | "pattern" | "none" } | null;
  onChange: (next: RuleDraft) => void;
  onSubmit: () => void;
  onCancel: () => void;
  onTest: () => void;
}) {
  const patch = (part: Partial<RuleDraft>) => onChange({ ...draft, ...part });
  /** 选中平台下的供应商。换平台要同时清掉已选供应商，否则会存出一个张冠李戴的组合。 */
  const vendorOptions = vendors.filter((item) => item.providerId === draft.providerId);

  return (
    <div className="editor">
      <Toolbar>
        <span className="row-meta">{draft.id ? "编辑规则" : "新增规则"}</span>
        <ToolbarSpacer />
        <Button small variant="primary" disabled={!draft.pattern.trim()} onClick={onSubmit}>
          {draft.id ? "保存修改" : "创建"}
        </Button>
        <Button small onClick={onCancel}>
          取消
        </Button>
      </Toolbar>

      <div className="field-grid">
        <Field
          label="作用域"
          hint="全局规则对所有平台生效；按平台/按供应商只作用于池子里的那一部分。"
        >
          <Select
            value={draft.scope}
            aria-label="作用域"
            onChange={(event) => {
              const scope = event.target.value as FailoverRuleScope;
              // 切回全局时把目标一并清掉（留着会被后端判成「全局规则不能指定」）；
              // 从供应商档退回按平台档时同理清掉供应商。
              patch(
                scope === "global"
                  ? { scope, providerId: "", vendorId: "" }
                  : { scope, ...(scope === "provider" ? { vendorId: "" } : {}) }
              );
            }}
          >
            <option value="global">全局</option>
            <option value="provider">按平台</option>
            <option value="vendor">按供应商</option>
          </Select>
        </Field>

        {/*
          作用域级联：选平台之后才能选供应商。
          **必须能选**：后端要求 Provider 级与供应商级规则带上 providerId
          （供应商级还要 vendorId，见 `crates/aivc-core/src/gateway.rs:270-291`），
          缺了直接拒收——此前表单里没有这两项，那两档作用域根本存不下去。
        */}
        {draft.scope !== "global" && (
          <Field label="目标平台" hint="这条规则作用于哪一个平台。">
            <Select
              value={draft.providerId}
              aria-label="目标平台"
              onChange={(event) =>
                patch({ providerId: event.target.value as AiProviderId | "", vendorId: "" })
              }
            >
              <option value="">请选择平台</option>
              {/*
                清单从 `providers` 派生（见 `providerIds`）：后端多一个平台，
                这里自动多一个选项——此前是一份手写数组，漏一个平台不会有任何提示。
              */}
              {providerIds(providers).map((id) => (
                <option key={id} value={id}>
                  {providerLabel(providers, id)}
                </option>
              ))}
            </Select>
          </Field>
        )}

        {draft.scope === "vendor" && (
          <Field
            label="目标供应商"
            hint={
              draft.providerId === ""
                ? "先选平台，这里才会列出该平台下的供应商。"
                : vendorOptions.length === 0
                  ? "该平台下还没有供应商。"
                  : "只对选中的这一家生效。"
            }
          >
            <Select
              value={draft.vendorId}
              aria-label="目标供应商"
              onChange={(event) => patch({ vendorId: event.target.value })}
            >
              <option value="">请选择供应商</option>
              {vendorOptions.map((item) => (
                <option key={item.id} value={item.id}>
                  {item.name}
                </option>
              ))}
            </Select>
          </Field>
        )}

        <Field label="错误文本模式" hint="不区分大小写的包含匹配，不是正则。">
          <TextInput
            value={draft.pattern}
            placeholder="例如 insufficient_quota"
            onChange={(event) => patch({ pattern: event.target.value })}
          />
        </Field>
        <Field label="HTTP 状态码" hint="逗号分隔，例如 429,500。留空表示不按状态码判定。">
          <TextInput
            value={draft.statusCodes}
            placeholder="429,500"
            onChange={(event) => patch({ statusCodes: event.target.value })}
          />
        </Field>
      </div>

      {/*
        自定义返回：命中时**由网关自己应答**，而不是把上游的错误原样透传给客户端。
        两项必须同时配置（后端校验），因此单独成块，让它们看起来是一组。
      */}
      <div className="gateway-custom-response">
        <div className="gateway-custom-response-title">自定义拦截应答</div>
        <p className="field-hint">
          命中后网关直接返回下面这份应答，不再透传上游不可读的原始错误；
          <b>两项必须同时配置或同时留空</b>。
        </p>
        <div className="field-grid">
          <Field
            label="自定义返回状态码"
            hint="命中时返回给客户端的状态码，100–599。留空表示透传上游。"
          >
            <TextInput
              value={draft.customStatus}
              inputMode="numeric"
              placeholder="例如 503"
              onChange={(event) => patch({ customStatus: event.target.value })}
            />
          </Field>
          <Field label="自定义返回内容（JSON）" hint="与状态码必须同时填；不超过 32 KB。">
            <TextInput
              value={draft.customBody}
              placeholder='例如 {"error":"上游不可用，请稍后重试"}'
              onChange={(event) => patch({ customBody: event.target.value })}
            />
          </Field>
        </div>
      </div>

      <label className="gateway-rule-enabled">
        <input
          type="checkbox"
          checked={draft.enabled}
          onChange={(event) => patch({ enabled: event.target.checked })}
        />
        <span>立即启用此规则</span>
        {/* 停用不是删除：说清楚它的后果，否则用户不敢停。 */}
        <span className="field-hint">停用后规则保留在列表中，但路由时会跳过它。</span>
      </label>

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
              onChange={(event) => patch({ testStatus: event.target.value })}
            />
          </Field>
          <Field label="试：上游错误文本" hint="粘贴供应商返回的错误信息，可留空。">
            <TextInput
              value={draft.testMessage}
              placeholder="例如 credit insufficient balance"
              onChange={(event) => patch({ testMessage: event.target.value })}
            />
          </Field>
        </div>
        <Toolbar>
          <Button small disabled={testing} onClick={onTest}>
            {testing ? "测试中…" : "试一下"}
          </Button>
          {testResult && (
            // **说清是被哪一项命中的**：只说「命中」时用户不知道是哪一条生效了，
            // 而两条都写错时会以为都对。
            <Badge tone={testResult.matched ? "accent" : "neutral"}>
              {testResult.matchedBy === "status"
                ? "会命中：状态码匹配"
                : testResult.matchedBy === "pattern"
                  ? "会命中：错误文本包含"
                  : "不会命中"}
            </Badge>
          )}
        </Toolbar>
      </div>
    </div>
  );
}
