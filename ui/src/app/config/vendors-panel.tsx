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
};

/** 平台筛选的取值。`"all"` 表示不过滤。 */
type ProviderFilter = AiProviderId | "all";

export function VendorsPanel({
  providerId,
  providers,
  onError
}: {
  /** 侧栏当前选中的平台。只用来给「新增」表单一个**默认值**。 */
  providerId: AiProviderId | "";
  /** 全部平台。厂商的下拉与筛选都从它来——名称与侧栏逐字一致。 */
  providers: AiProviderSummary[];
  onError: (message: string) => void;
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
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    }
  };

  const save = async () => {
    if (!draft) return;
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
        enabled: draft.enabled
      });
      setDraft(null);
      await load();
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
      enabled: true
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
          </div>

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
            <span className="row-meta">
              写入 CLI 配置前会自动备份到 <code>data/vendor-backups/</code>
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
                        enabled: vendor.enabled
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

      {/* 探活三条：测连接 / 查余额 / 读 CLI 实际读到的配置。
          它们与上面的编辑表单是**两件事**：表单改的是应用的记录，
          这里验证的是「那家供应商现在到底能不能用」。 */}
      <VendorProbePanel vendors={vendors} onError={onError} />
    </>
  );
}
