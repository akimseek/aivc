import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { errorText } from "../components";
import * as api from "../ipc/commands";
import type {
  AiMessage,
  AiProviderId,
  AiProviderSummary,
  AiSession,
  AiTarget,
  ExportFormat,
  GatewayPortStatus,
  SessionMutationRef,
  SessionQuickAccess,
  SessionView
} from "../types";
import type { SidebarPane, SidebarView } from "../types";
import { scopeTargets } from "./search-scope";

/**
 * 主流程的数据装配。
 *
 * # 为什么单独一个文件
 *
 * 界面只负责画，取数顺序（缓存先行 → 探测刷新）与几处**异步竞态**的处理
 * 是这块最容易写错的地方，值得单独放、单独读。
 */

/** 一次取多少条消息。会话通常很长，所以先给结尾，再按需往前翻。 */
const MESSAGE_PAGE_SIZE = 80;

/**
 * 历史视图窗口的**累计上限**。
 *
 * # 为什么封顶而不是无限往前翻
 *
 * 每翻一页都会往窗口前面插一段，DOM 与内存随翻页无界增长——几万条的老会话
 * 翻到深处会明显卡顿。封顶后**不再提供**「加载更早」，改指路「消息与分支」
 * 弹框（那边有完整分页）；而不是「继续加载、再丢掉最早一页」：在「向上翻
 * 找旧消息」的交互里，丢最早一页丢掉的正是用户刚装进来的内容——要么按钮
 * 空转，要么内容闪没。
 *
 * 取页大小的整数倍，窗口永远不会因为最后一次加载而过冲。
 */
export const HISTORY_MESSAGE_LIMIT = MESSAGE_PAGE_SIZE * 5;

/** 搜索防抖：每次按键都去读文件太贵（后端第二段会**扫全文**）。 */
const SEARCH_DEBOUNCE_MS = 250;

/*
  **这里原来有一段「记住上次的平台 / 目标」并从 localStorage 还原的逻辑，已删除。**
  
  用户明确要求：**应用打开时平台显示「请选择平台」，不选任何东西**。
  而我实测确认过它造成的就是「打开就选中 Codex / 本机 Codex」——
  因为 localStorage 里存着上一次的 `{providerId:"codex", targetId:"codex:local:…"}`。
  
  它也是**我自己加的**：旧平台全仓只有一处 localStorage，存的是**终端面板高度**
  （`system-terminal.tsx` 的 `STORAGE_HEIGHT_KEY`），没有「记住上次选择」。
  
  于是启动时的四个「替你选好」全部去掉了：平台、目标、会话，以及这里的还原。
*/

/**
 * 从候选里挑一个目标：**只认「当前值」，否则不选**。
 *
 * # 这里原来会退回「第一个可用目标」
 *
 * 那是我自己加的便利，旧平台没有这个行为。后果是**启动后什么都没点，
 * 界面已经替你选好了一个平台、一个目标、一条会话**——用户看到的是别人的选择，
 * 而他并不知道自己「已经选过了」。
 *
 * 旧平台的规则是（`app.tsx` 里那个依赖 `targetId` 的 effect）：
 *
 * ```js
 * const activeSessionForTarget = activeTab?.targetId === targetId ? activeTab.session : null;
 * setSelectedId(activeSessionForTarget?.id || "");
 * ```
 *
 * 也就是**跟着当前终端走，没有就不选**。
 *
 * `preferred` 现在只传**当前值**（列表刷新时别把用户已经选的那个丢掉），
 * 不再有「从 localStorage 还原上次选择」那一项——那同样是我加的，
 * 而且它正是「打开就选中 Codex / 本机 Codex」的原因。旧平台全仓唯一的
 * localStorage 用途是终端面板高度。
 */
export function pickTarget(items: AiTarget[], preferred: string[]): string {
  for (const id of preferred) {
    if (id && items.some((target) => target.id === id)) return id;
  }
  return "";
}

/** 把会话转成变更操作要的引用。带上 `filePath` 更准——后端会先按它做目录围栏校验。 */
export function toRefs(sessions: AiSession[], ids: string[]): SessionMutationRef[] {
  const wanted = new Set(ids);
  return sessions
    .filter((session) => wanted.has(session.id))
    .map((session) => ({ id: session.id, filePath: session.filePath }));
}

/**
 * 侧栏当前是「哪一处地方」与「哪一类列表」。
 *
 * `workbench` 是**概览**而不是列表：切到它时侧栏整块换成运行概览
 * （旧平台的 `SidebarWorkbench` 就是一级视图之一，位置在「会话」之前）。
 *
 * `view` 里现在有三个取值：`active` / `trash` 走会话列表，`favorites`
 * 走的是 `quickAccess`（收藏 + 最近打开）——它是**另一处内容**，不是
 * 当前目标列表的一个筛选，因此切到它时见下面的列表加载 effect。
 */
export type { SidebarPane, SidebarView };

