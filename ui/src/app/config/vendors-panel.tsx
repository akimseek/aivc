import { useCallback, useEffect, useState } from "react";

import {
  Button,
  Card,
  ChipGroup,
  EmptyState,
  Field,
  LoadingBlock,
  Select,
  TextInput,
  Toolbar,
  ToolbarSpacer
} from "../../components";
import * as api from "../../ipc/commands";
import type { AiProviderId, AiProviderSummary, ApiVendor } from "../../types";
import { VendorProbePanel } from "../vendor-probe-panel";

/**
 * 供应商管理。
 *
 * # 它管的是**所有厂商**的供应商，不是当前平台的
 *
 * 这一点此前错了：面板只列当前平台（侧栏选的那一个），而且新增时
 * **平台被写死成当前平台、界面上没有地方可改**。用户报的就是这个：
 * 「供应商没有模型厂商的选择」。
 *
 * 旧平台的形状是：列表带一个**平台筛选**（全部平台 / 各厂商），表格里有一列
 * **厂商**，表单里有一组**模型厂商**按钮可选。
 *
 * # 「模型厂商」为什么必须能改
 *
 * 供应商的 `providerId` 决定三件事：它进哪个候选池、网关按什么协议转发、
 * 以及 CLI 那边写进哪份配置。新增时按当前平台填一个默认值是对的（多数情况就是它），
 * 但**必须是可改的**——中转站常常一家同时提供多家模型，
 * 而用户当前正在看的是哪个平台与他此刻要加哪一家的供应商毫无关系。
 */

/** 供应商编辑草稿。数字字段用字符串存——输入框里「空」与「0」不是一回事。 */
type VendorDraft = {
  id: string;
  /** 这个供应商服务哪一家模型厂商。 */
  providerId: AiProviderId;
  name: string;
  apiKey: string;
  apiBaseUrl: string;
  sort: string;
  enabled: boolean;
  /**
   * 费率（每百万 Token，美元）。
   *
   * 后端 `VendorPricing { inputPerMillionUsd, outputPerMillionUsd }`。
   * 留空表示「未设置」——**与 0 不是一回事**：0 是免费，未设置是「不知道」。
   */
  inputPrice: string;
  outputPrice: string;
  /**
   * 高级 JSON：余额查询 / 模型查询配置。
   *
   * 后端是**结构化**的 `BalanceQueryConfig` / `ModelQueryConfig`（十几个字段、带枚举）。
   * 旧平台用两个 JSON 文本框来配，这里照做——为一个罕用的高级配置铺十几个输入控件，
   * 只会把常用路径埋掉。
   */
  balanceQuery: string;
  modelQuery: string;
};

/** 平台筛选的取值。`"all"` 表示不过滤。 */
type ProviderFilter = AiProviderId | "all";

/**
 * 解析一个「高级 JSON」输入框。空串 = 不设置。
 *
 * 两条提示语照旧平台：「JSON 格式无效」「必须是 JSON 对象」。
 * 后端要的是对象，早一步拦住能给出比后端反序列化错误更清楚的话。
 */
export function parseAdvancedJson(
  text: string,
  label: string
): { ok: true; value: unknown } | { ok: false; message: string } {
  const trimmed = text.trim();
  if (trimmed === "") return { ok: true, value: undefined };
  let parsed: unknown;
  try {
    parsed = JSON.parse(trimmed);
  } catch {
    return { ok: false, message: `${label}：JSON 格式无效` };
  }
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
    return { ok: false, message: `${label}：必须是 JSON 对象` };
  }
  return { ok: true, value: parsed };
}

