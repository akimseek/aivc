/**
 * 状态栏里的「当前供应商」。
 *
 * # 这个组件要回答的问题是「我现在花的是谁的钱」
 *
 * 一路终端一旦走网关，它实际服务的供应商就**不由用户直接控制**了：可以手动切换，
 * 也可以在请求失败时被自动换掉。用户界面上必须能随时看到**当前是哪一家**，
 * 否则只会在账单上发现不对。
 *
 * # 为什么启动时不在这里挑
 *
 * 与老平台一致：启动时界面**不指定**供应商，由后端按候选池选一家并返回。原因是界面
 * 并不知道哪家的 Key 有效、哪家被禁用——让它猜只会显示一个不存在的绑定。
 * 这里做的是**之后**的事：显示当前值，并允许手动改。
 *
 * # 切换为什么不先改本地状态
 *
 * 切换会因为「供应商被禁用 / 配置不全 / 平台不匹配」而失败。先改再等失败回滚，
 * 中间那一帧界面就在撒谎。因此只等后端确认（`vendor:route-switch` 成功），
 * 而真实的新值由 `gateway:vendor-switched` 事件推回来——那条路径同时也是**自动
 * 故障转移**的路径，所以手动与自动走的是同一段代码。
 */
import { useCallback, useEffect, useState } from "react";

import * as api from "../ipc/commands";
import type { ApiVendor } from "../types";
import type { TerminalSession, VendorRouteMode } from "./use-terminals";

/** 一辆终端的供应商绑定状态。 */
export function VendorRoutePicker({
  terminal,
  onSwitch,
  onSetMode
}: {
  terminal: TerminalSession | undefined;
  onSwitch: (terminalId: string, vendorId: string) => Promise<void>;
  onSetMode: (terminalId: string, mode: VendorRouteMode) => Promise<void>;
}) {
  const [vendors, setVendors] = useState<ApiVendor[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [modeError, setModeError] = useState("");

  const provider = terminal?.providerId ?? "";
  const terminalId = terminal?.id ?? "";

  /* 取该平台的供应商。
   *
   * **即使当前没有绑定也要取**：没有绑定时界面需要能说清「为什么没有」——
   * 是因为这一家都没配，还是因为都被禁用了。一个空的选择器说不出这件事。 */
  useEffect(() => {
    if (!provider) {
      setVendors([]);
      return;
    }
    let cancelled = false;
    void api
      .listVendors(provider)
      .then((list) => {
        if (!cancelled) setVendors(list);
      })
      .catch(() => {
        if (!cancelled) setVendors([]);
      });
    return () => {
      cancelled = true;
    };
  }, [provider]);

  const change = useCallback(
    async (vendorId: string) => {
      if (!terminalId) return;
      setBusy(true);
      setError("");
      try {
        await onSwitch(terminalId, vendorId);
      } catch (cause) {
        setError(cause instanceof Error ? cause.message : String(cause));
      } finally {
        setBusy(false);
      }
    },
    [onSwitch, terminalId]
  );

  const changeMode = useCallback(
    async (mode: VendorRouteMode) => {
      if (!terminalId) return;
      setModeError("");
      try {
        await onSetMode(terminalId, mode);
      } catch (cause) {
        setModeError(cause instanceof Error ? cause.message : String(cause));
      }
    },
    [onSetMode, terminalId]
  );

  if (!terminal) return null;
  // 系统终端不注入环境、也不绑路由，没有供应商可言。
  if (!provider) return null;

  const eligible = vendors.filter((vendor) => vendor.enabled);
  const nameOf = (id: string) => vendors.find((vendor) => vendor.id === id)?.name ?? id;

  // 没有绑定：说明后端没有可用供应商（没配、被禁用、或配置不全），
  // 于是**这一次终端根本没走网关**，CLI 会用它自己的登录 / 配置文件直连上游。
  //
  // **这一条必须显眼。** 用户真的撞上过：界面上写着「直连」，他就以为一切正常，
  // 而实际上请求去的是 `~/.codex/config.toml` 里那个地址、用的是他自己的密钥，
  // 直到余额耗尽看到 403 才发现。所以这里不能只是一个中性的状态词。
  if (!terminal.vendorId) {
    return (
      <span
        className="status-vendor warn"
        title="没有可用供应商，本次终端没有接入网关：CLI 会直连它自己配置的上游，并使用你自己的凭据"
      >
        ⚠ 未走网关 · <b>直连</b>（用自己的密钥）
      </span>
    );
  }

  const switched = terminal.lastSwitch;
  const switchedName = switched ? nameOf(switched.vendorId) : "";

  return (
    <span className="status-vendor">
      供应商{" "}
      <select
        className="vendor-pick"
        value={terminal.vendorId}
        disabled={busy}
        title={error || "切换本终端使用的供应商"}
        onChange={(event) => void change(event.target.value)}
      >
        {/* 当前绑定不在候选池里时也要能显示出来：
            供应商可能刚刚被禁用或被删掉，此时**必须让用户看见它已经不在了**，
            而不是让选择器悄悄跳到另一家。 */}
        {!eligible.some((vendor) => vendor.id === terminal.vendorId) && (
          <option value={terminal.vendorId}>{nameOf(terminal.vendorId)}（已停用）</option>
        )}
        {eligible.map((vendor) => (
          <option key={vendor.id} value={vendor.id}>
            {vendor.name}
          </option>
        ))}
      </select>
      {/* 路由模式：锁定之后不会被自动换掉。用户为某台终端特意选了一家时用它。 */}
      <button
        type="button"
        className="vendor-mode"
        title={
          modeError ||
          (terminal.vendorMode === "locked"
            ? "已锁定：不会自动故障转移，点一下改回动态"
            : "动态：请求失败时会自动换到其他供应商，点一下锁定")
        }
        onClick={() => void changeMode(terminal.vendorMode === "locked" ? "dynamic" : "locked")}
      >
        {terminal.vendorMode === "locked" ? "锁定" : "动态"}
      </button>
      {/* 自动切换必须显式标出来：用户没有做任何操作，界面却换了一家，
          不解释的话看起来像 bug。 */}
      {switched?.reason === "failure" && switchedName && (
        <span className="vendor-auto" title="上游请求失败，已自动切换到这家">
          已自动切到 {switchedName}
        </span>
      )}
      {error && <span className="err">{error}</span>}
    </span>
  );
}
