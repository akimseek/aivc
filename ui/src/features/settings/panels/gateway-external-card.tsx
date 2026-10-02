import { useCallback, useState } from "react";

import { Badge, Button, Card, Notice, errorText } from "../../../components";
import * as api from "../../../ipc/commands";
import type { AiProviderId, AiProviderSummary, ExternalApiStatus } from "../../../types";
import { SwitchPill } from "./gateway-switch-pill";

/**
 * 卡 2：外部接入（demo 第 1077-1171 行）。
 *
 * # 端点清单 = 网关根地址 + 平台路径（2026-10-02 统一外部接入）
 *
 * 外部接入路径是 `/<平台>`（如 `http://127.0.0.1:8787/codex`），直达该平台的候选池；
 * 协议后缀（`/v1/responses` 等）由接入方按自己的协议追加、网关原样透传。
 * 后端：`extract_external_provider` 认首段平台、每平台惰性注册一条合成路由
 * （`external-<平台>`），前缀剥离走 `VendorRoute::prefix()`（外部路由即 `/<平台>`）。
 * 设计与裁定见 `docs/superpowers/specs/2026-10-02-gateway-unified-external-design.md`。
 *
 * 旧两种形制都不能用：demo 的 `/external/<平台>`（旧 Electron 版）与更早界面上的
 * `/gateway/<平台>/<routeId>` 占位符（Rust 版外部路由当时从未注册，那个地址必然 404）。
 */

/**
 * 本地兜底平台清单。
 *
 * **真源是后端 `ai:list-providers`**（`providers`）；这一份只在拿不到平台列表时
 * 兜底。形制是 `Record<AiProviderId, true>` 再取键：新增平台时漏写一个键是
 * **编译错误**，而不是界面上静默少一条端点。
 */
const FALLBACK_PROVIDERS: Record<AiProviderId, true> = {
  codex: true,
  claude: true,
  gemini: true,
  qoder: true
};

/** 兜底平台清单（`Object.keys` 保持声明顺序，界面上的排列因此是稳定的）。 */
export const EXTERNAL_PROVIDERS: readonly AiProviderId[] = Object.keys(
  FALLBACK_PROVIDERS
) as readonly AiProviderId[];

/**
 * 平台编号序列（清单的**唯一推导点**）。
 *
 * 优先用后端 `providers`——顺序即显示顺序；拿不到（`undefined`/空）时才退回
 * 本地兜底清单。此前这里与规则表单各写一份手写数组，后端多一个平台时两处都
 * 不会有任何编译提示，只会静默少一条端点、少一个选项。
 */
export function providerIds(providers?: AiProviderSummary[]): readonly AiProviderId[] {
  return providers && providers.length > 0
    ? providers.map((item) => item.id)
    : EXTERNAL_PROVIDERS;
}

/**
 * 平台编号 → 显示名的**本地兜底表**。
 *
 * 真源是后端 `ai:list-providers` 的 label（见 `providerLabel`），这一份只在拿不到
 * 平台列表时兜底；值必须与后端逐字一致，否则同一个平台会出现两种叫法。
 * 修过的一次漂移：claude 写作 `"Claude"`，而后端是 `"Claude Code"`。
 *
 * 形制是 `Record<AiProviderId, …>`：新增平台时漏写是编译错误。
 */
export const PROVIDER_LABELS: Record<AiProviderId, string> = {
  codex: "Codex",
  claude: "Claude Code",
  gemini: "Gemini",
  qoder: "Qoder"
};

/**
 * 平台显示名。
 *
 * 优先用后端 `providers` 的 label（侧栏、标签栏、供应商面板、规则表、端点清单
 * 都是它），本地那份 `PROVIDER_LABELS` 只在拿不到平台列表时兜底——两处各叫各的时，
 * 同一个平台会在同一屏上出现「Claude」与「Claude Code」两种写法，用户会以为是两家。
 */
export function providerLabel(
  providers: AiProviderSummary[] | undefined,
  id: AiProviderId
): string {
  return providers?.find((item) => item.id === id)?.label ?? PROVIDER_LABELS[id];
}

/**
 * 外部路由编号在界面上的占位符已删除（2026-10-02）。
 *
 * 早前外部路由由网关注册、编号由它分配、而没有任何命令把它读给界面，
 * 因此界面只能展示 `…/gateway/<平台>/<routeId>` 这样一个**必然 404** 的地址。
 * 统一外部接入后端点就是 `{网关根地址}/<平台>`，编号概念对用户消失。
 */

export type ExternalEndpoint = {
  provider: AiProviderId;
  label: string;
  /** 形如 `http://127.0.0.1:8787/codex`：网关根地址 + 平台路径。 */
  url: string;
};