/**
 * 解析费率。
 *
 * 空串 → `undefined`（不设置）；「0」→ `0`（免费）；非数字、负数、**超过两位小数**
 * → 报错，而不是悄悄变成 0——那会让成本估算凭空归零。
 *
 * # 为什么界面也要管「两位小数」
 *
 * 后端 `normalize_rate` 就是这么校验的（消息是「必须是非负数，且最多保留 2 位小数」）。
 * 界面先拦一道能立刻告诉用户错在哪，而不是等一次往返。
 *
 * # 这里的比较**必须带容差**
 *
 * 后端原来用的是精确浮点相等（`scaled.round() != scaled`），于是
 * `0.55 * 100 = 55.00000000000001` 被判成「超过两位小数」——**0.55 这样的常见价格
 * 直接被拒**。那个缺陷已经修掉了（改成 `1e-9` 容差）。界面这边从一开始就带容差，
 * 免得同一个坑踩两次。
 */
export function parsePrice(
  text: string,
  label: string
): { ok: true; value: number | undefined } | { ok: false; message: string } {
  const trimmed = text.trim();
  if (trimmed === "") return { ok: true, value: undefined };
  const value = Number(trimmed);
  if (!Number.isFinite(value) || value < 0) {
    return { ok: false, message: `${label}：必须是不小于 0 的数字` };
  }
  const scaled = value * 100;
  if (Math.abs(scaled - Math.round(scaled)) > 1e-9) {
    return { ok: false, message: `${label}：最多保留 2 位小数` };
  }
  return { ok: true, value };
}

/**
 * 回填费率：数字转成输入框里的字符串。未设置 → 空串（**不是 "0"**）。
 */
export function formatPrice(value: number | undefined): string {
  return value === undefined ? "" : String(value);
}

/**
 * 回填高级 JSON：对象转成格式化过的文本。
 *
 * 用 `JSON.stringify(…, 2)` 而不是单行：这两项是给人看的，
 * 缩进过的 JSON 才看得出结构。未设置 → 空串。
 */
export function formatJson(value: Record<string, unknown> | undefined): string {
  if (value === undefined || Object.keys(value).length === 0) return "";
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    // 循环引用之类：回填不了就留空，总比让表单炸掉好。
    return "";
  }
}

