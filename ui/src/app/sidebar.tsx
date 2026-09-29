import { useState } from "react";

import {
  Button,
  ChipGroup,
  EmptyState,
  Field,
  List,
  Notice,
  SearchInput,
  Select,
  TextInput,
  Toolbar,
  ToolbarSpacer
} from "../components";
import type { AiProviderSummary, AiTarget, ApiVendor, GatewayPortStatus } from "../types";
import { emptySessionMessage } from "./session-display";
import { QuickGroup, SearchResultRow, SessionRow } from "./session-row";
import {
  paneForViewNav,
  showsSessionSearchRow,
  showsViewNav,
  supportsBatchActions,
  supportsTrash,
  viewNavKey,
  type ViewNavKey
} from "./sidebar-nav";
import { SidebarWorkbench } from "./sidebar-workbench";
import type { useConsole } from "./use-console";

/** 侧栏需要的那部分 console 状态与动作。用具名类型而不是 `any`，字段改名时会编译报错。 */
export type SidebarConsole = Pick<
  ReturnType<typeof useConsole>,
  | "busy"
  | "providers"
  | "providerId"
  | "selectProvider"
  | "targets"
  | "targetId"
  | "selectTarget"
  | "pane"
  | "setPane"
  | "view"
  | "setView"
  | "query"
  | "setQuery"
  | "searching"
  | "sessions"
  | "selectedId"
  | "selectedIds"
  | "toggleSelected"
  | "toggleSelectAll"
  | "toggleFavorite"
  | "reloadTargets"
  | "configureWslCodexHome"
  | "clearWslCodexHomeOverride"
  | "quickAccess"
  | "globalQuery"
  | "setGlobalQuery"
  | "globalSearching"
  | "searchTargetId"
  | "setSearchTargetId"
  | "globalResults"
  | "restoreSelected"
  | "purgeSelected"
  | "deleteSelected"
  /* 工作台要用：当前会话（含它自带的用量与速率限制）。 */
  | "selectedSession"
>;

/**
 * 侧栏。
 *
 * # 为什么把它整个抽出来
 *
 * `app.tsx` 里这一块有 **260 行内联 JSX**（平台/目标选择、目标状态说明、
 * 两级导航、三种列表、批量条、空状态）。它和「工作区」是两件事，
 * 混在一个文件里既读不下去，也没法单独测。
 *
 * 抽出来之后 `app.tsx` 只剩「谁在什么时候调什么」，而这一块可以按
 * 「给了这些状态，渲染出什么」来断言。
 *
 * # 三种 pane 的空状态各不相同
 *
 * 这一点必须显式写出来：`sessions` 的空要说清「没选目标 / 目标进不去 / 真没有」，
 * `quick` 的空要说「还没有收藏」，`search` 的空要说「输入关键词」。
 * 一句笼统的「没有数据」会让用户不知道下一步做什么。
 *
 * # 行点击只回传 `sessionId`
 *
 * 抽成 `onResume(target, sessionId)` 而不是把 `AiSession` 整个递出去：
 * 侧栏不关心会话内容，只关心「用户点了哪一条」。
 */
