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

  const load = useCallback(async () => {
    try {
      const [port, externalStatus] = await Promise.all([
        api.getGatewayPort(),
        api.getExternalApiStatus()
      ]);
      // 假后端（外壳测试里那张按命令名给值的表）对没登记的命令回 `null`。
      // 那种情况下保持等待态即可——真后端这两条一定给对象。
      if (!port || !externalStatus) return;
      setStatus(port);
      setExternal(externalStatus);
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    }
  }, [onError]);

  useEffect(() => {
    void load();
  }, [load]);

  const toggleGateway = useCallback(async () => {
    if (!status) return;
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
    }
  }, [status, onNotify, onError]);

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
