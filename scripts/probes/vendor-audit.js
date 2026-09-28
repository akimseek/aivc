/**
 * 看库里到底有哪些供应商 —— 用户这次「没走网关」的根因就在这里。
 *
 * 「该平台没有可用的供应商」可能是：一家都没配 / 全被禁用 / API Key 为空 /
 * 平台不匹配。这四种在界面上给出的提示是同一句，所以只能把**原始记录**打出来。
 *
 * 顺带看一眼网关自身与设置：`enabled` 为假时也不会建路由。
 */
(async () => {
  const p = window.__p;
  const out = {};

  const safe = async (name, action) => {
    try {
      return await action();
    } catch (error) {
      out[`${name}Error`] = String((error && error.message) || error);
      return null;
    }
  };

  const vendors = await safe("vendors", () => p.invoke("vendor:list", {}));
  out.vendors = (vendors || []).map((item) => ({
    id: String(item.id).slice(0, 8),
    providerId: item.providerId,
    name: item.name,
    apiBaseUrl: item.apiBaseUrl,
    // **只报长度与是否存在**，绝不把密钥本身打出来。
    apiKeyLength: item.apiKey ? String(item.apiKey).length : 0,
    enabled: item.enabled,
    sort: item.sort
  }));
  out.vendorCount = out.vendors.length;

  const settings = await safe("settings", () => p.invoke("settings:get", {}));
  if (settings) {
    out.gateway = {
      enabled: settings.gatewayEnabled,
      port: settings.gatewayPort
    };
  }

  out.port = await safe("port", () => p.invoke("gateway:get-port", {}));
  out.external = await safe("external", () => p.invoke("gateway:get-external-api-status", {}));

  const usable = out.vendors.filter((item) => item.enabled && item.apiKeyLength > 0);
  out.usableCount = usable.length;
  out.usableByProvider = [...new Set(usable.map((item) => item.providerId))];

  out.verdict =
    out.usableCount === 0
      ? "根因：没有任何「已启用且有密钥」的供应商 —— 所以终端不会接入网关"
      : `有 ${out.usableCount} 家可用供应商（平台：${out.usableByProvider.join(", ")}）`;
  return out;
})();