export function Sidebar({
  console: state,
  selectedTarget,
  onResume,
  onMenu,
  providers,
  vendors,
  gateway,
  lastSwitch,
  workbenchToken,
  collapsed = false,
  onToggleCollapsed
}: {
  console: SidebarConsole;
  selectedTarget: AiTarget | null;
  onResume: (target: AiTarget, sessionId: string) => void;
  onMenu: (x: number, y: number, sessionId: string) => void;
  providers: AiProviderSummary[];
  vendors: ApiVendor[];
  /**
   * 网关状态。
   *
   * 用**真实类型**而不是手抄一个子集：此前这里是
   * `{ enabled; activePort; configuredPort }`，抄漏了后端实际发的字段名，
   * 于是「字段名对不上」这件事被类型系统放过去了——两边都编译得过，
   * 运行时那一格永远显示错的。
   */
  gateway: GatewayPortStatus | null;
  /** 最近一次供应商路由变更，来自终端状态。 */
  lastSwitch?: { vendorId: string; reason: string; at: string };
  /** 变化时让工作台重新取数（后端推送请求记录时外层会加一）。 */
  workbenchToken: number;
  /** 折起来只留一条竖边。终端很宽时用得着。 */
  collapsed?: boolean;
  onToggleCollapsed?: () => void;
}) {
  const trash = state.view === "trash";
  const selectedCount = state.selectedIds.length;
  const quickCount = state.quickAccess.favorites.length + state.quickAccess.recent.length;
  const workbenchOpen = state.pane === "workbench";
  const provider = providers.find((item) => item.id === state.providerId);
  /**
   * 当前平台支不支持回收站与批量操作。
   *
   * 这两个判断**必须走能力矩阵**，不能写死：Qoder 不支持回收站
   * （`ProviderCapabilities::trash == false`），写死的话它的用户会看到一个
   * 点下去必然报错的「回收站」。
   */
  const canUseTrash = supportsTrash(providers, state.providerId);
  const canBatch = supportsBatchActions(providers, state.providerId);

  /**
   * 折起来时**只保留那个按钮**。
   *
   * 折叠的意义就是把宽度让给终端，留一半内容等于没折。但按钮本身必须留着——
   * 折起来之后没有别的入口能把它打开，那就是一条死路。
   */
  if (collapsed) {
    return (
      <aside className="sidebar collapsed" aria-label="会话侧栏（已折叠）">
        <Button small onClick={onToggleCollapsed} title="展开侧栏" aria-label="展开侧栏">
          »
        </Button>
      </aside>
    );
  }

  return (
    <aside className="sidebar">
      <div className="sidebar-head">
        <div className="title">
          <span className={`status-dot${state.busy ? " off" : ""}`} />
          <h1>会话</h1>
        </div>
        {/* 折起来把宽度让给终端。**必须留在标题行里**：折起来之后
            没有别的入口能把它打开。 */}
        {onToggleCollapsed && (
          <Button small onClick={onToggleCollapsed} title="折叠侧栏" aria-label="折叠侧栏">
            «
          </Button>
        )}
      </div>

      <div className="pickers">
        <Field label="平台">
          <Select
            value={state.providerId}
            onChange={(event) => state.selectProvider(event.target.value as never)}
          >
            {state.providers.length === 0 && <option value="">加载中…</option>}
            {state.providers.map((provider) => (
              <option key={provider.id} value={provider.id}>
                {provider.label}
              </option>
            ))}
          </Select>
        </Field>

        <Field label="目标">
          <Select
            value={state.targetId}
            disabled={state.targets.length === 0}
            onChange={(event) => state.selectTarget(event.target.value)}
          >
            {state.targets.length === 0 && <option value="">暂无可用目标</option>}
            {state.targets.map((target) => (
              <option key={target.id} value={target.id}>
                {/* 不可用的目标在选择列表里就标出来——等选中了才发现，用户已经先点了一次。 */}
                {target.available ? target.label : `${target.label}（不可用）`}
              </option>
            ))}
          </Select>
        </Field>

        {/* 目标状态说明。后端给出了四态 detail，界面必须显示：
            否则用户只看到「列表是空的」，分不清是**没有会话**还是**这个目标进不去**。 */}
        {selectedTarget && (
          <div className={`target-detail${selectedTarget.available ? "" : " bad"}`}>
            <span title={selectedTarget.detail ?? ""}>
              {selectedTarget.detail ?? "（无状态说明）"}
            </span>
          </div>
        )}
      </div>

      {selectedTarget && !selectedTarget.available && (
        <div className="sidebar-notice">
          <Notice
            message={selectedTarget.detail || "此目标当前不可用。"}
            actions={
              <>
                <Button small onClick={() => void state.reloadTargets()}>
                  重新探测
                </Button>
                {selectedTarget.kind === "wsl" && selectedTarget.distro && (
                  <WslHomeForm
                    distro={selectedTarget.distro}
                    onSave={(home) =>
                      void state.configureWslCodexHome(selectedTarget.distro!, home)
                    }
                    onClear={() => void state.clearWslCodexHomeOverride(selectedTarget.distro!)}
                  />
                )}
              </>
            }
          />
        </div>
      )}

      <div className="list-head">
        <Toolbar>
          {/* 一级导航。**包一层**是为了在 CSS 里和二级拉开轻重——两级用的是同一个
              `ChipGroup`，不区分的话看上去完全一样，用户分不出哪层是哪层。

              一级只有两个：**工作台 / 会话**。收藏夹与全局搜索此前是这里的一级项，
              但旧平台把它们放在下一层（收藏夹是二级筛选、「全部会话」是搜索行里的按钮），
              摆成一级会让人以为它们是另外两处地方。 */}
          <div className="pane-nav">
            <ChipGroup
              value={state.pane === "workbench" ? "workbench" : "sessions"}
              onChange={(value) => {
                if (value === "workbench") {
                  state.setPane("workbench");
                  return;
                }
                // 点「会话」回到当前会话列表：从收藏夹/回收站回来时这是最可能想去的地方。
                state.setPane("sessions");
                state.setView("active");
              }}
              options={[
                { value: "workbench", label: "工作台" },
                { value: "sessions", label: "会话" }
              ]}
            />
          </div>
          <ToolbarSpacer />
          <span className="row-meta">
            {workbenchOpen
              ? "概览"
              : state.pane === "sessions"
                ? `${state.sessions.length} 条`
                : state.pane === "favorites"
                  ? `${quickCount} 条`
                  : `${state.globalResults.length} 条`}
          </span>
        </Toolbar>

        {showsViewNav(state.pane) && (
          <Toolbar>
            {/* 二级视图。比一级轻一档：它是**同一批内容的筛选**，不是另一处地方。 */}
            <div className="view-nav">
              <ChipGroup
                value={viewNavKey(state.pane, state.view)}
                onChange={(value) => {
                  const next = paneForViewNav(value as ViewNavKey);
                  state.setPane(next.pane);
                  state.setView(next.view);
                }}
                options={[
                  { value: "active", label: "当前会话" },
                  { value: "favorites", label: "收藏夹" },
                  {
                    value: "trash",
                    label: "回收站",
                    // 平台不支持回收站时**禁用并说明原因**，而不是给一个点了会报错的项。
                    disabled: !canUseTrash,
                    title: canUseTrash ? "查看回收站" : "请选择支持回收站的平台"
                  }
                ]}
              />
            </div>
          </Toolbar>
        )}

        {showsSessionSearchRow(state.pane) && (
          <div className="session-search-row">
            <SearchInput
              value={state.query}
              onChange={state.setQuery}
              placeholder="搜索当前平台会话"
              busy={state.searching}
            />
            {/* 全局搜索是**一个按钮**，不是一级导航里的一处地方：
                它是「在当前这个列表里找不到，去别处找」这个动作。 */}
            <Button small onClick={() => state.setPane("search")}>
              全部会话
            </Button>
          </div>
        )}

        {state.pane === "search" && (
          <>
            <SearchInput
              value={state.globalQuery}
              onChange={state.setGlobalQuery}
              placeholder="在所有目标里搜索…"
              busy={state.globalSearching}
            />
            {/* 范围收窄：不只是少显示结果，而是**少发请求**——
                全量搜完再在前端过滤掉，那些请求已经付过代价了。 */}
            <Field label="搜索范围">
              <Select
                value={state.searchTargetId}
                onChange={(event) => state.setSearchTargetId(event.target.value)}
              >
                <option value="">全部目标（{state.targets.length}）</option>
                {state.targets.map((target) => (
                  <option key={target.id} value={target.id}>
                    {target.available ? target.label : `${target.label}（不可用）`}
                  </option>
                ))}
              </Select>
            </Field>
          </>
        )}
      </div>

      {/* 批量条只在**平台支持批量操作**且有勾选时出现。
          不支持批量操作的平台（例如 Qoder）上，勾选框本身也不渲染——
          那条规则在会话行那边，由 `selectable` 控制。 */}
      {state.pane === "sessions" && canBatch && selectedCount > 0 && (
        <div className="batchbar">
          <span>已选 {selectedCount} 项</span>
          <ToolbarSpacer />
          <Button small onClick={state.toggleSelectAll}>
            全选
          </Button>
          {trash ? (
            <>
              <Button small onClick={() => void state.restoreSelected()}>
                恢复
              </Button>
              <Button small variant="danger" onClick={() => void state.purgeSelected()}>
                彻底删除
              </Button>
            </>
          ) : (
            <Button small variant="danger" onClick={() => void state.deleteSelected()}>
              移入回收站
            </Button>
          )}
        </div>
      )}

      <List
        empty={
          <>
            {state.pane === "sessions" && state.sessions.length === 0 && (
              <EmptyState
                message={
                  state.query.trim()
                    ? "没有匹配的会话。"
                    : emptySessionMessage(state.targetId, selectedTarget, trash)
                }
              />
            )}
            {state.pane === "search" && state.globalResults.length === 0 && (
              <EmptyState
                message={
                  state.globalQuery.trim()
                    ? "没有匹配的会话。"
                    : "输入关键词，在所有目标里搜索。"
                }
                hint={
                  state.globalQuery.trim()
                    ? "后端会先匹配摘要，摘要不中再读整个文件——因此结果可能稍慢。"
                    : undefined
                }
              />
            )}
          </>
        }
      >
        {/* 工作台：**不占列表区**，它整块替换掉列表。
            它是「现在怎么样」的概览，与会话列表是两种东西——
            两者叠在一起会让人分不清哪些是概览、哪些是可点的会话。 */}
        {workbenchOpen && (
          <SidebarWorkbench
            provider={provider}
            target={selectedTarget}
            session={state.selectedSession}
            gateway={gateway}
            vendors={vendors}
            lastSwitch={lastSwitch}
            refreshToken={workbenchToken}
          />
        )}

        {state.pane === "sessions" &&
          state.sessions.map((session) => (
            <SessionRow
              key={session.id}
              session={session}
              active={session.id === state.selectedId}
              checked={state.selectedIds.includes(session.id)}
              selectable={canBatch}
              onOpen={() => {
                if (selectedTarget) onResume(selectedTarget, session.id);
              }}
              onToggle={() => state.toggleSelected(session.id)}
              onFavorite={() => void state.toggleFavorite(session)}
              onMenu={(x, y) => onMenu(x, y, session.id)}
            />
          ))}

        {state.pane === "favorites" && (
          <>
            <QuickGroup
              title="收藏"
              items={state.quickAccess.favorites}
              empty="还没有收藏。在会话行上点 ☆ 收藏它。"
              onOpen={(target, session) => onResume(target, session.id)}
            />
            <QuickGroup
              title="最近打开"
              items={state.quickAccess.recent}
              empty="还没有打开过会话。"
              onOpen={(target, session) => onResume(target, session.id)}
            />
          </>
        )}

        {state.pane === "search" &&
          state.globalResults.map((result) => (
            <SearchResultRow
              key={`${result.target.id}:${result.session.id}`}
              target={result.target}
              session={result.session}
              onOpen={() => onResume(result.target, result.session.id)}
            />
          ))}
      </List>
    </aside>
  );
}

/**
 * 手动指定 WSL 内的 Codex 目录。
 *
 * # 为什么它必须出现在「目标不可用」的告警里
 *
 * 发行版进不去时，自动探测**注定失败**；而用户往往知道自己的 Codex 装在哪。
 * 这条路径此前只藏在菜单里（甚至压根没有入口），于是「目标不可用」变成了死路。
 *
 * 保存后必须**重新探测**——否则界面还停在旧结果上，用户会以为没生效。
 */
function WslHomeForm({
  distro,
  onSave,
  onClear
}: {
  distro: string;
  onSave: (codexHome: string) => void;
  onClear: () => void;
}) {
  const [draft, setDraft] = useState("");
  return (
    <div className="wsl-home">
      <TextInput
        value={draft}
        placeholder={`${distro} 内的路径，例如 /home/你/.codex`}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && draft.trim()) onSave(draft.trim());
        }}
      />
      <div className="wsl-home-actions">
        <Button small disabled={!draft.trim()} onClick={() => onSave(draft.trim())}>
          指定并重新探测
        </Button>
        <Button small onClick={onClear} title="清除人工指定，回到自动探测">
          清除
        </Button>
      </div>
    </div>
  );
}