/**
 * 生成外部接入端点。
 *
 * 平台清单与显示名都从后端 `providers` 派生（顺序即显示顺序；名字与规则表共用
 * `providerLabel`，同屏不会一个叫「Claude」一个叫「Claude Code」）；拿不到平台
 * 列表时才退回本地兜底清单。
 *
 * 没在监听（`baseUrl` 为空）时返回空数组：没有地址可抄，编一个 `:0` 的地址只会误导。
 */
export function buildExternalEndpoints(
  baseUrl: string,
  providers?: AiProviderSummary[]
): ExternalEndpoint[] {
  // host 去尾斜杠，再拼平台路径（后端 `VendorRoute::prefix()` 对外部路由就是 `/<平台>`）。
  const host = baseUrl.trim().replace(/\/+$/, "");
  if (host === "") return [];
  return providerIds(providers).map((id) => ({
    provider: id,
    label: providerLabel(providers, id),
    url: `${host}/${id}`
  }));
}

export function GatewayExternalCard({
  baseUrl,
  providers,
  status,
  statusError,
  onRetry,
  onStatusChange,
  onError,
  onNotify
}: {
  /** 当前监听地址（取自网关状态）；空串表示网关没在监听。 */
  baseUrl: string;
  /**
   * 全部平台（后端 `ai:list-providers`）。
   *
   * 端点清单与平台名都从它派生——本地那份兜底表只在这份数据拿不到时使用。
   * 可选是为了让测试与旧调用点少传一个参数时仍能退回兜底清单。
   */
  providers?: AiProviderSummary[];
  status: ExternalApiStatus;
  /**
   * 状态读取失败的原因（`null`/省略 = 读到了）。
   *
   * 失败时 `status` 是面板给的**占位值**（关闭且无令牌），而它看起来与真值
   * 一模一样——不写明的话用户会以为自己真的关了外部 API。
   */
  statusError?: string | null;
  /** 重新读取状态（失败态下的出口）。 */
  onRetry?: () => void;
  onStatusChange: (next: ExternalApiStatus) => void;
  onError: (message: string) => void;
  onNotify?: (message: string) => void;
}) {
  /** 刚轮换出来的令牌。**只在这里出现一次**，界面必须留着它让用户复制。 */
  const [revealedToken, setRevealedToken] = useState("");
  /** 复制成功提示：`"token"` 或平台编号。 */
  const [copied, setCopied] = useState("");
  /** 开关的请求在途（见 `SwitchPill` 的说明：不挡第二下就会把状态又翻回去）。 */
  const [toggling, setToggling] = useState(false);
  /** 轮换的请求在途。**每一次轮换都会让上一个令牌立即失效**，因此不能连点。 */
  const [rotating, setRotating] = useState(false);

  const copy = useCallback(
    async (key: string, text: string) => {
      try {
        await api.copyText(text);
        setCopied(key);
        window.setTimeout(() => setCopied(""), 2000);
      } catch (error) {
        onError(errorText(error));
      }
    },
    [onError]
  );

  const toggle = async () => {
    // 在途时不再接第二次：`disabled` 已经挡在界面上，这里是同一件事的第二道闸。
    if (toggling) return;
    setToggling(true);
    try {
      const next = await api.setExternalApiEnabled(!status.enabled);
      onStatusChange(next);
      onNotify?.(next.enabled ? "外部 API 已启用。" : "外部 API 已关闭。");
    } catch (error) {
      onError(errorText(error));
    } finally {
      setToggling(false);
    }
  };

  const rotate = async () => {
    // 在途时不再接第二次：每轮换一次旧令牌**立即失效**，连点两下会让界面上
    // 展示的明文与后端最后生效的那个对不上（用户拿去更新客户端，仍然连不上）。
    if (rotating) return;
    setRotating(true);
    try {
      const result = await api.rotateExternalApiToken();
      // 令牌只在这里出现一次，界面必须留着它让用户复制。
      setRevealedToken(result.token);
      // 本地状态跟着更新：轮换之后「已设置」必然为真。
      onStatusChange({ enabled: status.enabled, hasToken: true });
      // 旧令牌**立即失效**，这句不说用户不会知道别的客户端已经断了。
      onNotify?.("已生成新令牌；旧令牌立即失效。");
    } catch (error) {
      onError(errorText(error));
    } finally {
      setRotating(false);
    }
  };

  // 清单与显示名都从 `providers` 派生（见 `buildExternalEndpoints`）：
  // 这一行的平台名与规则表的作用域列因此必然同源。
  const endpoints = buildExternalEndpoints(baseUrl, providers);

  return (
    <Card
      title="外部接入"
      description="供本机第三方应用与 IDE 扩展调用；地址即网关根地址 + 平台路径，复用供应商候选池与熔断机制。"
      actions={
        <SwitchPill
          on={status.enabled}
          label={status.enabled ? "外部接入已开启" : "外部接入已关闭"}
          action={status.enabled ? "关闭外部接入" : "启用外部接入"}
          disabled={toggling}
          onClick={() => void toggle()}
        />
      }
    >
      {/*
        失败态写在最上面：下面的开关与令牌状态都是占位值，先说清楚再给人看数据。
        重试入口放在这里而不是靠重新进入页面——一次读取失败不该让用户去别处绕一圈。
      */}
      {statusError != null && (
        <Notice
          message={
            <>
              外部接入状态读取失败，下面显示的是「关闭且无令牌」的占位值：
              {statusError}
            </>
          }
          actions={
            <Button small onClick={onRetry}>
              重新读取
            </Button>
          }
        />
      )}
      {/* 令牌区块：状态 + 说明 + 轮换。三者说的是同一件事，因此同框。 */}
      <div className="gateway-token-block">
        <div className="gateway-token-head">
          <div className="gateway-token-text">
            <div className="gateway-token-title">
              共享访问令牌
              <Badge tone={status.hasToken ? "accent" : "neutral"}>
                {status.hasToken ? "已设置" : "未设置"}
              </Badge>
            </div>
            {/*
              说清当前这一步该做什么。外部 API 开着却没有令牌是**半配置状态**：
              请求会全部被拒，而界面上只看得出「开着」。
              令牌本体**绝不回显**——后端只存哈希。
            */}
            <p className="field-hint">
              {!status.hasToken
                ? "尚未生成访问令牌。外部客户端现在连不上——请先点「轮换令牌」生成一个。"
                : status.enabled
                  ? "已配置；轮换后旧令牌立即失效，用它的客户端需要同步更新。"
                  : "已配置令牌，但外部 API 处于关闭状态。"}
            </p>
          </div>
          <Button
            small
            disabled={rotating}
            title="生成并启用新令牌；旧令牌立即失效"
            onClick={() => void rotate()}
          >
            {rotating ? "轮换中…" : "轮换令牌"}
          </Button>
        </div>

        {revealedToken && (
          // demo 第 1116-1128 行：令牌横幅（明文仅此一次）。
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
                <Button
                  small
                  variant="primary"
                  onClick={() => void copy("token", revealedToken)}
                >
                  {copied === "token" ? "已复制" : "复制令牌"}
                </Button>
                <Button small onClick={() => setRevealedToken("")}>
                  我已保存
                </Button>
              </>
            }
          />
        )}
      </div>

      <div className="gateway-endpoints">
        <div className="gateway-endpoints-head">
          <span className="gateway-endpoints-title">平台外部接入端点</span>
          {/* 只监听回环：局域网里的别人连不上，这一点决定了这个地址能不能外发。 */}
          <span className="row-meta">仅监听 127.0.0.1，拒绝局域网非本机连接</span>
        </div>
        {endpoints.length === 0 ? (
          <p className="field-hint">
            网关没有在监听（或还没选端口），因此还没有可用地址。先在上面保存并重启应用。
          </p>
        ) : (
          <div className="gateway-endpoint-list">
            {endpoints.map((item) => (
              <div className="gateway-endpoint" key={item.provider}>
                <div className="gateway-endpoint-main">
                  <span className="gateway-endpoint-platform">{item.label}</span>
                  <code className="mono gateway-endpoint-url" title={item.url}>
                    {item.url}
                  </code>
                </div>
                <Button
                  small
                  // 四行的按钮文字都是「复制」：读屏与自动化里四个同名按钮分不出谁是谁，
                  // 因此把**平台名**放进可访问名（可见文字保持短）。
                  aria-label={`复制 ${item.label} 端点`}
                  onClick={() => void copy(item.provider, item.url)}
                >
                  {copied === item.provider ? "已复制" : "复制"}
                </Button>
              </div>
            ))}
          </div>
        )}
        <p className="field-hint">
          把上面的地址作为外部客户端的<b> API 基地址</b>使用：协议路径（例如 Codex 客户端的
          <code>/v1/responses</code>、Claude 的 <code>/v1/messages</code>）由客户端按自己的
          协议追加，网关把平台之后的路径<b>原样透传</b>给供应商。凭据用上面那个共享令牌，
          四种携带方式都认：<code>Authorization: Bearer</code>、<code>x-api-key</code>、
          <code>x-goog-api-key</code> 或查询参数 <code>?key=</code>（查询参数那种在转发前会被
          摘掉，不会带给上游）。
        </p>
      </div>
    </Card>
  );
}
