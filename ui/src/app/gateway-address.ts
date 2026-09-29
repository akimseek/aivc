import type { GatewayPortStatus } from "../types";

/**
 * 网关那一格该显示什么。
 *
 * # 为什么单拎出来
 *
 * 这里有**四种**处境，而此前只处理了两种的一半：
 *
 * | 处境 | 显示 |
 * | --- | --- |
 * | 还没取到 | 读取中… |
 * | 设置里没开 | 未启用 |
 * | 开了但没在监听 | 已启用，未监听 |
 * | 正在监听 | `:50064` |
 *
 * 第三行是这次补的，也是**最容易漏的一种**：设置里开着，但端口被占、启动失败或
 * 还在启动中。把它显示成「未启用」会让人去设置里翻一个本来就是开的开关。
 *
 * # 为什么状态栏与工作台共用这一个函数
 *
 * 它们此前各写了一遍，而且都写着 `activePort || configuredPort`——端口为 0 时
 * 这个表达式会算出 `:undefined`（字段名错位）或者 `:0`（自动分配时按配置端口显示）。
 * 一处算错只有一格显示不对，很难发现。
 */
export function gatewayPortLabel(gateway: GatewayPortStatus | null): string {
  if (!gateway) return "读取中…";
  if (!gateway.enabled) return "未启用";
  // `activePort` 为 0 = 监听地址解析不出端口 = 此刻没有在服务。
  if (gateway.activePort <= 0) return "已启用，未监听";
  return `:${gateway.activePort}`;
}

/** 网关那一格的语义色调。 */
export function gatewayTone(gateway: GatewayPortStatus | null): "ok" | "warn" | "off" {
  if (!gateway) return "off";
  if (!gateway.enabled) return "off";
  return gateway.activePort > 0 ? "ok" : "warn";
}

/**
 * 「这个地址能一直用吗」的说明。
 *
 * # 为什么必须写出来
 *
 * 端口为 0 时地址**每次启动都不一样**。用户复制它去配外部客户端，
 * 如果以为它是固定的，下次重启就连不上了——而那种故障看起来像「客户端坏了」。
 *
 * 反过来，填了固定端口之后就真的是固定的，那时再吓得用户不敢用也不对。
 */
export function gatewayAddressHint(gateway: GatewayPortStatus | null): string {
  if (!gateway) return "";
  if (gateway.configuredPort === 0) {
    return "端口由系统自动分配，**每次启动都可能不同**。要长期给别的客户端用，请填一个固定端口。";
  }
  return `已固定在 ${gateway.configuredPort}，重启后不变。`;
}
