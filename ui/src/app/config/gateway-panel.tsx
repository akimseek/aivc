import { useCallback, useEffect, useState } from "react";

import { EmptyState } from "../../components";
import { PanelHeader } from "../../features/settings/panel-header";
import * as api from "../../ipc/commands";
import type { ExternalApiStatus, GatewayPortStatus } from "../../types";
import { GatewayInsights } from "../gateway-insights";
import { GatewayExternalCard } from "./gateway-external-card";
import { GatewayPortCard } from "./gateway-port-card";
import { GatewayRulesCard } from "./gateway-rules-card";
import { SwitchPill } from "./gateway-switch-pill";

/**
 * 本地网关 & 路由（demo 第 1006-1371 行）。
 *
 * # 这一页只做组装与取数
 *
 * 页面所需的两份状态（网关端口/阈值、外部 API）在这里各读一次，再传给两张卡——
 * **不让每张卡自己读一份**：总开关一按、端口一改，两份副本就会各说各话，
 * 而这一页上的每个数字都是要照着配外部客户端的。
 *
 * # 三张卡各自的职责
 *
 * | 卡 | 文件 | 管什么 |
 * | --- | --- | --- |
 * | 端口与熔断韧性 | `gateway-port-card.tsx` | 端口 + 三条阈值，集中保存 |
 * | 外部网关 | `gateway-external-card.tsx` | 外部开关、共享令牌、平台端点清单 |
 * | 故障规则 | `gateway-rules-card.tsx` | 规则列表 + 表单 + 在线测试 |
 *
 * 下面还留着 `GatewayInsights`（用量 / 供应商健康 / 最近请求）：它的
 * 「重置全部健康」是**全应用唯一**的熔断重置入口（`gateway:reset-vendor-health`），
 * 删掉这一块就等于把这个能力从界面上摘掉。用量、日志、异常各自有专属面板
 * （导航里的「用量报表 / 异常诊断 / 请求日志」），这里只是扫一眼的那一份。
 */
export function GatewayPanel({
  onError,
  onNotify
}: {
  onError: (message: string) => void;
  /** 成功反馈。见 `SettingsCenter` 同名字段的说明。 */
  onNotify?: (message: string) => void;
}) {
  const [status, setStatus] = useState<GatewayPortStatus | null>(null);
  const [external, setExternal] = useState<ExternalApiStatus | null>(null);
  /**
   * 总开关的请求在途。
   *
   * 在途时第二次点击读到的是**切换前**的 `status`，于是刚改完的状态又被翻回去
   * （后端每次都要写一遍设置）。因此把它交给 `SwitchPill` 的 `disabled`。
   */
  const [toggling, setToggling] = useState(false);

  const load = useCallback(async () => {
    // 两份状态**互不依赖**，因此不用 `Promise.all`：那样一条失败会把另一条也废掉
    // （见下面 external 的处理）。
    const [port, externalStatus] = await Promise.allSettled([
      api.getGatewayPort(),
      api.getExternalApiStatus()
    ]);

    // 端口状态是**整页的门面**（端口卡、三条阈值都靠它），读不到就真的没有页面
    // 可渲染，只能停在等待态。假后端（外壳测试里那张按命令名给值的表）对没登记的
    // 命令回 `null`，也走这里。
    if (port.status === "rejected") {
      onError(describeFailure(port.reason));
      return;
    }
    if (!port.value) return;
    setStatus(port.value);

    /*
      外部状态读不到时**不能让整页一起停住**：它此前与端口状态捆在 `Promise.all`
      里，一失败整页就永远停在「读取网关状态…」——而端口与阈值那一团明明读得到、
      也用得上。现在退化成「关着且没有令牌」这一档继续渲染，失败本身仍然报出去
      （否则没人知道看到的是退化值，而它看起来与真值没有区别）。
    */
    if (externalStatus.status === "fulfilled" && externalStatus.value) {
      setExternal(externalStatus.value);
      return;
    }
    if (externalStatus.status === "rejected") {
      onError(describeFailure(externalStatus.reason));
    }
    setExternal({ enabled: false, hasToken: false });
  }, [onError]);

  useEffect(() => {
    void load();
  }, [load]);

  const toggleGateway = useCallback(async () => {
    // 在途时不再接第二次：`disabled` 已经挡在界面上，这里是同一件事的第二道闸。
    if (!status || toggling) return;
    setToggling(true);
    try {
      const next = await api.setGatewayEnabled(!status.enabled);
      setStatus(next);
      // 关网关会**立刻切断终端路由**（旧平台专门写了这句警告），
      // 因此这里必须说清是哪一种结果。
      onNotify?.(
        next.enabled
          ? "已启用本地网关。重启应用后开始监听。"
          : "已关闭本地网关。使用网关的终端路由将立即停止。"
      );
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    } finally {
      setToggling(false);
    }
  }, [status, toggling, onNotify, onError]);

  if (!status || !external) {
    return (
      <>
        {/* 头部先出现：等待状态读回来时，用户至少知道自己在哪一页。 */}
        <PanelHeader
          title="本地网关 & 路由"
          description="控制本地反向代理端口、外部客户端接入、熔断器阈值与故障识别引擎。"
        />
        <EmptyState message="读取网关状态…" />
      </>
    );
  }

  return (
    <div className="gateway-panel">
      <PanelHeader
        title="本地网关 & 路由"
        description="控制本地反向代理端口、外部客户端接入、熔断器阈值与故障识别引擎。"
        actions={
          <SwitchPill
            on={status.enabled}
            label={status.enabled ? "本地网关已启用" : "本地网关已关闭"}
            action={status.enabled ? "关闭网关" : "启用网关"}
            disabled={toggling}
            onClick={() => void toggleGateway()}
          />
        }
      />
      <GatewayPortCard
        status={status}
        onStatusChange={setStatus}
        onError={onError}
        onNotify={onNotify}
      />
      <GatewayExternalCard
        // 端点清单要用真的监听地址：端口自动分配时它每次都不同，
        // 界面自己拼只会拼出 `http://127.0.0.1:0`。
        baseUrl={status.baseUrl}
        status={external}
        onStatusChange={setExternal}
        onError={onError}
        onNotify={onNotify}
      />
      <GatewayRulesCard onError={onError} onNotify={onNotify} />
      <GatewayInsights onError={onError} />
    </div>
  );
}

/**
 * 取数失败的原因 → 一句话。
 *
 * `Promise.allSettled` 的拒因是 `unknown`，与别处的 `catch` 同一条规矩：
 * `Error` 取 `message`，其余原样字符串化（后端抛的往往是字符串）。
 */
function describeFailure(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}