export function VendorsPanel({
  providerId,
  providers,
  onError,
  onNotify
}: {
  /** 侧栏当前选中的平台。只用来给「新增」表单一个**默认值**。 */
  providerId: AiProviderId | "";
  /** 全部平台。厂商的下拉与筛选都从它来——名称与侧栏逐字一致。 */
  providers: AiProviderSummary[];
  onError: (message: string) => void;
  /** 成功反馈。见 `ConfigOverlay` 同名字段的说明。 */
  onNotify?: (message: string) => void;
}) {
  const [vendors, setVendors] = useState<ApiVendor[]>([]);
  const [loading, setLoading] = useState(true);
  const [filter, setFilter] = useState<ProviderFilter>("all");
  /** 正在编辑的草稿；`id` 为空表示新建。`null` 表示表单未打开。 */
  const [draft, setDraft] = useState<VendorDraft | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      // 筛选交给后端：它按 `providerId` 过滤，不必把全部供应商拉回来再在前端丢。
      setVendors(await api.listVendors(filter === "all" ? undefined : filter));
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    } finally {
      setLoading(false);
    }
  }, [filter, onError]);

  useEffect(() => {
    void load();
  }, [load]);

  const toggle = async (vendor: ApiVendor) => {
    try {
      await api.setVendorEnabled(vendor.id, !vendor.enabled);
      await load();
      onNotify?.(vendor.enabled ? `已禁用供应商：${vendor.name}` : `已启用供应商：${vendor.name}`);
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    }
  };

  const save = async () => {
    if (!draft) return;
    // 三个高级字段先解析：**任何一处不合法就整体不提交**。
    // 提交一半再报错，用户不知道哪一项存进去了。
    const inputPrice = parsePrice(draft.inputPrice, "输入费率");
    if (!inputPrice.ok) {
      onError(inputPrice.message);
      return;
    }
    const outputPrice = parsePrice(draft.outputPrice, "输出费率");
    if (!outputPrice.ok) {
      onError(outputPrice.message);
      return;
    }
    const balanceQuery = parseAdvancedJson(draft.balanceQuery, "余额查询配置");
    if (!balanceQuery.ok) {
      onError(balanceQuery.message);
      return;
    }
    const modelQuery = parseAdvancedJson(draft.modelQuery, "模型查询配置");
    if (!modelQuery.ok) {
      onError(modelQuery.message);
      return;
    }
    /** 两个费率都没填时**整个 `pricing` 不传**：空对象在后端是「设置了但什么都没有」。 */
    const pricing =
      inputPrice.value === undefined && outputPrice.value === undefined
        ? undefined
        : {
            ...(inputPrice.value === undefined ? {} : { inputPerMillionUsd: inputPrice.value }),
            ...(outputPrice.value === undefined ? {} : { outputPerMillionUsd: outputPrice.value })
          };
    try {
      await api.saveVendor({
        ...(draft.id ? { id: draft.id } : {}),
        // **用草稿里的厂商**，不是外层的当前平台。这一行就是用户报的那个缺陷。
        providerId: draft.providerId,
        name: draft.name.trim(),
        // 留空即沿用旧值——后端归一化时这样处理，因此编辑时不必重输密钥。
        apiKey: draft.apiKey.trim(),
        apiBaseUrl: draft.apiBaseUrl.trim(),
        ...(draft.sort.trim() ? { sort: Number.parseInt(draft.sort, 10) || 0 } : {}),
        ...(pricing ? { pricing } : {}),
        ...(balanceQuery.value === undefined ? {} : { balanceQuery: balanceQuery.value }),
        ...(modelQuery.value === undefined ? {} : { modelQuery: modelQuery.value }),
        enabled: draft.enabled
      });
      setDraft(null);
      await load();
      onNotify?.(draft.id ? `供应商已保存：${draft.name.trim()}` : `已新增供应商：${draft.name.trim()}`);
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    }
  };

  /** 新草稿的厂商默认值：优先当前平台，没有就用第一个平台。 */
  const defaultProvider = (): AiProviderId =>
    (providerId || providers[0]?.id || "codex") as AiProviderId;

  const startNew = () =>
    setDraft({
      id: "",
      providerId: defaultProvider(),
      name: "",
      apiKey: "",
      apiBaseUrl: "",
      sort: String(vendors.length),
      enabled: true,
      inputPrice: "",
      outputPrice: "",
      balanceQuery: "",
      modelQuery: ""
    });

  /** 平台显示名。取后端给的 `label`（与侧栏逐字一致），查不到才退回编号。 */
  const labelOf = (id: string): string =>
    providers.find((item) => item.id === id)?.label ?? id;

  const enabledCount = vendors.filter((item) => item.enabled).length;

  return (
    <>
      <Card
        title="供应商"
        description="这里管的是**所有厂商**的供应商。网关按供应商所属的厂商把它们放进对应的候选池。"
        actions={
          <>
            <label className="field">
              <span className="field-label">平台</span>
              <Select
                value={filter}
                aria-label="按平台筛选供应商"
                onChange={(event) => setFilter(event.target.value as ProviderFilter)}
              >
                <option value="all">全部平台</option>
                {providers.map((item) => (
                  <option key={item.id} value={item.id}>
                    {item.label}
                  </option>
                ))}
              </Select>
            </label>
            <Button small variant="primary" onClick={startNew}>
              新增供应商
            </Button>
          </>
        }
      >
        <span className="row-meta">
          {loading ? "加载中…" : `${vendors.length} 个 · ${enabledCount} 个启用`}
        </span>
      </Card>

      {draft && (
        <div className="editor">
          <div className="field-grid">
            {/*
              **模型厂商**是一组按钮而不是下拉。
              选项只有四个、而且这是这张表单里最要紧的一个选择——下拉会把
              「我要加的是哪一家」藏在一次点击之后，而按钮让四家一直看得见。
              这与旧平台一致。
            */}
            <Field
              label="模型厂商"
              hint={
                draft.id
                  ? "改动会把这条供应商移到另一家的候选池。"
                  : "这个供应商服务哪一家模型；决定它进哪个候选池。"
              }
            >
              <ChipGroup
                value={draft.providerId}
                onChange={(value) =>
                  setDraft((current) =>
                    current ? { ...current, providerId: value as AiProviderId } : current
                  )
                }
                options={providers.map((item) => ({ value: item.id, label: item.label }))}
              />
            </Field>

            <Field label="名称">
              <TextInput
                value={draft.name}
                placeholder="例如 官方 v2"
                onChange={(event) =>
                  setDraft((current) =>
                    current ? { ...current, name: event.target.value } : current
                  )
                }
              />
            </Field>

            <Field label="API 地址">
              <TextInput
                value={draft.apiBaseUrl}
                placeholder="https://api.example.com/v1"
                onChange={(event) =>
                  setDraft((current) =>
                    current ? { ...current, apiBaseUrl: event.target.value } : current
                  )
                }
              />
            </Field>

            <Field label="API Key" hint={draft.id ? "留空表示沿用原密钥。" : undefined}>
              <TextInput
                value={draft.apiKey}
                placeholder={draft.id ? "（不修改）" : "sk-…"}
                onChange={(event) =>
                  setDraft((current) =>
                    current ? { ...current, apiKey: event.target.value } : current
                  )
                }
              />
            </Field>

            <Field label="排序" hint="同厂商内唯一，决定候选池顺序。">
              <TextInput
                value={draft.sort}
                inputMode="numeric"
                onChange={(event) =>
                  setDraft((current) =>
                    current ? { ...current, sort: event.target.value } : current
                  )
                }
              />
            </Field>

            {/*
              费率。后端 `VendorPricing` 两个字段都是可选的，因此留空 = 未设置。
              **与 0 不是一回事**：0 是免费，未设置是「不知道」，金额那一列显示 `-`。
            */}
            <Field label="输入费率" hint="每百万 Token 的美元价；留空表示未设置。">
              <TextInput
                value={draft.inputPrice}
                inputMode="decimal"
                placeholder="例如 0.55"
                onChange={(event) =>
                  setDraft((current) =>
                    current ? { ...current, inputPrice: event.target.value } : current
                  )
                }
              />
            </Field>

            <Field label="输出费率" hint="每百万 Token 的美元价；留空表示未设置。">
              <TextInput
                value={draft.outputPrice}
                inputMode="decimal"
                placeholder="例如 2.2"
                onChange={(event) =>
                  setDraft((current) =>
                    current ? { ...current, outputPrice: event.target.value } : current
                  )
                }
              />
            </Field>
          </div>

          {/*
            高级 JSON 单独一块、默认收起。旧的供应商对话框把这两项混在表单里，
            十几个字段的配置摊开之后，真正常用的「名称 / 地址 / Key」反而不好找。
          */}
          <details className="vendor-advanced">
            <summary>高级：余额查询 / 模型查询配置（JSON，可选）</summary>
            <p className="field-hint">
              多数中转商的余额与模型清单接口不是标准形状，靠这两项告诉程序怎么查。
              留空表示用默认行为。
            </p>
            <Field label="余额查询配置">
              <textarea
                className="control mono"
                rows={4}
                spellCheck={false}
                value={draft.balanceQuery}
                placeholder='{"endpoint":"/user/balance","method":"GET"}'
                onChange={(event) =>
                  setDraft((current) =>
                    current ? { ...current, balanceQuery: event.target.value } : current
                  )
                }
              />
            </Field>
            <Field label="模型查询配置">
              <textarea
                className="control mono"
                rows={4}
                spellCheck={false}
                value={draft.modelQuery}
                placeholder='{"endpoint":"/models","authMode":"bearer"}'
                onChange={(event) =>
                  setDraft((current) =>
                    current ? { ...current, modelQuery: event.target.value } : current
                  )
                }
              />
            </Field>
          </details>

          <Toolbar>
            <Button
              small
              variant="primary"
              disabled={!draft.name.trim() || !draft.apiBaseUrl.trim()}
              onClick={() => void save()}
            >
              {draft.id ? "保存修改" : "创建"}
            </Button>
            <Button small onClick={() => setDraft(null)}>
              取消
            </Button>
            <ToolbarSpacer />
            {/*
              **这里此前写的是「写入 CLI 配置前会自动备份到 data/vendor-backups/」——
              那是一句这个界面做不到的承诺。**

              实测：后端 `vendor:enable` 才会把供应商的配置模板写进用户的 CLI 配置文件
              （写前备份），而它要求「至少启用一个配置文件」；本界面既不设 `configs`，
              启用走的也是 `vendor:set-enabled`（只翻数据库里的标志位）。
              也就是说**写 CLI 配置这条路在本界面上根本走不到**，那句备份提示是空头支票。

              现在的做法：如实说明这里改的是什么、凭据是怎么到达 CLI 的。
              至于「要不要恢复旧平台的写配置模型」是**产品决定**，见
              `docs/rust-rewrite/38-legacy-parity-audit.md` 的 A4。
            */}
            <span className="row-meta">
              这里改的是**应用数据库里的记录**；终端启动时由网关注入凭据，不改动你的 CLI 配置文件。
            </span>
          </Toolbar>
        </div>
      )}

      {loading ? (
        <LoadingBlock label="正在读取供应商…" />
      ) : vendors.length === 0 ? (
        <EmptyState
          message={
            filter === "all"
              ? "还没有配置任何供应商。"
              : `${labelOf(filter)} 下还没有供应商。`
          }
          hint="供应商决定 CLI 走哪个 API 地址与密钥。"
          action={
            <Button onClick={startNew}>
              新增供应商
            </Button>
          }
        />
      ) : (
        <table className="data-table">
          <thead>
            <tr>
              {/* 厂商列非有不可：列表是跨平台的，不标出来同名供应商分不清。 */}
              <th>厂商</th>
              <th>名称</th>
              <th>API 地址</th>
              <th>排序</th>
              <th>状态</th>
              <th />
              <th />
              <th />
            </tr>
          </thead>
          <tbody>
            {vendors.map((vendor) => (
              <tr key={vendor.id}>
                <td>{labelOf(vendor.providerId)}</td>
                <td>{vendor.name}</td>
                {/* 地址可能很长，用等宽字体——截断会让同名中转商分不出来。 */}
                <td className="mono">{vendor.apiBaseUrl}</td>
                <td>{vendor.sort}</td>
                <td>
                  <span className={`tag ${vendor.enabled ? "ok" : "off"}`}>
                    {vendor.enabled ? "已启用" : "已禁用"}
                  </span>
                </td>
                <td>
                  <Button
                    small
                    onClick={() =>
                      setDraft({
                        id: vendor.id,
                        providerId: vendor.providerId,
                        name: vendor.name,
                        // **不回填密钥**：接口本来就不返回它，界面也不该假装有。
                        apiKey: "",
                        apiBaseUrl: vendor.apiBaseUrl,
                        sort: String(vendor.sort),
                        enabled: vendor.enabled,
                        // 费率与高级 JSON 要回填——不回填的话，用户点一次「保存修改」
                        // 就会把它们当成「没填」而清掉。
                        inputPrice: formatPrice(vendor.pricing?.inputPerMillionUsd),
                        outputPrice: formatPrice(vendor.pricing?.outputPerMillionUsd),
                        balanceQuery: formatJson(vendor.balanceQuery),
                        modelQuery: formatJson(vendor.modelQuery)
                      })
                    }
                  >
                    编辑
                  </Button>
                </td>
                <td>
                  <Button small onClick={() => void toggle(vendor)}>
                    {vendor.enabled ? "禁用" : "启用"}
                  </Button>
                </td>
                <td>
                  <Button
                    small
                    variant="danger"
                    onClick={() =>
                      void api
                        .deleteVendor(vendor.id)
                        .then(() => {
                          onNotify?.(`供应商已删除：${vendor.name}`);
                          return load();
                        })
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

      {/* 探活三条：测连接 / 查余额 / 读 CLI 实际读到的配置。
          它们与上面的编辑表单是**两件事**：表单改的是应用的记录，
          这里验证的是「那家供应商现在到底能不能用」。 */}
      <VendorProbePanel vendors={vendors} onError={onError} />
    </>
  );
}