/**
 * 切到某个 `view` 时，要不要去取会话列表。
 *
 * # 为什么收藏夹不发请求
 *
 * 「收藏夹」的内容是 `quickAccess`（跨目标的「收藏」与「最近打开」两组，
 * 只读缓存、按需刷新），与会话列表接口取的是**两批数据**。此前收藏夹是另一个
 * `pane`，靠 `pane` 判断；现在它与「全部 / 回收站」并排成 chips，判据只剩 `view`，
 * 因此这条规则必须显式写出来，并且**只写一处**（effect 与 `loadSessions` 都用它）。
 *
 * 抽成导出函数是为了能直接断言：这是一个「不发请求」的约定，没有第二个地方能证明它。
 */
export function shouldLoadSessionList(view: SidebarView): boolean {
  return view !== "favorites";
}

/**
 * 打开一条会话时，平台与目标该不该真的换。
 *
 * # 为什么要单独算这一步
 *
 * `selectTarget` 带一个**清空会话列表**的副作用（换目标时必须清，否则会显示
 * 上一个目标的残留）。而它的重载 effect 只依赖 `[targetId, view, query]`——
 * **目标没变时 effect 不会重跑**，于是清了就再也填不回来。
 *
 * 用户报的「点击进入终端会话后，列表内容就消失了」正是这条路径：
 * 在当前目标下点会话 → `setTargetId(同一个值)`（无变化）+ `setSessions([])`（清了）。
 *
 * 所以判据必须是「**真的换了**」而不是「打开了一条会话」。
 * 抽成纯函数是为了能直接断言，而不是靠渲染去猜。
 *
 * 泛型参数让调用方能保留自己的平台编号类型（调用处是 `AiProviderId`），
 * 而不是被这个纯函数抹成 `string`。
 */
export function planTargetSwitch<P extends string>(
  currentProvider: string,
  currentTargetId: string,
  target: { id: string; provider: P }
): { provider: P | null; target: string | null } {
  const providerChanged = target.provider !== currentProvider;
  // 平台变了，目标一定要换（新平台的目标集合与旧平台不通用）。
  const targetChanged = providerChanged || target.id !== currentTargetId;
  return {
    provider: providerChanged ? target.provider : null,
    target: targetChanged ? target.id : null
  };
}

/** 跨平台搜索的一条结果：会话 + 它属于哪个目标。 */
export type GlobalSearchResult = {
  target: AiTarget;
  session: AiSession;
};

/**
 * 一句话结果反馈的出口。
 *
 * 由调用方注入（`app.tsx` 传 `useToast().show`）。**可选**：不传时不报，
 * 这让不需要它的测试与别处复用不必造一个假的。
 */
export type Notify = (message: string) => void;

