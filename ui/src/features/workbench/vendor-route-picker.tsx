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
 *
 * # 为什么改成浮层
 *
 * 状态栏只有 28px，原生 `<select>` 在那里的观感与整体格格不入，而且它装不下
 * 「锁定」「健康」「已停用」这些要一起看的信息。浮层（`Popover`）能一次说清：
 * 当前是谁、还有哪些候选、各自什么状态、锁没锁。
 */
import { useCallback, useEffect, useRef, useState } from "react";

import { Badge, Icon, Popover, StatusDot, errorText, type StatusDotTone } from "../../components";
import * as api from "../../ipc/commands";
import type { AiProviderId, ApiVendor, GatewayVendorHealth } from "../../types";
import type { TerminalSession, VendorRouteMode } from "../../app/use-terminals";

/** 健康快照里的状态 → 界面文案。`healthy` 与未知值都不标注（见 `healthBadge`）。 */
function healthBadge(status: string): { label: string; tone: StatusDotTone } | null {
  if (status === "open") return { label: "熔断中", tone: "danger" };
  if (status === "degraded" || status === "half-open") return { label: "异常", tone: "warn" };
  return null;
}

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
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [modeError, setModeError] = useState("");
  /**
   * 供应商健康快照。
   *
   * `null` = **没取到**（或还没取）→ 界面上不出现任何健康信息。它是可选增强：
   * 后端读不到健康表时（那张表在某些版本里恒为空），浮层照常能切换供应商。
   */
  const [health, setHealth] = useState<GatewayVendorHealth[] | null>(null);
  /** 浮层的锚。挂在触发按钮上，由 `Popover` 自己算不越界的位置。 */
  const anchorRef = useRef<HTMLButtonElement | null>(null);

  const provider = terminal?.providerId ?? "";
  const terminalId = terminal?.id ?? "";
  const vendorId = terminal?.vendorId;
  const mode = terminal?.vendorMode ?? "dynamic";

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

  /* 打开浮层时取一次健康快照。
   *
   * 取不到就**静默省略**：健康是锦上添花，为它弹一条错误会把「切换供应商」
   * 这件正事盖掉。关掉时清空，免得下次打开先闪一眼上一次的旧快照。 */
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setHealth(null);
    void api
      .getVendorHealth()
      .then((list) => {
        if (!cancelled) setHealth(list);
      })
      .catch(() => {
        if (!cancelled) setHealth(null);
      });
    return () => {
      cancelled = true;
      setHealth(null);
    };
  }, [open]);

  const change = useCallback(
    async (vendorId: string) => {
      if (!terminalId) return;
      setBusy(true);
      setError("");
      try {
        await onSwitch(terminalId, vendorId);
        // 成功才收起：失败时浮层要留在原地把事情说清楚。
        setOpen(false);
      } catch (cause) {
        setError(errorText(cause));
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
        setModeError(errorText(cause));
      }
    },
    [onSetMode, terminalId]
  );

  if (!terminal) return null;
  // 终端都不知道自己属于哪个平台时，供应商列表无从谈起。
  if (!provider) return null;

  // 没有绑定：说明后端没有可用供应商（没配、被禁用、或配置不全），
  // 于是**这一次终端根本没走网关**，CLI 会用它自己的登录 / 配置文件直连上游。
  //
  // **这一条必须显眼。** 用户真的撞上过：界面上写着「直连」，他就以为一切正常，
  // 而实际上请求去的是 `~/.codex/config.toml` 里那个地址、用的是他自己的密钥，
  // 直到余额耗尽看到 403 才发现。所以这里不能只是一个中性的状态词。
  if (!vendorId) {
    return (
      <span
        className="status-vendor warn"
        title="没有可用供应商，本次终端没有接入网关：CLI 会直连它自己配置的上游，并使用你自己的凭据"
      >
        <Icon name="alert" size={12} />
        未走网关 · <b>直连</b>（用自己的密钥）
      </span>
    );
  }

  const nameOf = (id: string) => vendors.find((vendor) => vendor.id === id)?.name ?? id;
  const currentName = nameOf(vendorId);

  /**
   * 列表里要有哪些供应商。
   *
   * 候选池里的全部 + **当前绑定的那一家**（它可能刚被禁用）。**绑定不在列表里**时
   * （被删掉了）合成一条，名字退回编号——界面必须让用户看见它已经不在了，
   * 而不是悄悄跳到另一家候选上（那会让用户以为绑定还在）。
   */
  const listed: ApiVendor[] = vendors.some((vendor) => vendor.id === vendorId)
    ? vendors.filter((vendor) => vendor.enabled || vendor.id === vendorId)
    : [
        {
          id: vendorId,
          /*
            平台收窄在这一处完成：`TerminalSession.providerId` 是 `string`
            （终端只知道自己从后端拿到的编号字符串），而这条合成记录要落进
            `ApiVendor`。上面的 `if (!provider) return null` 已经排除了空串，
            因此这个转换在这里是安全的。
          */
          providerId: provider as AiProviderId,
          name: vendorId,
          apiBaseUrl: "",
          sort: 0,
          enabled: false
        },
        ...vendors.filter((vendor) => vendor.enabled)
      ];

  /** 健康快照给某一家供应商的标注；没数据就是 `null`（不显示）。 */
  const badgeFor = (id: string) => {
    const snapshot = health?.find((item) => item.vendorId === id && item.providerId === provider);
    return snapshot ? healthBadge(snapshot.status) : null;
  };

  return (
    <span className="status-vendor">
      <button
        type="button"
        ref={anchorRef}
        className={`vendor-trigger${open ? " on" : ""}`}
        aria-haspopup="dialog"
        aria-expanded={open}
        title={error || "切换本终端使用的供应商"}
        onClick={() => setOpen((value) => !value)}
      >
        供应商 <b>{currentName}</b>
        {/* 锁定时给一个徽章：锁定与动态的行为差别很大（不会自动故障转移），
            只在浮层里说得清，但状态栏上也要看得出来。 */}
        {terminal.vendorMode === "locked" && <Badge tone="warn">锁定</Badge>}
        <Icon name="chevron-down" size={12} />
      </button>

      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={anchorRef}
        className="vendor-popover"
        // 面板是 `role="dialog"`，名字与触发按钮上的文字一致：读屏是从按钮跳进来的。
        ariaLabel="切换供应商"
      >
        <div className="vendor-pop-head">
          <strong>{currentName}</strong>
          {/* 健康快照取到了 = 网关回答了这次请求，于是可以说「在线」。 */}
          {health && (
            <span className="vendor-pop-net">
              <StatusDot tone="ok" pulse size={7} />
              网关在线
            </span>
          )}
        </div>

        <ul className="vendor-pop-list">
          {listed.map((vendor) => {
            const active = vendor.id === vendorId;
            const bad = badgeFor(vendor.id);
            return (
              <li key={vendor.id}>
                <button
                  type="button"
                  className={`vendor-option${active ? " on" : ""}`}
                  disabled={busy}
                  title={active ? "当前供应商" : `切换到 ${vendor.name}`}
                  onClick={() => void change(vendor.id)}
                >
                  <StatusDot tone={bad ? bad.tone : active ? "ok" : "idle"} />
                  <span className="vendor-option-name">{vendor.name}</span>
                  {!vendor.enabled && <Badge tone="neutral">已停用</Badge>}
                  {/* 当前项与其余项的差别要一眼看出来：名字旁边写「活跃」而不是靠加粗。 */}
                  <Badge tone={active ? "accent" : "neutral"}>{active ? "活跃" : "候选"}</Badge>
                  {bad && <Badge tone="warn">{bad.label}</Badge>}
                </button>
              </li>
            );
          })}
        </ul>

        {/* 锁定开关是个复选框而不是按钮：它表达的是**一个开关状态**，
            而按钮点了之后界面上不会留下「现在是锁的」这个事实。 */}
        <label className="vendor-lock" title="锁定之后本终端不会被自动换到其他供应商">
          <input
            type="checkbox"
            checked={mode === "locked"}
            disabled={busy}
            onChange={(event) => void changeMode(event.target.checked ? "locked" : "dynamic")}
          />
          <span>锁定此供应商（禁用故障自动切换）</span>
        </label>

        {modeError && <span className="err">{modeError}</span>}
      </Popover>

      {/* 自动切换必须显式标出来：用户没有做任何操作，界面却换了一家，
          不解释的话看起来像 bug。 */}
      {terminal.lastSwitch?.reason === "failure" && (
        <span className="vendor-auto" title="上游请求失败，已自动切换到这家">
          已自动切到 {nameOf(terminal.lastSwitch.vendorId)}
        </span>
      )}
      {error && <span className="err">{error}</span>}
    </span>
  );
}
