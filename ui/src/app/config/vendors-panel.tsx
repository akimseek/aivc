import { useCallback, useEffect, useState } from "react";

import {
  Badge,
  Button,
  ChipGroup,
  EmptyState,
  Field,
  Icon,
  LoadingBlock,
  TextInput,
  Toolbar,
  ToolbarSpacer,
  useToast
} from "../../components";
import { PanelHeader } from "../../features/settings/panel-header";
import * as api from "../../ipc/commands";
import type {
  AiProviderId,
  AiProviderSummary,
  ApiVendor,
  GatewayVendorHealth,
  VendorBalance
} from "../../types";
import { formatWhen } from "../session-display";
import { VendorConfigReader, VendorModelProbe, balanceText } from "../vendor-probe-panel";

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
 *
 * # 批次二重排后的信息架构
 *
 * 主表在上、工具与调试信息在下：面板头四个按钮（查余额 / 测连接 / 读本地配置 /
 * 新增供应商），下面是厂商过滤 chips 与主表；「测连接」与「读本地配置」按需在
 * 表格下方展开卡片，不再各占一张常驻卡片——它们是偶尔用一次的动作，
 * 常驻会把主表挤到屏幕外。余额则**回到它属于的地方**：供应商自己那一行。
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

/** 厂商过滤的取值。`"all"` 表示不过滤。 */
type ProviderFilter = AiProviderId | "all";

/**
 * 一行的余额状态机。
 *
 * | 状态 | 显示 |
 * | --- | --- |
 * | 没查过 | 未获取 |
 * | 正在查 | 查询中… |
 * | 查到了 | `balanceText` 的结果（「未提供」表示上游不给余额，**不是 0**） |
 * | 查询失败 | 上一次成功的数字 +（已过期）／没有数字则「查询失败」 |
 *
 * 「失败但留着上一次的数字」是旧平台的做法：抹掉它等于把「查不到」说成「没有」，
 * 而两者的下一步动作完全不同。
 */
type BalanceState = {
  status: "idle" | "loading" | "ok" | "error";
  balance?: VendorBalance;
  error?: string;
};

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
 * 表格里的费率一格：`$0.15 / $0.60`。
 *
 * 未设置的那一边用 `-`：**0 是免费，未设置是「不知道」**，两者在成本估算里
 * 是完全不同的两件事，因此不能都显示成 `$0`。
 */
