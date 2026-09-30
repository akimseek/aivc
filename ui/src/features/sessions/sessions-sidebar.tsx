import {
  Button,
  ChipGroup,
  EmptyState,
  Icon,
  List,
  Notice,
  SearchInput,
  Select,
  StatusDot,
  ToolbarSpacer
} from "../../components";
import type {
  AiProviderId,
  AiProviderSummary,
  AiSession,
  AiTarget,
  GatewayPortStatus,
  SessionQuickAccessItem
} from "../../types";
import { gatewayPortLabel, gatewayTone } from "../../app/gateway-address";
import { supportsSessionSettings } from "../../app/provider-capabilities";
import { emptySessionMessage } from "../../app/session-display";
import { QuickGroup, SearchResultRow } from "../../app/session-row";
import { supportsBatchActions, supportsTrash } from "../../app/sidebar-nav";
import type { useConsole } from "../../app/use-console";
import { resolveSidebarListMode } from "./sidebar-search";
import { SessionCard } from "./session-card";
import { WslHomeForm } from "./wsl-home-form";

/** 侧栏需要的那部分 console 状态与动作。用具名类型而不是 `any`，字段改名时会编译报错。 */
export type SessionsSidebarConsole = Pick<
  ReturnType<typeof useConsole>,
  | "providers"
  | "providerId"
  | "selectProvider"
  | "targets"
  | "targetId"
  | "selectTarget"
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
  | "reloadSessions"
  | "configureWslCodexHome"
  | "clearWslCodexHomeOverride"
  | "quickAccess"
  | "globalQuery"
  | "setGlobalQuery"
  | "globalSearching"
  | "searchTargetId"
  | "setSearchTargetId"
  | "searchProviderId"
  | "setSearchProviderId"
  | "globalResults"
  | "restoreSelected"
  | "purgeSelected"
  | "deleteSelected"
>;

/** 没有终端在跑的会话集合。模块级常量：每次渲染新造一个 `Set` 会让卡片白白重渲染。 */
const NO_RUNNING_SESSIONS: ReadonlySet<string> = new Set();

/**
 * 收藏夹里的**本地**筛选：在「收藏」与「最近打开」两组里按关键词筛。
 *
 * # 它只能筛 `quickAccess` 这个本地列表
 *
 * **不要拿它去重算后端的搜索结果**。曾经有过一个「搜索消息正文」开关，关掉时
 * 用它把跨目标搜索结果再筛一遍「只有摘要命中的」——那是恒真的：后端搜索的第二阶段
 * 命中正文时返回的是**完整解析结果**，命中词必然出现在返回的 `preview` 里，
 * 所以按同一批字段重算必然也命中。在「不得新增后端命令」的前提下，客户端没有
 * 办法诚实区分「摘要命中」与「正文命中」，那个开关已删除（控制器裁定 R23）。
 *
 * 这里剩下的是另一件事：用户站在收藏夹里打字，是在**眼前这两组**里找一条会话，
 * 没命中就是没命中——与服务端搜过什么无关。
 *
 * 语义按空白分词、全部词都要命中（AND），字段集也与后端一致
 * （`aivc-core/src/text.rs` 的 `session_search_haystack`：编号、标题、来源标题、
 * 工作目录、模型、**CLI 版本**、全部预览正文）。CLI 版本这一项曾经漏掉：
 * 用户照着报错里的 `0.154.0` 去找那条会话时，本地筛选会把它判成「没命中」。
 */
function matchesQuickQuery(session: AiSession, query: string): boolean {
  const terms = query
    .trim()
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean);
  if (terms.length === 0) return true;
  const haystack = [
    session.id,
    session.title,
    session.sourceTitle ?? "",
    session.cwd ?? "",
    session.model ?? "",
    session.cliVersion ?? "",
    ...session.preview.map((message) => message.text)
  ]
    .join("\n")
    .toLowerCase();
  return terms.every((term) => haystack.includes(term));
}

