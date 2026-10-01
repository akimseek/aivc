import { useCallback, useState } from "react";

import { Badge, Button, Icon, IconButton, ToolbarSpacer } from "../../components";
import * as api from "../../ipc/commands";
import type { AiProviderSummary, AiSession, AiTarget, ApiVendor, GatewayPortStatus } from "../../types";
import { rangeBounds } from "../../app/gateway-range";
import { useAsyncData } from "../../components/use-async-data";
import { AttentionListCard } from "./attention-list-card";
import { RateLimitCard } from "./rate-limit-card";
import { RouteChangeCard } from "./route-change-card";
import { RuntimeCard } from "./runtime-card";
import { UsageCard } from "./usage-card";
import { getRateLimitWindows, type RouteChange } from "./workbench-cards";

/**
 * 工作台侧栏：**当前会话与 Gateway 的运行概览**。
 *
 * # 它与会话侧栏的关系
 *
 * 活动栏切的是「哪一处地方」——工作台与会话是两块**整块替换**的侧栏内容。
 * 因此它自带 `<aside className="sidebar workbench-pane">`（与会话侧栏同一套网格列，
 * 只宽 30px），而不是被谁包在一个壳里。
 *
 * # 为什么这个类名不叫 `workbench`
 *
 * `workbench` 已经被占用了：会话头里那三粒用量数字（`session-workspace.tsx`）带着它，
 * `app.css` 里那条 `.workbench { display:flex; gap:12px; color: var(--ui-muted);
 * font-size:12px; white-space:nowrap }` 是给那些数字写的。侧栏挂上同名类，那四条声明会
 * **继承进整棵工作台子树**：标题与数字变灰、卡片说明不再换行（长句被 `overflow: hidden`
 * 裁掉且没有省略号）、头部与滚动区之间多出一条 12px 的缝。所以侧栏另起一个名字。
 * 想要「宽 340px」那条规则的人，找 `.sidebar.workbench-pane`。
 *
 * # 滚动容器与内边距为什么是它自己的事（R24）
 *
 * 五张卡加起来远高于一屏，而 `.sidebar` 是 `overflow: hidden`。此前这块内容被
 * 直接塞进裸 `.sidebar` 里，窗口一矮卡片就被压缩、末两张根本滚不到。
 * 现在固定头部（标题 / 刷新 / 关闭），只让 `.workbench-body` 滚。
 *
 * # 为什么不额外轮询
 *
 * 旧平台的注释写得很明确：「工作台不为获取限额额外请求 CLI 或 Gateway」。
 * 速率限制只读会话摘要里已有的字段（`getRateLimitWindows`）；用量与异常则在
 * 「后端推送 `gateway:request-recorded` 时」刷新——那条事件的合并窗口
 * （`REQUEST_REFRESH_WINDOW_MS`，见 `app/refresh-coalescer.ts`）与计数器都在
 * `app.tsx`，这里只接收 `refreshToken`。
 */
export function WorkbenchSidebar({
  provider,
  target,
  session,
  gateway,
  vendors,
  lastSwitch,
  refreshToken,
  onClose,
  onOpenUsage,
  onOpenFailures
}: {
  provider: AiProviderSummary | undefined;
  target: AiTarget | null;
  session: AiSession | null;
  /** 网关状态。用**真实类型**而不是手抄的子集，理由见 `status-bar.tsx` 同一处注释。 */
  gateway: GatewayPortStatus | null;
  vendors: ApiVendor[];
  /** 最近一次供应商路由变更（来自终端状态）。 */
  lastSwitch?: RouteChange;
  /**
   * 变化时重新取数。
   *
   * 用一个**计数器**而不是把「刷新函数」传下来：外层只要在收到后端事件时
   * `setRefreshToken((n) => n + 1)` 即可，不需要知道工作台内部取了哪些接口。
   */
  refreshToken: number;
  /** 关闭按钮：回到会话侧栏（活动栏那个「会话」的同一个语义）。 */
  onClose: () => void;
  /** 「用量明细 ↗」：设置中心的用量报表页。 */
  onOpenUsage: () => void;
  /** 「查看详情」：设置中心的异常诊断页。 */
  onOpenFailures: () => void;
}) {
  const [manual, setManual] = useState(0);
  /**
   * 取数键。
   *
   * `refreshToken`（外部事件驱动）与 `manual`（用户点刷新）拼在一起作为
   * `useAsyncData` 的依赖——两者走的是**同一条**取数路径，不存在
   * 「事件刷新和手动刷新行为不一样」这种偏差。
   */
  const loadKey = `${refreshToken}:${manual}`;

  const load = useCallback(async () => {
    const { start, end } = rangeBounds("today");
    const [usage, failures] = await Promise.all([
      api.getUsageSummary(start, end).catch(() => null),
      api
        .getFailureDiagnostics({ page: 1, pageSize: 5 })
        .then((value) => value.items)
        .catch(() => [])
    ]);
    return { usage, failures };
    // `loadKey` 只用来让这个回调在键变化时换一个身份——它本身不参与取数。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loadKey]);

  const data = useAsyncData(load, { errorMessage: "工作台读取失败" });
  const rateWindows = getRateLimitWindows(session);

  return (
    <aside className="sidebar workbench-pane" aria-label="工作台侧栏">
      <div className="workbench-head">
        <div className="workbench-head-row">
          <Icon name="chart" size={16} />
          <h1 className="workbench-title">运行概览与工作台</h1>
          <ToolbarSpacer />
          <IconButton label="关闭工作台" title="关闭工作台，回到会话" onClick={onClose}>
            <Icon name="close" size={16} />
          </IconButton>
        </div>
        {/* 第二行：这一块的性质（跟着事件刷新）与手动刷新。挤在第一行会让标题
            那一行读起来像工具栏（会话侧栏的头部也是这个分法）。 */}
        <div className="workbench-head-row">
          <Badge tone="accent">实时监控</Badge>
          <ToolbarSpacer />
          <Button
            small
            onClick={() => setManual((value) => value + 1)}
            title="重新读取今日用量与最近异常"
          >
            <Icon name="refresh" size={12} />
            刷新
          </Button>
        </div>
      </div>

      <div className="workbench-body">
        <RuntimeCard provider={provider} target={target} session={session} gateway={gateway} />
        {/* 速率窗口缺席（多数平台都没有）时整块不渲染：一张空标题的卡只会让人以为功能坏了。 */}
        {rateWindows.length > 0 && <RateLimitCard windows={rateWindows} />}
        <RouteChangeCard change={lastSwitch} vendors={vendors} />
        <UsageCard usage={data.data?.usage ?? null} loading={data.loading} onOpenUsage={onOpenUsage} />
        <AttentionListCard
          failures={data.data?.failures ?? []}
          loading={data.loading}
          error={data.error}
          onOpenFailures={onOpenFailures}
        />
      </div>
    </aside>
  );
}