export function priceText(pricing: ApiVendor["pricing"]): string {
  const input = pricing?.inputPerMillionUsd;
  const output = pricing?.outputPerMillionUsd;
  if (input === undefined && output === undefined) return "-";
  const part = (value: number | undefined) => (value === undefined ? "-" : `$${formatPrice(value)}`);
  return `${part(input)} / ${part(output)}`;
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

/**
 * 行内余额文案。
 *
 * 与旧平台同一套状态机（`vendor-manager-dialog.tsx` 的 `renderVendorBalance`）：
 * 查询中… / 金额 / 查询失败 / 未获取，失败时留着上一次的数字并标「（已过期）」。
 * 后端说 `isValid === false`（「密钥可能无效」）时在上述文案后追加一句——
 * 旧平台的那个标签在重排时不能丢（它是唯一指向「去查密钥」的信号）。
 */
export function balanceLabel(state: BalanceState | undefined): string {
  if (!state || state.status === "idle") return "未获取";
  if (state.status === "loading") return "查询中…";
  const text = state.balance ? balanceText(state.balance) : "未提供";
  // 「上游不给余额」（未提供）与「这次查失败」都要说成查询失败：两者在界面上
  // 都需要用户再试一次或去看配置，而 `balanceText` 只管有没有数字。
  const label =
    text === "未提供"
      ? "查询失败"
      : state.status === "error"
        ? `${text}（已过期）`
        : text;
  // **`isValid === false` 是一句独立的话**（后端明确说「密钥可能无效」），
  // 不是「查询失败」的同义词——数字查得到也会带上它。因此追加而不是并进上面那句；
  // 丢掉它，用户就只会看到一个笼统的失败，不知道该去查密钥。
  return state.balance?.isValid === false ? `${label}（密钥可能无效）` : label;
}

/** 健康状态 → 界面文案。四个取值照旧平台（`open` 是熔断中，不是「离线」）。 */
export function healthLabel(status: string): string {
  if (status === "open") return "已熔断";
  if (status === "half-open") return "探测中";
  if (status === "degraded") return "降级";
  return "健康";
}

function healthTone(status: string): "accent" | "warn" | "danger" {
  if (status === "open") return "danger";
  if (status === "degraded" || status === "half-open") return "warn";
  return "accent";
}

/**
 * 熔断剩余时间。
 *
 * 给不出正数（没有截止时间、时间格式不对、已经过期）就返回空串——
 * 那时熔断其实已经结束或没有熔断，写「剩 0 分钟」是假话。
 */
export function circuitRemaining(until: string | undefined, now = Date.now()): string {
  if (!until) return "";
  const at = new Date(until).getTime();
  if (Number.isNaN(at)) return "";
  const ms = at - now;
  if (ms <= 0) return "";
  const minutes = Math.ceil(ms / 60_000);
  return minutes < 60 ? `${minutes} 分钟` : `${Math.round(minutes / 60)} 小时`;
}

export function VendorsPanel({
  providerId,
  providers,
  onError,
  onNotify
}: {
  /** 侧栏当前选中的平台。只用来给「新增」表单一个**默认值**。 */
  providerId: AiProviderId | "";
  /** 全部平台。厂商的按钮与筛选都从它来——名称与侧栏逐字一致。 */
  providers: AiProviderSummary[];
  onError: (message: string) => void;
  /** 成功反馈。见 `SettingsCenter` 同名字段的说明。 */
  onNotify?: (message: string) => void;
}) {
  const [vendors, setVendors] = useState<ApiVendor[]>([]);
  const [loading, setLoading] = useState(true);
  const [filter, setFilter] = useState<ProviderFilter>("all");
  /** 正在编辑的草稿；`id` 为空表示新建。`null` 表示表单未打开（显示主表）。 */
  const [draft, setDraft] = useState<VendorDraft | null>(null);
  /** 下方展开的工具：测连接 / 读本地配置。空串表示都收起。 */
  const [tool, setTool] = useState<"" | "probe" | "config">("");
  /** 批量刷余额是否在跑。单家的进行中状态在 `balance` 里逐行记。 */
  const [busy, setBusy] = useState(false);
  /** 每一行的余额状态。不在表里的行就是「未获取」。 */
  const [balance, setBalance] = useState<Record<string, BalanceState>>({});
  /**
   * 健康快照。
   *
   * 空数组 = 还没取或取不到（那张表在某些版本里恒为空）。**这一列不编造内容**：
   * 没有快照的那一家那一格就是空的，而不是写一句「未知」假装有数据。
   */
  const [health, setHealth] = useState<GatewayVendorHealth[]>([]);
  const toast = useToast();

  const load = useCallback(async () => {
    setLoading(true);
    try {
      // **一次取全量**：chips 上的条数要按各厂商的分布算，只取当前筛选的结果就算不出来。
      // 供应商是「几十条」这个量级，一次取回并在前端过滤更省事，也更不容易错。
      const [list, healthList] = await Promise.all([
        api.listVendors(),
        // 健康是可选增强：读不到就整列留空，不把它升级成一条错误。
        api.getVendorHealth().catch(() => [] as GatewayVendorHealth[])
      ]);
      setVendors(list);
      setHealth(Array.isArray(healthList) ? healthList : []);
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    } finally {
      setLoading(false);
    }
  }, [onError]);

  useEffect(() => {
    void load();
  }, [load]);

  const toggle = async (vendor: ApiVendor) => {
    try {
      if (vendor.enabled) {
        // 停用：只翻启用位，**不动文件**——把用户 CLI 里的配置改回去不是
        // 「停用」该做的事（那份文件可能有他自己后来加的改动）。
        await api.setVendorEnabled(vendor.id, false);
        await load();
        onNotify?.(`已禁用供应商：${vendor.name}`);
        return;
      }
      // 启用：**先写 CLI 配置（带备份），再翻启用位**。旧平台的含义就是这样——
      // 供应商被启用的意思是「CLI 的配置指向它」。
      const result = await api.enableVendor(vendor.id);
      await load();
      onNotify?.(
        result.written.length === 0
          ? `已启用供应商：${vendor.name}（这个平台没有标准配置模板，未写入文件）`
          : `已启用供应商：${vendor.name}，写入 ${result.written.length} 个配置文件，原文件已备份到 ${result.backupRoot}`
      );
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    }
  };

  /**
   * 删除一家供应商。
   *
   * **不可撤销，因此先问一次**。那句警示照旧平台原文：
   * 删掉的是数据库里的这条记录，**已经写进 CLI 配置文件的内容不会跟着还原**——
   * 用户以为删了就干净了，是最容易出事的一种误解。
   */
  const remove = useCallback(
    async (vendor: ApiVendor) => {
      if (!window.confirm(`确认删除供应商「${vendor.name}」？此操作不会还原已写入的 CLI 配置。`)) {
        return;
      }
      try {
        await api.deleteVendor(vendor.id);
        onNotify?.(`供应商已删除：${vendor.name}`);
        await load();
      } catch (error) {
        onError(error instanceof Error ? error.message : String(error));
      }
    },
    [load, onError, onNotify]
  );

  /** 刷新一家。失败时**留着上一次成功的数字**并标「已过期」。 */
  const refreshOne = useCallback(
    async (vendor: ApiVendor) => {
      setBalance((current) => ({ ...current, [vendor.id]: { ...current[vendor.id], status: "loading" } }));
      try {
        const result = await api.refreshVendorBalance(vendor.id);
        setBalance((current) => ({
          ...current,
          [vendor.id]: result.error
            ? { status: "error", balance: current[vendor.id]?.balance, error: result.error }
            : { status: "ok", balance: result.balance }
        }));
        if (result.error) onError(`余额查询失败：${result.error}`);
      } catch (problem) {
        setBalance((current) => ({
          ...current,
          [vendor.id]: { status: "error", balance: current[vendor.id]?.balance }
        }));
        onError(problem instanceof Error ? problem.message : String(problem));
      }
    },
    [onError]
  );

  /**
   * 刷新全部。
   *
   * 后端**按批次并发**（避免给中转站造成瞬时压力），因此这一步会花几秒。
   * 返回的每一项各自成功或失败——一家失败不影响其他家，所以这里逐条报，
   * 而不是整体成功/失败。
   */
  const refreshAll = useCallback(async () => {
    if (vendors.length === 0) return;
    setBusy(true);
    // 先把每一行标成「查询中…」：批量要几秒，不标的话界面看起来没反应。
    setBalance((current) => {
      const next = { ...current };
      for (const vendor of vendors) next[vendor.id] = { ...current[vendor.id], status: "loading" };
      return next;
    });
    try {
      const results = await api.refreshAllVendorBalances();
      setBalance((current) => {
        const next = { ...current };
        for (const result of results) {
          next[result.vendorId] = result.error
            ? { status: "error", balance: current[result.vendorId]?.balance, error: result.error }
            : { status: "ok", balance: result.balance };
        }
        return next;
      });
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
      setBusy(false);
    }
  }, [vendors, toast, onError]);

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

  const startNew = () => {
    // 表单开着时点「新增」会把草稿整个换掉——和删除一样是**不可逆**的（刚填的没了），
    // 因此先问一次；取消则什么都不动。手法与删除确认一致（`window.confirm`）。
    if (draft !== null && !window.confirm("正在编辑供应商，放弃当前修改并新建？")) return;
    setDraft({
      id: "",
      providerId: defaultProvider(),
      name: "",
      apiKey: "",
      apiBaseUrl: "",
      sort: String(vendors.length),
      /**
       * **新建默认不启用**。
       *
       * 启用会去**写用户的 CLI 配置文件**（见 `toggle`），那是一个有副作用的动作，
       * 不该由「点了新增」顺手完成。用户先把地址、密钥、费率填对、确认无误，
       * 再自己点「启用」。
       */
      enabled: false,
      inputPrice: "",
      outputPrice: "",
      balanceQuery: "",
      modelQuery: ""
    });
  };

  /** 平台显示名。取后端给的 `label`（与侧栏逐字一致），查不到才退回编号。 */
  const labelOf = (id: string): string =>
    providers.find((item) => item.id === id)?.label ?? id;

  /** 健康快照按供应商编号找。编号全局唯一，因此不必再比 `providerId`。 */
  const healthOf = (id: string): GatewayVendorHealth | undefined =>
    health.find((item) => item.vendorId === id);

  /**
   * 这家是不是它所在候选池的**第一顺位**。
   *
   * 后端的候选池按 `sort` 升序取第一家（`compare_vendor_order`），因此「优先」
   * 不是装饰：它就是网关在没有既有绑定时会先试的那一家。
   *
   * 判定只能在**可见字段**上做——后端还要求密钥非空，而密钥从不发给界面
   * （`ApiVendor` 刻意没有这个字段），因此这里用地址非空代替，并在文案上
   * 只说「已启用 · 优先」而不是断言它一定能被选中。
   */
  const isFirstInPool = (vendor: ApiVendor): boolean => {
    if (!vendor.enabled) return false;
    const pool = vendors
      .filter(
        (item) =>
          item.enabled &&
          item.providerId === vendor.providerId &&
          item.apiBaseUrl.trim() !== ""
      )
      .slice()
      .sort(
        (left, right) =>
          left.sort - right.sort || (left.createdAt ?? "").localeCompare(right.createdAt ?? "")
      );
    return pool[0]?.id === vendor.id;
  };

  const shown = filter === "all" ? vendors : vendors.filter((item) => item.providerId === filter);
  const enabledCount = shown.filter((item) => item.enabled).length;
  const countOf = (id: string) => vendors.filter((item) => item.providerId === id).length;

  /** 表单开着时不允许切走：那会把用户刚填的东西悄悄丢掉。 */
  const editing = draft !== null;

  return (
    <div className="vendor-panel">
      {toast.message && (
        <div className="export-hint" role="status">
          {toast.message}
          <Button small onClick={toast.dismiss}>
            知道了
          </Button>
        </div>
      )}

      <PanelHeader
        title={`供应商管理 (${vendors.length})`}
        description="配置 Codex、Claude、Gemini、Qoder 的 API 端点、密钥与费率；网关按厂商把它们放进对应的候选池。"
        actions={
          <>
            <Button
              small
              disabled={vendors.length === 0 || busy || editing}
              title="查询所有供应商的在线余额（按批次并发，要几秒）"
              onClick={() => void refreshAll()}
            >
              查余额
            </Button>
            <Button
              small
              disabled={vendors.length === 0 || editing}
              title="选一家供应商，查它现在能列出哪些模型"
              // 再点一次收起：这个按钮是「展开/收起」，不是一次性动作。
              onClick={() => setTool((current) => (current === "probe" ? "" : "probe"))}
            >
              测连接（列模型）
            </Button>
            <Button
              small
              disabled={editing}
              title="读取 CLI 实际读到的配置文件（只读白名单里的三个路径）"
              onClick={() => setTool((current) => (current === "config" ? "" : "config"))}
            >
              读本地配置
            </Button>
            <Button small variant="primary" onClick={startNew} title="打开新增表单">
              {/* 图标而不是文字「＋」：按钮的可读名称要正好是「新增供应商」，
                  多一个符号会让按名字找它的读屏用户与测试都对不上。 */}
              <Icon name="plus" size={13} />
              新增供应商
            </Button>
          </>
        }
      />

      {draft ? (
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
              **与 0 不是一回事**：0 是免费，未设置是「不知道」，表里那一格显示 `-`。
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
              那是一句这个界面做不到的承诺；后来一度矫枉过正改写成「本界面不会写 CLI 配置」，
              同样不对。两种动作的后果必须如实分开说：**

              实测：**启用**走 `vendor:enable`，它会把供应商的配置模板写进用户的 CLI
              配置文件（写前备份；这个平台没有标准模板时不写，提示里会说）。
              **禁用**走 `vendor:set-enabled`，只翻数据库里的标志位、不动文件。

              表单保存本身改的只是应用数据库里的那条记录；凭据由网关在终端启动时注入。
            */}
            <span className="row-meta">
              表单保存的是**应用数据库里的记录**；「启用」会把配置模板写进你的 CLI
              配置文件（写前备份），「禁用」只翻启用位、不改动文件。
            </span>
          </Toolbar>
        </div>
      ) : (
        <>
          <Toolbar>
            {/*
              厂商过滤用 chips 而不沿用下拉：每一家后面跟着**条数**，
              而条数只有与「全部」并排时才读得出分布。
            */}
            <ChipGroup
              value={filter}
              onChange={(value) => setFilter(value as ProviderFilter)}
              /* 这一组 chips 在无障碍树里要有名字：旧平台的筛选下拉有
                 `aria-label="按平台筛选供应商"`，换成 chips 时不能丢。 */
              ariaLabel="按厂商筛选供应商"
              options={[
                { value: "all", label: `全部 (${vendors.length})` },
                ...providers.map((item) => ({
                  value: item.id,
                  label: `${item.label} (${countOf(item.id)})`
                }))
              ]}
            />
            <ToolbarSpacer />
            <span className="row-meta">
              {busy
                ? "正在刷新全部余额（按批次并发）…"
                : loading
                  ? "加载中…"
                  : `${shown.length} 个 · ${enabledCount} 个启用`}
            </span>
          </Toolbar>

          {loading ? (
            <LoadingBlock label="正在读取供应商…" />
          ) : shown.length === 0 ? (
            <EmptyState
              message={
                filter === "all"
                  ? "还没有配置任何供应商。"
                  : `${labelOf(filter)} 下还没有供应商。`
              }
              hint="供应商决定 CLI 走哪个 API 地址与密钥。"
              action={<Button onClick={startNew}>新增供应商</Button>}
            />
          ) : (
            <table className="data-table">
              <thead>
                <tr>
                  <th>名称 / 端点</th>
                  {/* 厂商列非有不可：列表是跨平台的，不标出来同名供应商分不清。 */}
                  <th>厂商</th>
                  <th>费率 (输入/输出/M)</th>
                  <th>健康</th>
                  <th>候选池状态</th>
                  {/* 这一格装着两样东西：余额状态机（含单家刷新）与三个操作按钮。
                      列头如实写出来，用户才找得到「余额」在哪一列。 */}
                  <th>操作 / 余额</th>
                </tr>
              </thead>
              <tbody>
                {shown.map((vendor) => {
                  const snapshot = healthOf(vendor.id);
                  const state = balance[vendor.id];
                  const remaining = snapshot ? circuitRemaining(snapshot.circuitUntil) : "";
                  /*
                    健康这一格**只列真实可得的信息**：
                    快照里没有的（例如延时——后端根本不记）就不显示，
                    不拿 0 或「未知」去填满它。
                  */
                  const healthDetails = snapshot
                    ? [
                        snapshot.failureCount > 0
                          ? `失败率 ${Math.round(snapshot.failureRate * 100)}%`
                          : "",
                        snapshot.lastSuccessAt ? `最近成功 ${formatWhen(snapshot.lastSuccessAt)}` : "",
                        remaining ? `熔断剩余 ${remaining}` : ""
                      ].filter(Boolean)
                    : [];
                  return (
                    <tr key={vendor.id}>
                      <td>
                        <div className="vendor-identity">
                          <span className="vendor-identity-name">{vendor.name}</span>
                          {/* 地址可能很长，用等宽字体——截断会让同名中转商分不出来。 */}
                          <span className="mono">{vendor.apiBaseUrl}</span>
                        </div>
                      </td>
                      <td>{labelOf(vendor.providerId)}</td>
                      <td className="mono">{priceText(vendor.pricing)}</td>
                      <td>
                        {snapshot && (
                          <div className="vendor-health">
                            <Badge tone={healthTone(snapshot.status)}>
                              {healthLabel(snapshot.status)}
                            </Badge>
                            {healthDetails.length > 0 && (
                              <span className="row-meta">{healthDetails.join(" · ")}</span>
                            )}
                          </div>
                        )}
                      </td>
                      <td>
                        <Badge tone={vendor.enabled ? "accent" : "neutral"}>
                          {vendor.enabled
                            ? isFirstInPool(vendor)
                              ? "已启用 · 优先"
                              : "已启用"
                            : "未启用"}
                        </Badge>
                      </td>
                      <td>
                        <div className="vendor-balance-line">
                          <span className="row-meta" title={state?.error ?? undefined}>
                            余额 {balanceLabel(state)}
                          </span>
                          {/* 单家刷新入口：批量要几秒，而用户常常只想知道某一家。 */}
                          <Button
                            small
                            disabled={busy || state?.status === "loading"}
                            aria-label={`刷新 ${vendor.name} 的余额`}
                            title="只刷新这一家的余额"
                            onClick={() => void refreshOne(vendor)}
                          >
                            刷新
                          </Button>
                        </div>
                        <div className="vendor-row-actions">
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
                          <Button
                            small
                            title={
                              vendor.enabled
                                ? "禁用：只翻启用位，不改动你的 CLI 配置文件"
                                : "启用：先把配置模板写进 CLI 配置文件（写前备份），再翻启用位"
                            }
                            onClick={() => void toggle(vendor)}
                          >
                            {vendor.enabled ? "禁用" : "启用"}
                          </Button>
                          <Button small variant="danger" onClick={() => void remove(vendor)}>
                            删除
                          </Button>
                        </div>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}

          {/* 工具与调试信息在主表**下方**：它们是偶尔用一次的动作，
              放上面会把「这十几家现在什么样」挤出首屏。 */}
          {tool === "probe" && <VendorModelProbe vendors={vendors} onError={onError} />}
          {tool === "config" && <VendorConfigReader />}
        </>
      )}
    </div>
  );
}
