import { useCallback, useState } from "react";

import { Badge, Button, Card, Notice } from "../../components";
import * as api from "../../ipc/commands";
import type { AiProviderId, ExternalApiStatus } from "../../types";
import { SwitchPill } from "./gateway-switch-pill";

/**
 * 卡 2：外部网关与 API 接入（demo 第 1077-1171 行）。
 *
 * # 端点清单以**后端真实路由**为准，不照抄 demo 字符串
 *
 * demo 写的是 `/external/codex/v1`、`/external/claude` 这类路径——那是**旧平台**的形制
 * （`D:/code/ai/src/electron/gateway/vendor-gateway.ts:238-240`）。
 * Rust 侧的路由前缀不在这里：见 `crates/aivc-gateway/src/routes.rs`：
 *
 * - `routes.rs:33` —— `pub const ROUTE_PREFIX: &str = "/gateway";`
 * - `routes.rs:112-114` —— `route_prefix()` 拼出 `/gateway/<平台>/<路由编号>`
 * - `routes.rs:102-108` —— `build_route_url()` 用 host + 上面的前缀，host 会去掉尾斜杠
 * - `routes.rs:118` —— 路径形状就是 `/gateway/<平台>/<路由编号>/…`
 *
 * 客户端把「编号」替换成自己那条外部路由的编号，**前缀之后的路径原样转发**给上游
 * （`crates/aivc-gateway/src/server.rs:682` 的 `strip_route_prefix`）。
 */

/** 四个平台。顺序与侧栏/平台枚举一致，界面上的排列因此是稳定的。 */
export const EXTERNAL_PROVIDERS: readonly AiProviderId[] = [
  "codex",
  "claude",
  "gemini",
  "qoder"
];

const PROVIDER_LABELS: Record<AiProviderId, string> = {
  codex: "Codex",
  claude: "Claude",
  gemini: "Gemini",
  qoder: "Qoder"
};

/**
 * 每个平台客户端习惯用的凭据携带方式。
 *
 * 后端其实**四种都收**——`AuthRequest` 收集 `Authorization: Bearer`、`x-api-key`、
 * `x-goog-api-key` 与查询串里的 `key`（`crates/aivc-gateway/src/routes.rs:152-196`，
 * 组装点见 `crates/aivc-gateway/src/server.rs:559-565`）。这里给出的是各家客户端自己的约定。
 */
const PROVIDER_AUTH: Record<AiProviderId, string> = {
  codex: "Authorization: Bearer <Token>",
  claude: "x-api-key: <Token>",
  gemini: "x-goog-api-key: <Token> 或 ?key=<Token>",
  qoder: "Authorization: Bearer <Token>"
};

/**
 * 外部路由编号在界面上的占位符。
 *
 * **不能编一个具体的值**：外部路由由网关注册，编号由它分配，而当前没有任何命令
 * 把它读给界面（`gateway:get-external-api-status` 只回 `enabled` / `hasToken`）。
 * 写死一个假编号会让用户复制到一个 404 的地址，比明说「这里是编号」更糟。
 */
export const ROUTE_ID_PLACEHOLDER = "<routeId>";

export type ExternalEndpoint = {
  provider: AiProviderId;
  label: string;
  /** 形如 `http://127.0.0.1:8787/gateway/codex/<routeId>`。 */
  url: string;
  /** 该平台客户端常用的凭据头。 */
  auth: string;
};

/**
 * 生成四个平台的接入端点。
 *
 * 没在监听（`baseUrl` 为空）时返回空数组：没有地址可抄，编一个 `:0` 的地址只会误导。
 */
export function buildExternalEndpoints(
  baseUrl: string,
  providers: readonly AiProviderId[] = EXTERNAL_PROVIDERS
): ExternalEndpoint[] {
  // host 去尾斜杠：`build_route_url` 就是 `host.trim_end_matches('/') + 前缀`。
  const host = baseUrl.trim().replace(/\/+$/, "");
  if (host === "") return [];
  return providers.map((provider) => ({
    provider,
    label: PROVIDER_LABELS[provider],
    url: `${host}/gateway/${provider}/${ROUTE_ID_PLACEHOLDER}`,
    auth: PROVIDER_AUTH[provider]
  }));
}

export function GatewayExternalCard({
  baseUrl,
  status,
  onStatusChange,
  onError,
  onNotify
}: {
  /** 当前监听地址（取自网关状态）；空串表示网关没在监听。 */
  baseUrl: string;
  status: ExternalApiStatus;
  onStatusChange: (next: ExternalApiStatus) => void;
  onError: (message: string) => void;
  onNotify?: (message: string) => void;
}) {
  /** 刚轮换出来的令牌。**只在这里出现一次**，界面必须留着它让用户复制。 */
  const [revealedToken, setRevealedToken] = useState("");
  /** 复制成功提示：`"token"` 或平台编号。 */
  const [copied, setCopied] = useState("");

  const copy = useCallback(
    async (key: string, text: string) => {
      try {
        await api.copyText(text);
        setCopied(key);
        window.setTimeout(() => setCopied(""), 2000);
      } catch (error) {
        onError(error instanceof Error ? error.message : String(error));
      }
    },
    [onError]
  );

  const toggle = async () => {
    try {
      const next = await api.setExternalApiEnabled(!status.enabled);
      onStatusChange(next);
      onNotify?.(next.enabled ? "外部 API 已启用。" : "外部 API 已关闭。");
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    }
  };

  const rotate = async () => {
    try {
      const result = await api.rotateExternalApiToken();
      // 令牌只在这里出现一次，界面必须留着它让用户复制。
      setRevealedToken(result.token);
      // 本地状态跟着更新：轮换之后「已设置」必然为真。
      onStatusChange({ enabled: status.enabled, hasToken: true });
      // 旧令牌**立即失效**，这句不说用户不会知道别的客户端已经断了。
      onNotify?.("已生成新令牌；旧令牌立即失效。");
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    }
  };

  const endpoints = buildExternalEndpoints(baseUrl);

  return (
    <Card
      title="外部网关与 API 接入"
      description="供本机第三方应用与 IDE 扩展调用；按路径选择平台并复用供应商路由池与熔断机制。"
      actions={
        <SwitchPill
          on={status.enabled}
          label={status.enabled ? "外部网关已开启" : "外部网关已关闭"}
          action={status.enabled ? "关闭外部 API" : "启用外部 API"}
          onClick={() => void toggle()}
        />
      }
    >
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
          <Button small onClick={() => void rotate()}>
            轮换令牌
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
                  <span className="gateway-endpoint-auth">{item.auth}</span>
                </div>
                <Button small onClick={() => void copy(item.provider, item.url)}>
                  {copied === item.provider ? "已复制" : "复制"}
                </Button>
              </div>
            ))}
          </div>
        )}
        <p className="field-hint">
          接入路径的形状是 <code>/gateway/&lt;平台&gt;/&lt;路由编号&gt;</code>
          （<code>crates/aivc-gateway/src/routes.rs:112</code>）：把 <code>&lt;routeId&gt;</code>
          换成该平台外部路由的编号，<b>前缀之后的路径原样转发</b>给上游（例如 Codex 客户端
          再拼 <code>/v1/responses</code>）。凭据四种都可以：<code>Authorization: Bearer</code>
          、<code>x-api-key</code>、<code>x-goog-api-key</code> 或查询参数 <code>?key=</code>
          （四者统一用上面那个共享令牌）；查询参数那种在转发前会被摘掉，不会带给上游。
        </p>
      </div>
    </Card>
  );
}