export function useConsole(options: { notify?: Notify } = {}) {
  /**
   * 用 ref 持有回调，暴露一个**恒等稳定**的 `notify`。
   *
   * 直接把它塞进十几个 `useCallback` 的依赖数组里不是不行（`useToast().show`
   * 本身是稳定的），但那样每个动作都要记得加——漏一个就是「这个动作不报」，
   * 而且不会报错。用 ref 之后调用点只关心「说什么」，不关心它从哪来。
   */
  const notifyRef = useRef<Notify | undefined>(options.notify);
  notifyRef.current = options.notify;
  const notify = useCallback((message: string) => {
    notifyRef.current?.(message);
  }, []);

  const [providers, setProviders] = useState<AiProviderSummary[]>([]);
  /** 平台编号。`""` 表示还没选——**不要用 `null`**：界面各处都按字符串判空。 */
  const [providerId, setProviderId] = useState<AiProviderId | "">("");
  const [targets, setTargets] = useState<AiTarget[]>([]);
  const [targetId, setTargetId] = useState("");
  const [sessions, setSessions] = useState<AiSession[]>([]);
  const [view, setView] = useState<SidebarView>("active");
  const [query, setQuery] = useState("");
  const [searching, setSearching] = useState(false);
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [selectedId, setSelectedId] = useState("");
  const [messages, setMessages] = useState<AiMessage[]>([]);
  /**
   * 消息窗口的**代际**。
   *
   * 选中会话/目标一变就 +1；在途的「往前翻」请求带着快照，回来时不等于当前值
   * 就把整页丢掉——否则旧会话的页会插进新会话的窗口（P0-B 的 seq 模式，
   * 42 §五.2 记的正是这里没覆盖）。
   */
  const messagesSeqRef = useRef(0);
  /** 当前已加载消息的**起始绝对序号**；> 0 说明前面还有。 */
  const [messageStart, setMessageStart] = useState(0);
  /**
   * 会话的可见消息**总数**，随分页响应回来。
   *
   * `null` = 还不知道。界面上的会话头需要它：`selectedSession` 来自**列表**，那里的
   * 条数只是**下界**（只读了头部窗口），真正精确的总数只有详情路径有。
   */
  const [messageTotal, setMessageTotal] = useState<number | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  const [gateway, setGateway] = useState<GatewayPortStatus | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  /* ── 侧栏的三个视图 ── */
  const [pane, setPane] = useState<SidebarPane>("sessions");
  const [quickAccess, setQuickAccess] = useState<SessionQuickAccess>({
    favorites: [],
    recent: []
  });
  const [globalQuery, setGlobalQuery] = useState("");
  /** 只搜某个目标；空串表示全部。收窄范围能**少发请求**，不只是少显示结果。 */
  const [searchTargetId, setSearchTargetId] = useState("");
  /**
   * 只搜某个平台；空串表示全部。
   *
   * 与「目标」是两个独立的筛选项（旧平台也是两个下拉）：一个平台可能有多个目标
   * （本机 + 各个 WSL 发行版），先按平台收窄再挑目标是最常见的用法。
   */
  const [searchProviderId, setSearchProviderId] = useState<AiProviderId | "">("");
  const [globalResults, setGlobalResults] = useState<GlobalSearchResult[]>([]);
  const [globalSearching, setGlobalSearching] = useState(false);
  const globalSeqRef = useRef(0);

  const startedRef = useRef(false);

  /* ── 平台 ── */
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const items = await api.listProviders();
        if (cancelled) return;
        setProviders(items);
        /*
          **平台不选：一律留空，等用户自己挑。**
          
          这里踩过两次，都是「替你选好」：
          
          1. 曾经退回 `items[0]`（第一个平台）；
          2. 改成只认 localStorage 里的上次选择之后，**用户打开仍然看到选中了
             Codex**——因为那个键里就存着他上次用的
             `{providerId:"codex", targetId:"codex:local:…"}`。
             「还原」和「挑第一个」在他眼里是同一件事：不是我选的。
          
          现在一律留空。没选平台时各面板显示「请选择平台」，
          那是旧平台本来就有的状态（`请选择平台` / `请选择一个目标` 都在它的文案里）。
        */
        setProviderId("");
      } catch (problem) {
        if (!cancelled) setError(errorText(problem));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  /* ── 网关端口：状态栏要显示 ── */
  useEffect(() => {
    let cancelled = false;
    void api
      .getGatewayPort()
      .then((status) => {
        if (!cancelled) setGateway(status);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);

  /* ── 目标：**缓存先行，再探测刷新** ──
   *
   * 顺序不能反。探测要碰文件系统与 WSL（冷启动 5–7 秒），先等它会让首屏空着。
   */
  const loadTargets = useCallback(
    async (wantedProvider: string) => {
      if (!wantedProvider) {
        setTargets([]);
        setTargetId("");
        return;
      }
      setBusy(true);
      try {
        const cached = await api.listCachedTargets(wantedProvider);
        if (cached.length > 0) {
          setTargets(cached);
          // `preferred` 只传**当前值**：列表刷新时别把用户已经选的那个丢掉。
          // 不再传「上次记住的目标」——那会让打开应用就选中一个目标。
          setTargetId((current) => pickTarget(cached, [current]));
        }
        const fresh = await api.listTargets(wantedProvider);
        setTargets(fresh);
        setTargetId((current) => pickTarget(fresh, [current]));
        setError("");
      } catch (problem) {
        setError(errorText(problem));
      } finally {
        setBusy(false);
      }
    },
    []
  );

  useEffect(() => {
    void loadTargets(providerId);
  }, [providerId, loadTargets]);

  /* ── 会话 ──
   *
   * 这里有一个真实的竞态：输入搜索词会连续触发请求，而**后发的可能先回**。
   * 用一个自增的请求号，只有最新那次的结果才允许落地。
   */
  const requestSeqRef = useRef(0);
  const loadSessions = useCallback(
    async (wantedTarget: string, wantedView: SidebarView, wantedQuery: string) => {
      if (!wantedTarget) {
        setSessions([]);
        setSelectedId("");
        return;
      }
      /*
        收藏夹**不发会话列表请求**：它的内容来自 `quickAccess`，只有 `active` / `trash`
        才对应这个接口。这里再挡一次（effect 里也挡）是因为 `loadSessions` 还有别的
        调用点（批量操作后、重命名后…）：它们带着当时的 `view` 进来，而那时完全可能
        已经站在收藏夹上——少这一层就会用收藏夹的 `view` 去发一个列表请求。
      */
      if (!shouldLoadSessionList(wantedView)) return;
      // 列表接口只认 `active` / `trash`；收藏夹在上面已经返回了。
      const listView: SessionView = wantedView === "trash" ? "trash" : "active";
      const seq = ++requestSeqRef.current;
      const keyword = wantedQuery.trim();
      if (keyword) setSearching(true);
      try {
        if (!keyword) {
          const cached = await api.listCachedSessions(wantedTarget, listView);
          // 只有最新那次请求可以落地；否则旧结果会把新结果盖掉。
          if (seq === requestSeqRef.current && cached.length > 0) setSessions(cached);
        }
        const fresh = keyword
          ? await api.searchSessions(wantedTarget, listView, keyword)
          : listView === "trash"
            ? await api.listTrashSessions(wantedTarget)
            : await api.listSessions(wantedTarget);
        if (seq !== requestSeqRef.current) return;
        setSessions(fresh);
        /*
          会话：列表刷新后**只在原选中还在时保留它**，否则不选。
          
          这里原来会退回 `fresh[0]`（列表第一项）。那是错的，而且用户直接看到了后果：
          「现在默认选中历史会话中的第一项"——右侧那一排动作（详情/重命名/复制/导出）
          于是全部作用在一条**用户从没选过**的会话上。
          
          旧平台的规则是**跟着当前终端那条会话走**（`activeTab.session`）；
          终端没开或不属于当前目标时就是不选。跟随终端那部分由 `App` 的
          effect 做（它才拿得到终端），这里只负责「别自作主张选第一条」。
        */
        setSelectedId((current) => (fresh.some((s) => s.id === current) ? current : ""));
        setSelectedIds([]);
        setError("");
        return true;
      } catch (problem) {
        if (seq === requestSeqRef.current) setError(errorText(problem));
        return false;
      } finally {
        if (seq === requestSeqRef.current) setSearching(false);
      }
    },
    []
  );

  // 搜索防抖：连续输入只发最后一次。
  useEffect(() => {
    // 收藏夹的数据来自 `quickAccess`（它自己按 view 变化取一次），**不发列表请求**；
    // 从收藏夹切回全部 / 回收站时这个 effect 会重跑，列表照常刷新。
    if (!shouldLoadSessionList(view)) return;
    const timer = window.setTimeout(() => {
      void loadSessions(targetId, view, query);
    }, query.trim() ? SEARCH_DEBOUNCE_MS : 0);
    return () => window.clearTimeout(timer);
  }, [targetId, view, query, loadSessions]);

  /* ── 消息：选中会话后取**最后一页**，再按需往前翻 ── */
  useEffect(() => {
    // 换会话/目标即作废在途的翻页（见 `messagesSeqRef` 的说明）。
    const seq = ++messagesSeqRef.current;
    if (!targetId || !selectedId) {
      setMessages([]);
      setMessageStart(0);
      setMessageTotal(null);
      setError("");
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const page = await api.getSessionMessagesPage(targetId, selectedId, -1, MESSAGE_PAGE_SIZE);
        if (cancelled || seq !== messagesSeqRef.current) return;
        setMessages(page.messages);
        setMessageStart(page.offset);
        // 0 表示后端也拿不到总数——那时界面回退到列表的下界。
        setMessageTotal(page.messageCount > 0 ? page.messageCount : null);
      } catch (problem) {
        if (!cancelled && seq === messagesSeqRef.current) setError(errorText(problem));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [targetId, selectedId]);

  const loadEarlierMessages = useCallback(async () => {
    if (
      !targetId ||
      !selectedId ||
      messageStart <= 0 ||
      loadingMore ||
      // 到累计上限就不再取更早的页（判据的取舍见 `HISTORY_MESSAGE_LIMIT`）。
      messages.length >= HISTORY_MESSAGE_LIMIT
    ) {
      return;
    }
    const seq = messagesSeqRef.current;
    setLoadingMore(true);
    try {
      const offset = Math.max(0, messageStart - MESSAGE_PAGE_SIZE);
      const page = await api.getSessionMessagesPage(targetId, selectedId, offset, MESSAGE_PAGE_SIZE);
      // 会话/目标已经切走：这一页属于**上一个窗口**，整个丢掉。
      if (seq !== messagesSeqRef.current) return;
      /*
        页尾可能与已加载的窗口**重叠**：`0 < messageStart < 页大小` 时请求从 0 起，
        后端回的整页里 `[messageStart, …)` 那段已经在窗口里——直接前置会把它们插
        两遍（列表里同一条出现两次、同 `sourceLine` 撞 key）。只保留真正更早的部分。
      */
      const fresh = page.messages.slice(0, Math.max(0, messageStart - page.offset));
      // **插到前面**——往前翻的是更早的消息，追加到末尾就全乱了。
      setMessages((current) => [...fresh, ...current]);
      setMessageStart(page.offset);
      setError("");
    } catch (problem) {
      // 旧会话的失败也不许盖到新窗口的错误位上。
      if (seq === messagesSeqRef.current) setError(errorText(problem));
    } finally {
      setLoadingMore(false);
    }
  }, [targetId, selectedId, messageStart, loadingMore, messages.length]);

  /*
    **这里曾经有一段「选择变化就记下来，供下次启动还原」的 effect，已删除。**
    
    它写的是 `aivc.ui.lastSelection`。用户报「打开还是选中 Codex / 本机 Codex」，
    实测就是它在起作用：那个键里存着上一次的
    `{providerId:"codex", targetId:"codex:local:…"}`。
    
    写和读一起删掉——只删读取的话，那个键会变成没人用的死数据，
    以后有人看到它又会去「修好」这条还原。
  */

  useEffect(() => {
    if (startedRef.current) return;
    startedRef.current = true;
    void api.logPerformance("ui.started", performance.now(), "ok");
  }, []);

  /**
   * 切换平台。
   *
   * # 取值要校验
   *
   * 它带一串**清空副作用**（目标、会话、选中项全作废——留着会显示另一个平台的
   * 内容）。于是脏值不再是「下拉框显示不准」这种小问题：把清单里不存在的编号写进
   * `providerId`，目标会被清空、而下拉里又没有匹配的项，用户必须重选一次真实平台
   * 才能恢复。
   *
   * 侧栏 `chooseProvider` 已经挡了一层（它读得到 `providers`），这一层是**第二道**：
   * 那个回调是界面代码，而这个 hook 的返回值是公共入口——`openAt` 也会调它，
   * 将来多一个调用点不该要求每个调用方都记得自己校验一遍。
   */
  const selectProvider = useCallback(
    (next: AiProviderId | "") => {
      // `""` 是合法的（未选平台），其余必须在清单里。
      if (next !== "" && !providers.some((item) => item.id === next)) return;
      setProviderId(next);
      // 平台变了，目标与会话都作废——留着会让界面显示另一个平台的会话。
      setTargets([]);
      setTargetId("");
      setSessions([]);
      setSelectedId("");
      setSelectedIds([]);
    },
    [providers]
  );

  const selectTarget = useCallback((next: string) => {
    setTargetId(next);
    setSessions([]);
    setSelectedId("");
    setSelectedIds([]);
  }, []);

  /* ── 快捷访问：只读缓存 ──
   *
   * 它是**收藏夹的内容**，也是侧栏 chip 上「★ 收藏 N」的那个 N。这两处要求它
   * 在没进过收藏夹时也得是对的，所以取数有三条路径（都在下面的 effect / 动作里）：
   * 启动取一次、进收藏夹再取一次（可能已过很久）、收藏动作成功后刷新。
   * 定义放在 `toggleFavorite` 之前：那个动作要引用它（`const` 有 TDZ）。
   */
  const loadQuickAccess = useCallback(async () => {
    try {
      setQuickAccess(await api.listSessionQuickAccess());
      setError("");
    } catch (problem) {
      setError(errorText(problem));
    }
  }, []);

  const toggleFavorite = useCallback(
    async (session: AiSession) => {
      if (!targetId) return;
      const next = !session.metadata?.favorite;
      // **先改本地再发请求**：收藏是即时反馈的操作，等一个往返会让点击显得没反应。
      setSessions((current) =>
        current.map((item) =>
          item.id === session.id
            ? { ...item, metadata: { ...item.metadata, favorite: next } }
            : item
        )
      );
      try {
        await api.setSessionFavorite(targetId, session.id, next);
        /*
          收藏刚变过，`quickAccess`（收藏夹的内容、以及 chip 上的数字）立刻过期。
          不刷新的话，从未进过收藏夹的用户会看到「★ 收藏 0」在点星之后**不动**。
          刷新失败只在 `error` 里说一句，不回滚：收藏本身已经成功了。
        */
        await loadQuickAccess();
        // 收藏是即时的视觉变化，但**列表里那颗星可能不在视野内**（快捷视图、
        // 搜索结果里都长得不一样），说一句比让用户去找那颗星更确定。
        notify(next ? "已收藏会话。" : "已取消收藏会话。");
      } catch (problem) {
        setError(errorText(problem));
        // 失败就退回去，别让界面显示一个并不存在的状态。
        setSessions((current) =>
          current.map((item) =>
            item.id === session.id
              ? { ...item, metadata: { ...item.metadata, favorite: !next } }
              : item
          )
        );
      }
    },
    [targetId, loadQuickAccess]
  );

  const toggleSelected = useCallback((id: string) => {
    setSelectedIds((current) =>
      current.includes(id) ? current.filter((item) => item !== id) : [...current, id]
    );
  }, []);

  const toggleSelectAll = useCallback(() => {
    setSelectedIds((current) => (current.length === sessions.length ? [] : sessions.map((s) => s.id)));
  }, [sessions]);

  /** 批量操作的公共骨架：调用、刷新、清空选择、报错、**报成功**。 */
  const runBatch = useCallback(
    async (label: string, action: (refs: SessionMutationRef[]) => Promise<unknown>) => {
      if (!targetId) return;
      const refs = toRefs(sessions, selectedIds);
      if (refs.length === 0) return;
      setBusy(true);
      try {
        await action(refs);
        setSelectedIds([]);
        await loadSessions(targetId, view, query);
        setError("");
        // 批量操作之后列表会变，但**用户可能记不清自己有没点成功**（选中状态被清掉了）。
        notify(`${label}完成：共 ${refs.length} 条。`);
      } catch (problem) {
        setError(`${label}失败：${errorText(problem)}`);
      } finally {
        setBusy(false);
      }
    },
    [targetId, sessions, selectedIds, view, query, loadSessions, notify]
  );

  const deleteSelected = useCallback(
    () => runBatch("移入回收站", (refs) => api.deleteSessions(targetId, refs)),
    [runBatch, targetId]
  );

  const purgeSelected = useCallback(
    () => runBatch("彻底删除", (refs) => api.purgeSessions(targetId, refs)),
    [runBatch, targetId]
  );

  /**
   * 恢复。
   *
   * 后端**没有**批量恢复命令（只有单条 `restore-session`），因此这里逐条发。
   * 一条失败不该让其余停下——把失败条数与成功条数一起报出来，而不是整体回滚。
   */
  const restoreSelected = useCallback(async () => {
    if (!targetId) return;
    const refs = toRefs(sessions, selectedIds);
    if (refs.length === 0) return;
    setBusy(true);
    let failed = 0;
    for (const ref of refs) {
      try {
        await api.restoreSession(targetId, ref.id);
      } catch {
        failed += 1;
      }
    }
    setSelectedIds([]);
    await loadSessions(targetId, view, query);
    setBusy(false);
    if (failed > 0) {
      // 部分失败要**两个数都报**，只说「失败」会让用户不知道有多少条成了。
      setError(`恢复完成：成功 ${refs.length - failed} 条，失败 ${failed} 条。`);
    } else {
      setError("");
      notify(`恢复完成：共 ${refs.length} 条。`);
    }
  }, [targetId, sessions, selectedIds, view, query, loadSessions, notify]);

  /**
   * 恢复**一条**（右键菜单用）。
   *
   * 与 `restoreSelected` 分开而不是复用它：后者读的是勾选集，
   * 而右键一条会话时用户并没有勾选它。硬要复用就得先偷偷改勾选集——
   * 那会让「勾选状态」在用户没动它的时候发生变化。
   */
  const restoreOne = useCallback(
    async (session: AiSession) => {
      if (!targetId) return;
      try {
        await api.restoreSession(targetId, session.id);
        await loadSessions(targetId, view, query);
        // 恢复之后它**从回收站列表里消失了**——不说一句，看起来像被删掉了。
        notify("会话已恢复。");
      } catch (error) {
        setError(errorText(error));
      }
    },
    [targetId, view, query, loadSessions, notify]
  );

  const selectedTarget = useMemo(
    () => targets.find((target) => target.id === targetId) ?? null,
    [targets, targetId]
  );

  /* ── 会话级操作：重命名 / 复制 / 导出 / 分支 ──
   *
   * 四者都改动了磁盘上的东西，因此共同点是：**做完必须刷新列表**——
   * 复制与分支会产生新会话，重命名会改标题，导出虽不改会话但用户会想知道去了哪。
   */

  const renameSession = useCallback(
    async (title: string) => {
      const trimmed = title.trim();
      if (!targetId || !selectedId || !trimmed) return;
      try {
        await api.setSessionCustomTitle(targetId, selectedId, trimmed);
        await loadSessions(targetId, view, query);
        setError("");
        notify("会话名称已保存。");
      } catch (problem) {
        setError(`重命名失败：${errorText(problem)}`);
      }
    },
    [targetId, selectedId, view, query, loadSessions, notify]
  );

  /**
   * 复制当前选中的会话。
   *
   * `title` 可选：右键菜单那条路径会**问名字**（旧平台的复制对话框有个
   * 「会话名称（可选）」字段）。不问的话列表里立刻出现两条一模一样的标题，
   * 而用户复制它通常正是为了改出个不同的名字。
   */
  const duplicateCurrent = useCallback(
    async (title?: string) => {
      if (!targetId || !selectedId) return;
      try {
        const copy = await api.duplicateSession(targetId, selectedId, title);
        await loadSessions(targetId, view, query);
        // 复制完直接选中副本——否则用户要在列表里找哪一条才是新的。
        setSelectedId(copy.id);
        setError("");
        notify("会话已复制。");
      } catch (problem) {
        setError(`复制失败：${errorText(problem)}`);
      }
    },
    [targetId, selectedId, view, query, loadSessions, notify]
  );

  /**
   * 导出会话。
   *
   * `sessionId` 省略时导出**当前选中**的那一条（菜单栏那一套的语义）。
   * 会话右键菜单会显式传编号：它要导出的是**被右键的那一条**，而 `setSelectedId`
   * 与导出发生在同一个事件里——`setState` 要等下一次渲染才生效，只读选中值的
   * 那一刻拿到的还是上一条（什么都没选时则什么都不导）。
   */
  const exportCurrent = useCallback(
    async (format: ExportFormat, sessionId?: string) => {
      const id = sessionId ?? selectedId;
      if (!targetId || !id) return;
      try {
        // 不传路径：由后端挑一个确定的位置，再把实际路径回给我们显示。
        const result = await api.exportSession(targetId, id, format, "");
        setError("");
        return result;
      } catch (problem) {
        setError(`导出失败：${errorText(problem)}`);
        return null;
      }
    },
    [targetId, selectedId]
  );

  /**
   * 从某条消息处创建分支。
   *
   * 锚点由界面按平台提供：Codex 用原始行号，Claude/Gemini 用绝对可见序号。
   *
   * `sessionId` 省略时从**当前选中**的那条分出去。消息弹框会显式传编号：它开着的
   * 未必是选中项（侧栏卡片上的「消息与分支」就能为非选中会话开弹框），只读选中值
   * 会把分支分到**别的会话**上去——与 [`exportCurrent`] 同一条理由、同一种做法。
   */
  const branchPendingRef = useRef(false);
  const currentTargetIdRef = useRef(targetId);
  useEffect(() => { currentTargetIdRef.current = targetId; }, [targetId]);
  const branchFromMessage = useCallback(
    async (messageLine: number, sessionId?: string, sourceTargetId = targetId) => {
      const id = sessionId ?? selectedId;
      if (!sourceTargetId || !id || branchPendingRef.current) return null;
      branchPendingRef.current = true;
      try {
        const branch = await api.branchSession(sourceTargetId, id, messageLine);
        if (!branch.id || branch.id === id) throw new Error("分支编号无效，已停止打开原会话。");
        let refreshed = true;
        if (sourceTargetId === currentTargetIdRef.current) {
          setView("active");
          setQuery("");
          refreshed = (await loadSessions(sourceTargetId, "active", "")) !== false;
          // 列表有延迟或刷新失败时仍保留已经创建的分支。
          if (sourceTargetId === currentTargetIdRef.current) {
            setSessions((current) => [branch, ...current.filter((item) => item.id !== branch.id)]);
            setSelectedId(branch.id);
          }
        }
        setError("");
        notify(refreshed ? "已创建新的分支会话。" : "分支已创建，但会话列表刷新失败，请稍后手动刷新。");
        return branch;
      } catch (problem) {
        setError(`分支失败：${errorText(problem)}`);
        notify(`分支失败：${errorText(problem)}`);
        return null;
      } finally {
        branchPendingRef.current = false;
      }
    },
    [targetId, selectedId, loadSessions, notify]
  );

  /** 在系统文件管理器里打开当前会话所在的目录。 */
  const openCurrentFolder = useCallback(async () => {
    if (!targetId || !selectedId) return;
    try {
      await api.openSessionFolder(targetId, selectedId);
      setError("");
    } catch (problem) {
      setError(`打开目录失败：${errorText(problem)}`);
    }
  }, [targetId, selectedId]);

  /** 单个会话移入回收站 / 彻底删除。 */
  const removeSession = useCallback(
    async (session: AiSession, permanent: boolean) => {
      if (!targetId) return;
      try {
        if (permanent) await api.purgeSession(targetId, session.id, session.filePath);
        else await api.deleteSession(targetId, session.id, session.filePath);
        await loadSessions(targetId, view, query);
        setError("");
        // 两种结果**必须说清是哪一种**：彻底删除不可撤销。
        notify(permanent ? "会话已彻底删除。" : "会话已移入回收站。");
      } catch (problem) {
        setError(`${permanent ? "彻底删除" : "移入回收站"}失败：${errorText(problem)}`);
      }
    },
    [targetId, view, query, loadSessions, notify]
  );

  /**
   * 手动指定 WSL 内的 Codex 目录，然后重新探测。
   *
   * **这一步是发行版进不去时的唯一出路**：自动探测注定失败，但用户知道 Codex 装在哪。
   * 必须重新探测——否则界面还停在旧结果上，用户会以为没生效。
   */
  const configureWslCodexHome = useCallback(
    async (distro: string, codexHome: string) => {
      try {
        await api.setWslCodexHome(distro, codexHome);
        await loadTargets(providerId);
        setError("");
        notify("WSL Codex 目录已保存。");
      } catch (problem) {
        setError(`指定目录失败：${errorText(problem)}`);
      }
    },
    [providerId, loadTargets, notify]
  );

  const clearWslCodexHomeOverride = useCallback(
    async (distro: string) => {
      try {
        await api.clearWslCodexHome(distro);
        await loadTargets(providerId);
        setError("");
        notify("已恢复自动探测会话目录。");
      } catch (problem) {
        setError(`清除目录覆盖失败：${errorText(problem)}`);
      }
    },
    [providerId, loadTargets, notify]
  );

  /* ── 快捷访问的取数时机
   *
   * 「★ 收藏 N」这个数字在**没进过收藏夹**时也要是对的，所以启动就取一次。
   * 另外两条：进收藏夹时再取（可能已过很久），以及收藏动作成功后刷新
   * （`toggleFavorite` 里调）。
   */
  useEffect(() => {
    void loadQuickAccess();
  }, [loadQuickAccess]);

  useEffect(() => {
    // 进收藏夹时再取一次：启动那次（上面）可能已经过很久了。
    if (view === "favorites") void loadQuickAccess();
  }, [view, loadQuickAccess]);

  /* ── 跨平台搜索 ──
   *
   * 后端一次只搜一个目标（`codex:search-sessions` 的 `targetId` 是必填的），因此
   * 跨平台 = 逐个目标发。并发发出去，并且**一个目标失败不拖垮整次搜索**——
   * 某台发行版进不去是常态，不该让本机的结果也看不见。
   *
   * # 触发条件只有 `globalQuery` 非空
   *
   * 它此前挂在一个「搜索 pane」上（一级导航里的第三处地方）。搜索区不占一级导航之后，
   * 唯一进入跨目标搜索的动作是搜索结果区里的「在所有目标中搜索」——那个按钮做的事
   * 就是**把当前关键词写进 `globalQuery`**。因此这里只看关键词：有就搜（防抖后），
   * 清空就撤掉结果（列表区随之回到当前目标）。
   */
  useEffect(() => {
    const keyword = globalQuery.trim();
    if (!keyword) {
      setGlobalResults([]);
      return;
    }
    const seq = ++globalSeqRef.current;
    // 只对**范围内的**目标发请求：收窄筛选的价值一半在于少做无用功，
    // 全量搜完再在前端过滤掉，那些请求已经付过代价了。
    //
    // 两层筛选（平台 → 目标）与旧平台 `global-session-search-panel.tsx` 的两个下拉一致。
    const scoped = scopeTargets(targets, searchProviderId, searchTargetId);
    const timer = window.setTimeout(() => {
      void (async () => {
        setGlobalSearching(true);
        try {
          const groups = await Promise.all(
            scoped.map(async (target): Promise<GlobalSearchResult[]> => {
              try {
                const found = await api.searchSessions(target.id, "active", keyword);
                return found.map((session) => ({ target, session }));
              } catch {
                return [];
              }
            })
          );
          // 后发的可能先回：只让最新那次的结果落地。
          if (seq !== globalSeqRef.current) return;
          setGlobalResults(groups.flat());
          setError("");
        } finally {
          if (seq === globalSeqRef.current) setGlobalSearching(false);
        }
      })();
    }, SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [globalQuery, targets, searchTargetId, searchProviderId]);

  /**
   * 打开一条会话：切到它所属的平台与目标，选中它，回到会话视图。
   *
   * # 为什么必须判断「目标是不是真的变了」
   *
   * 这里此前**无条件**调用 `selectTarget(target.id)`，而 `selectTarget` 会
   * `setSessions([])` 把列表清空；列表的重载 effect 只依赖 `[targetId, view, query]`。
   *
   * 于是最常见的那条路径——**在当前目标下点一条会话**——会这样：
   *
   * 1. `setTargetId(同一个值)` → React 认为状态没变，**effect 不重跑**；
   * 2. `setSessions([])` → 列表已经空了；
   * 3. 没有任何东西会把它填回来。
   *
   * 用户看到的就是「点击进入会话后，会话列表的内容消失了」。
   *
   * 修法不是「别清空」，而是**只在真的换目标时才清空**——那条路径上 effect 会
   * 重跑并把新目标的会话装回来，清空正是为了避免显示上一个目标的残留。
   */
  const openAt = useCallback(
    (target: AiTarget, session: AiSession) => {
      const plan = planTargetSwitch(providerId, targetId, target);
      if (plan.provider) selectProvider(plan.provider);
      // 平台或目标**确实**变了才走 `selectTarget`（它带清空副作用）。
      if (plan.target) selectTarget(plan.target);
      setSelectedId(session.id);
      setPane("sessions");
      setError("");
      /*
        从**收藏夹**打开一条会话时回到当前会话列表。

        旧模型里收藏夹是另一个 pane，这条路会顺手把侧栏切回会话列表；并进 chips
        之后 `view` 不再跟着 pane 变，所以这一步要显式做——否则用户看到的是
        「选中了，但列表里没有这条」（收藏夹是缓存里的另一批数据）。

        回收站**不切**：它与当前会话是同一批数据的两处筛选，打开一条就跳走
        会让人丢掉刚才的位置。
      */
      setView((current) => (current === "favorites" ? "active" : current));
      void api.markSessionOpened(target.id, session.id).catch(() => undefined);
    },
    [providerId, targetId, selectProvider, selectTarget, setView]
  );

  return {
    providers,
    providerId,
    selectProvider,
    targets,
    targetId,
    selectTarget,
    selectedTarget,
    sessions,
    view,
    setView,
    query,
    setQuery,
    searching,
    selectedIds,
    toggleSelected,
    toggleSelectAll,
    selectedId,
    setSelectedId,
    selectedSession: sessions.find((session) => session.id === selectedId) ?? null,
    messages,
    messageStart,
    messageTotal,
    hasEarlier: messageStart > 0,
    loadEarlierMessages,
    loadingMore,
    toggleFavorite,
    deleteSelected,
    purgeSelected,
    restoreSelected,
    restoreOne,
    renameSession,
    duplicateCurrent,
    exportCurrent,
    branchFromMessage,
    openCurrentFolder,
    removeSession,
    configureWslCodexHome,
    clearWslCodexHomeOverride,
    gateway,
    error,
    setError,
    busy,
    pane,
    setPane,
    quickAccess,
    loadQuickAccess,
    globalQuery,
    setGlobalQuery,
    searchTargetId,
    setSearchTargetId,
    searchProviderId,
    setSearchProviderId,
    globalResults,
    globalSearching,
    openAt,
    reloadTargets: () => loadTargets(providerId),
    /*
      底部摘要的「刷新」按**眼前这批**刷新：收藏夹的内容是 `quickAccess`，
      与会话列表接口是两批数据。不做这个分支的话，站在收藏夹上点刷新
      `loadSessions` 会直接早退（`shouldLoadSessionList("favorites") === false`），
      连缓存都不重取——一个点了没反应的死按钮。
    */
    reloadSessions: () =>
      view === "favorites" ? loadQuickAccess() : loadSessions(targetId, view, query)
  };
}