/**
 * 会话侧栏。
 *
 * # 它现在只管「会话」这一件事
 *
 * 「工作台 / 会话」是两块**整块替换**的侧栏内容，由活动栏切换（`pane`）：
 * 工作台那一块在 `app.tsx` 里就地替换掉本组件（见那边的侧栏槽位）。
 * 侧栏内部只剩「同一批内容的筛选」（chips）。此前一级、二级、搜索各占一行，
 * 顶掉了列表三行的位置——列表是这里唯一被反复读的东西。
 *
 * 自上而下：头部（网关胶囊 / 新建 / 折叠）→ 平台与目标 → 目标不可用告警 →
 * 搜索 → 过滤 chips → 列表区 → 批量条 → 底部摘要。
 *
 * # 单击与双击是两件事
 *
 * 单击 `onOpen` = 打开/激活标签（终端没在跑就显示历史）；**双击** `onResume` =
 * 在终端中恢复。旧侧栏单击就起终端，那正是用户抱怨的「既慢又意外」——
 * 看一眼不该付一个进程的代价。恢复仍然显式（双击、或标签栏的「继续会话」），
 * 而且这条会话的终端正在跑时双击不接任何动作。
 */
export function SessionsSidebar({
  console: state,
  selectedTarget,
  onOpen,
  onResume,
  onMenu,
  onNewSession,
  onDetail,
  onOpenGateway,
  providers,
  gateway,
  runningSessionIds = NO_RUNNING_SESSIONS,
  collapsed = false,
  onToggleCollapsed
}: {
  console: SessionsSidebarConsole;
  selectedTarget: AiTarget | null;
  /** 打开一条会话的标签（历史）。**只回传编号**：侧栏不持有会话内容。 */
  onOpen: (target: AiTarget, sessionId: string) => void;
  /** 在终端中恢复这条会话（双击卡片）。与标签栏的「继续会话」同一个动作。 */
  onResume: (target: AiTarget, sessionId: string) => void;
  /** 右键菜单：坐标由侧栏回传，菜单开着鼠标的位置。 */
  onMenu: (x: number, y: number, sessionId: string) => void;
  /** 「新建会话」：打开新会话对话框（与工作区那个 +Codex 同一个入口）。 */
  onNewSession: () => void;
  /** 「消息与分支」：打开会话详情（消息列表与分支关系都在那里）。 */
  onDetail: (session: AiSession) => void;
  /** 网关胶囊：打开设置中心的**网关页**——端口、启停、为什么没起来都在那里。 */
  onOpenGateway: () => void;
  providers: AiProviderSummary[];
  /**
   * 网关状态。
   *
   * 用**真实类型**而不是手抄一个子集：此前这里是
   * `{ enabled; activePort; configuredPort }`，抄漏了后端实际发的字段名，
   * 于是「字段名对不上」这件事被类型系统放过去了——两边都编译得过，
   * 运行时那一格永远显示错的。
   */
  gateway: GatewayPortStatus | null;
  /** 有终端在跑的会话编号：卡片左上角那个点按它亮起来。 */
  runningSessionIds?: ReadonlySet<string>;
  /** 折起来把宽度全让给终端。展开的入口在活动栏（见 `app.tsx`）。 */
  collapsed?: boolean;
  onToggleCollapsed?: () => void;
}) {
  const trash = state.view === "trash";
  const selectedCount = state.selectedIds.length;
  /**
   * 「★ 收藏 N」的 N **只数收藏**（裁定 R22）。
   *
   * 它此前是 `favorites.length + recent.length`——那个数字与 chip 上的字对不上，
   * 而且用户心里「收藏 N」说的就是收藏夹里星标过的条数。「最近打开」是另一组数据
   * （收藏夹视图里仍然两组都显示，底部摘要按那一批算）。
   */
  const favoriteCount = state.quickAccess.favorites.length;
  /** 收藏夹视图里眼前这批的总数：两组都列出来，摘要说的是它们之和。 */
  const quickCount = favoriteCount + state.quickAccess.recent.length;
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
   * 当前平台支不支持会话设置（WSL 的 `CODEX_HOME` 覆盖）。
   *
   * 与设置中心环境页「WSL 会话目录」卡**同一判据**（都走 `supportsSessionSettings`）：
   * 保存/清除走 `codex:set-wsl-codex-home`，写的是 Codex 的**全局**
   * `wsl_codex_homes` 映射——对 claude/qoder 放行，就是拿别的平台的目标去改
   * Codex 的配置（跨平台数据污染），而且保存点了必然失败。
   */
  const canSessionSettings = supportsSessionSettings(providers, state.providerId);
  /** 新建会话要有一个能进去的目标——与工作区那个按钮同一条判据（`canStartCodex`）。 */
  const canStart = Boolean(selectedTarget?.available);
  /** 网关胶囊的状态色（`off` 落到状态点的 `idle`）。 */
  const gwTone = gatewayTone(gateway);

  /** 列表区该显示什么：全局 > 收藏夹 > 过滤 > 普通列表（顺序即理由，见 `sidebar-search.ts`）。 */
  const mode = resolveSidebarListMode({
    view: state.view,
    query: state.query,
    globalSearching: state.globalSearching,
    globalQuery: state.globalQuery
  });
  const global = mode.kind === "global";

  /**
   * 「按目标筛选」里该列哪些目标。
   *
   * 选了平台之后只列那个平台的目标——这正是「先按平台再按目标」的意义，
   * 否则第二个下拉里全是别的平台的目标，挑起来和没有筛选一样。
   */
  const scopedTargets = state.searchProviderId
    ? state.targets.filter((target) => target.provider === state.searchProviderId)
    : state.targets;

  /** 收藏夹里的本地筛选：在收藏里打字就是在**这两组里**筛。 */
  const filterQuick = (items: SessionQuickAccessItem[]) =>
    state.query.trim()
      ? items.filter((item) => matchesQuickQuery(item.session, state.query))
      : items;

  /**
   * 平台下拉的取值防御。
   *
   * 下拉里有两种**不是平台**的项：占位项（`""`）与加载提示。合法的取值只有
   * 前者的空值，以及**清单里真实存在**的编号——加载提示那一项虽然已 `disabled`，
   * 也不该把「用户没点、代码却写进去」这条路留着：脏值一旦落进 `providerId`，
   * 目标会被清空、下拉里又没有匹配项，用户必须重选一次真实平台才能恢复。
   */
  const chooseProvider = (value: string) => {
    if (value !== "" && !state.providers.some((item) => item.id === value)) return;
    state.selectProvider(value as AiProviderId | "");
  };

  /** 底部摘要的数：说的是**眼前这批**，不是当前目标的会话总数。 */
  const visibleCount =
    mode.kind === "global"
      ? state.globalResults.length
      : mode.kind === "favorites"
        ? quickCount
        : state.sessions.length;

  /**
   * 折起来时宽度归零，**但节点仍然留在流内**（0 宽的网格项，仍占第 2 列）。
   *
   * 这里必须继续渲染一个 `<aside>`，而且它不能是 `display: none`：网格里
   * `display: none` 的项**不参与放置**，后面两项会被自动放进第 1、2 列，
   * 第 3 列的 `1fr` 空着——主区被撑到 max-content、右侧留一条空白。
   * `.sidebar.hidden` 的做法是 `width: 0`（`app.css`），节点照常占位，
   * 只是看不见。展开的入口在**活动栏**——点「会话」或「工作台」都会把侧栏
   * 叫回来（见 `app.tsx` 的 `onActivity`）；旧的 36px 竖条随之删掉：
   * 它占着一整条竖边只为放一个按钮。
   */
  if (collapsed) {
    return <aside className="sidebar hidden" aria-label="会话侧栏（已折叠）" />;
  }

  return (
    <aside className="sidebar">
      <div className="sidebar-head">
        <div className="sidebar-head-row">
          <h1 className="sidebar-title">会话</h1>
          <ToolbarSpacer />
          <Button
            small
            disabled={!canStart}
            title={canStart ? "新建会话" : "请先选择一个可用的目标"}
            onClick={onNewSession}
          >
            <Icon name="plus" size={13} />
            新建会话
          </Button>
          <button
            type="button"
            className="icon-btn"
            title="折叠侧栏"
            aria-label="折叠侧栏"
            onClick={onToggleCollapsed}
          >
            <Icon name="panel-left" size={16} />
          </button>
        </div>
        {/* 网关胶囊：网关有没有在跑、在哪个端口，是这一屏最常被问的一件事。
            点它去**设置中心的网关页**——改端口、重启网关、看它为什么没起来都在那里。 */}
        <button
          type="button"
          className={`gateway-pill tone-${gwTone}`}
          title={`网关${gateway?.enabled ? "已启用" : "未启用"}：${gatewayPortLabel(gateway)}（点击打开设置中心网关页）`}
          onClick={onOpenGateway}
        >
          <StatusDot
            tone={gwTone === "ok" ? "ok" : gwTone === "warn" ? "warn" : "idle"}
            pulse={gwTone === "ok"}
            size={8}
          />
          <span className="gateway-pill-label">网关</span>
          <span className="gateway-pill-port ui-num ui-mono">{gatewayPortLabel(gateway)}</span>
        </button>
      </div>

      <div className="pickers">
        <div className="picker-row">
          <span className="picker-label">1. 平台</span>
          <Select
            value={state.providerId}
            aria-label="平台"
            onChange={(event) => chooseProvider(event.target.value)}
          >
            {/*
              **占位项：没有选中时显示「请选择平台」。**

              少了它，`value=""` 而 option 里没有空值时浏览器会**视觉上高亮第一项**
              ——于是「没选平台」看起来像「选了 Codex」。用户报的正是这个：
              应用状态其实已经是空的（目标没被拉取、侧栏写着「暂无可用目标」），
              但下拉框里显示着 Codex。

              文案与旧平台一致（它有 `请选择平台` / `请选择一个目标`）。
            */}
            <option value="">请选择平台</option>
            {/*
              加载提示**不带 `value=""`**：平台列表为空时它和上面那条占位项同时存在，
              两个 `value=""` 的 option 会让「哪一条是占位」变得有歧义（原生 select 只
              认第一个匹配项，第二条永远选不中）。不给 value 时它的值退化成文字，
              与空值不再相撞，显示的仍是占位项。

              另外它必须 `disabled`：值退化成文字之后它就是**一个可被选中的脏值**
              （`"加载中…"`）——用户加载期顺手选它一下，`providerId` 就被写成这串文字，
              目标被清空、下拉里又没有这一项，得重选一次真实平台才恢复。
            */}
            {state.providers.length === 0 && <option disabled>加载中…</option>}
            {state.providers.map((item) => (
              <option key={item.id} value={item.id}>
                {item.label}
              </option>
            ))}
          </Select>
        </div>

        <div className="picker-row">
          <span className="picker-label">2. 目标</span>
          <Select
            value={state.targetId}
            aria-label="目标"
            disabled={state.targets.length === 0}
            onChange={(event) => state.selectTarget(event.target.value)}
          >
            {/*
              **占位项：没有选中时显示「请选择目标」。**

              与平台那个同理：少了它，`value=""` 而 option 里没有空值时浏览器会**视觉上
              高亮第一项**——「没选目标」看起来像「选了本机 Codex」。

              平台没选时「暂无可用目标」（那是另一句话：目标列表还没被拉取），
              两者不能混成一个。
            */}
            {state.targets.length === 0 ? (
              <option value="">暂无可用目标</option>
            ) : (
              <option value="">请选择目标</option>
            )}
            {state.targets.map((target) => (
              <option key={target.id} value={target.id}>
                {/* 不可用的目标在选择列表里就标出来——等选中了才发现，用户已经先点了一次。 */}
                {target.available ? target.label : `${target.label}（不可用）`}
              </option>
            ))}
          </Select>
          {/* 右端提示：选中的目标此刻进不去。选择器里那一项已经带了「（不可用）」，
              但下拉收起之后那一行看不见，所以这里再标一次。 */}
          {selectedTarget && !selectedTarget.available && (
            <span className="picker-hint bad">不可用</span>
          )}
        </div>

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
                {canSessionSettings && selectedTarget.kind === "wsl" && selectedTarget.distro && (
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

      {/*
        搜索框只有**一个**。进入跨目标搜索之后同一个框改搜全局（值换成 `globalQuery`）：
        两个框并排会让人以为要填两次，而它们本来就是同一个意图的两段路。
      */}
      <div className="sidebar-search">
        <SearchInput
          value={global ? state.globalQuery : state.query}
          onChange={global ? state.setGlobalQuery : state.setQuery}
          placeholder={global ? "搜索全部活跃会话" : "搜索当前平台会话"}
          busy={global ? state.globalSearching : state.searching}
        />
      </div>

      {/* 过滤 chips：同一批会话的三种看法。收藏夹与回收站此前是二级导航里的一行，
          现在它们是**这个列表的筛选**，与列表挨在一起才不会以为是别的地方。 */}
      <div className="filter-chips">
        <ChipGroup
          // 回收站由右侧的图标按钮表示，那时两个 chip 都不高亮。
          value={trash ? "" : state.view}
          options={[
            { value: "active", label: `全部 ${state.sessions.length}` },
            { value: "favorites", label: `★ 收藏 ${favoriteCount}` }
          ]}
          onChange={(value) => {
            // 换一处地方看 = 退出跨目标搜索，否则点了 chip 界面不动（全局优先）。
            state.setGlobalQuery("");
            state.setView(value === "favorites" ? "favorites" : "active");
          }}
        />
        <ToolbarSpacer />
        <button
          type="button"
          className={`icon-btn${trash ? " on" : ""}`}
          aria-label="回收站"
          aria-pressed={trash}
          // 平台不支持回收站时**禁用并说明原因**，而不是给一个点了会报错的按钮。
          disabled={!canUseTrash}
          title={canUseTrash ? (trash ? "返回当前会话" : "查看回收站") : "请选择支持回收站的平台"}
          onClick={() => {
            state.setGlobalQuery("");
            state.setView(trash ? "active" : "trash");
          }}
        >
          <Icon name="trash" size={15} />
        </button>
      </div>

      {/*
        搜索结果区的头部。

        「在所有目标中搜索」是**一个动作**，不是一处地方：它把当前关键词送进
        `globalQuery`（`use-console` 里那条跨目标搜索随之启动）。进了全局之后，
        这里换成两个筛选（平台 / 目标，与旧平台 `global-session-search-panel` 一致），
        并给一条回到当前列表的出路。

        这里**没有**「搜索消息正文」开关了（裁定 R23）：后端搜索的第二阶段命中正文时
        返回的是完整解析结果，命中词必然出现在结果的 `preview` 里——前端按同一批字段
        重算「摘要是否命中」是恒真的，那个开关只是看起来有效。结果行显示的就是
        后端返回的事实。
      */}
      {(mode.kind === "filtered" || global) && (
        <div className="search-actions">
          {mode.kind === "filtered" ? (
            <Button
              small
              title="把当前关键词发给所有目标"
              onClick={() => state.setGlobalQuery(state.query)}
            >
              在所有目标中搜索
            </Button>
          ) : (
            <>
              <Select
                value={state.searchProviderId}
                aria-label="按平台筛选"
                className="search-select"
                onChange={(event) => {
                  state.setSearchProviderId(event.target.value as AiProviderId | "");
                  // 换了平台，之前挑的目标多半已经不在范围内了，清掉免得看起来「没结果」。
                  state.setSearchTargetId("");
                }}
              >
                <option value="">全部平台</option>
                {state.providers.map((item) => (
                  <option key={item.id} value={item.id}>
                    {item.label}
                  </option>
                ))}
              </Select>
              <Select
                value={state.searchTargetId}
                aria-label="按目标筛选"
                className="search-select"
                onChange={(event) => state.setSearchTargetId(event.target.value)}
              >
                <option value="">全部目标（{scopedTargets.length}）</option>
                {scopedTargets.map((target) => (
                  <option key={target.id} value={target.id}>
                    {target.available ? target.label : `${target.label}（不可用）`}
                  </option>
                ))}
              </Select>
              <Button small onClick={() => state.setGlobalQuery("")}>
                返回当前列表
              </Button>
            </>
          )}
        </div>
      )}

      {/* 批量条只在**平台支持批量操作**且有勾选时出现。
          不支持批量操作的平台（例如 Qoder）上，勾选框本身也不渲染——
          那条规则在卡片那边，由 `selectable` 控制。 */}
      {canBatch && selectedCount > 0 && mode.kind !== "favorites" && mode.kind !== "global" && (
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
          mode.kind === "favorites" ? undefined : mode.kind === "global" ? (
            state.globalResults.length === 0 ? (
              <EmptyState
                message={
                  state.globalQuery.trim() ? "没有匹配的会话。" : "输入关键词，在所有目标里搜索。"
                }
                hint={
                  state.globalQuery.trim()
                    ? "后端会先匹配摘要，摘要不中再读整个文件——因此结果可能稍慢。"
                    : undefined
                }
              />
            ) : undefined
          ) : state.sessions.length === 0 ? (
            <EmptyState
              message={
                state.query.trim() ? "没有匹配的会话。" : emptySessionMessage(state.targetId, selectedTarget, trash)
              }
            />
          ) : undefined
        }
      >
        {(mode.kind === "list" || mode.kind === "filtered") &&
          state.sessions.map((session) => {
            const running = runningSessionIds.has(session.id);
            return (
              <SessionCard
                key={session.id}
                session={session}
                active={session.id === state.selectedId}
                checked={state.selectedIds.includes(session.id)}
                selectable={canBatch}
                running={running}
                onOpen={() => {
                  if (selectedTarget) onOpen(selectedTarget, session.id);
                }}
                // 终端已经在跑就不接双击：再起一路只会把它替换掉。
                onResume={
                  running || !selectedTarget
                    ? undefined
                    : () => onResume(selectedTarget, session.id)
                }
                onToggle={() => state.toggleSelected(session.id)}
                onFavorite={() => void state.toggleFavorite(session)}
                onBranch={() => onDetail(session)}
                onMenu={(x, y) => onMenu(x, y, session.id)}
              />
            );
          })}

        {mode.kind === "favorites" && (
          <>
            <QuickGroup
              title="收藏"
              items={filterQuick(state.quickAccess.favorites)}
              empty={
                state.query.trim() ? "没有匹配的收藏。" : "还没有收藏。在会话卡片上点 ☆ 收藏它。"
              }
              onOpen={(target, session) => onOpen(target, session.id)}
            />
            <QuickGroup
              title="最近打开"
              items={filterQuick(state.quickAccess.recent)}
              empty={state.query.trim() ? "没有匹配的最近打开。" : "还没有打开过会话。"}
              onOpen={(target, session) => onOpen(target, session.id)}
            />
          </>
        )}

        {/*
          跨目标搜索的结果：**后端返回什么就显示什么**。
          这里原来有一个「搜索消息正文」开关，关掉时用本地重算的摘要命中筛一遍——
          那个筛子恒真（后端第二阶段命中正文时返回的是完整解析结果，命中词必然在
          `preview` 里），所以它只是让开关看起来有效。裁定 R23 已删除该开关与筛子：
          命中来源以**后端返回的事实**为准——命中的词就在结果行的预览里。
        */}
        {global &&
          state.globalResults.map((result) => (
            <SearchResultRow
              key={`${result.target.id}:${result.session.id}`}
              target={result.target}
              session={result.session}
              onOpen={() => onOpen(result.target, result.session.id)}
            />
          ))}
      </List>

      {/* 底部摘要：说的是**眼前这批**有多少条（收藏夹 / 搜索结果各算各的），
          并给一个手动刷新。刷新走 `reloadSessions`——收藏夹里它取的是
          `quickAccess`（`use-console` 里分了支），否则那个按钮点了没反应。 */}
      <div className="sidebar-foot">
        <span className="ui-num">
          {provider?.label ?? "未选择平台"} · 共 {visibleCount} 条会话
        </span>
        <ToolbarSpacer />
        <button
          type="button"
          className="icon-btn"
          title="刷新列表"
          aria-label="刷新列表"
          onClick={() => void state.reloadSessions()}
        >
          <Icon name="refresh" size={14} />
        </button>
      </div>
    </aside>
  );
}
