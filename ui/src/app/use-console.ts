import { useCallback, useEffect, useMemo, useRef, useState } from "react";

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

/**
 * 主流程的数据装配。
 *
 * # 为什么单独一个文件
 *
 * 界面只负责画，取数顺序（缓存先行 → 探测刷新 → 自动选中）与几处**异步竞态**的处理
 * 是这块最容易写错的地方，值得单独放、单独读。
 */

const LAST_SELECTION_KEY = "aivc.ui.lastSelection";

/** 一次取多少条消息。会话通常很长，所以先给结尾，再按需往前翻。 */
const MESSAGE_PAGE_SIZE = 80;

/** 搜索防抖：每次按键都去读文件太贵（后端第二段会**扫全文**）。 */
const SEARCH_DEBOUNCE_MS = 250;

type LastSelection = { providerId: AiProviderId; targetId: string };

function readLastSelection(): LastSelection | null {
  try {
    const raw = window.localStorage.getItem(LAST_SELECTION_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<LastSelection> | null;
    if (typeof parsed?.providerId !== "string" || !parsed.providerId) return null;
    return {
      providerId: parsed.providerId,
      targetId: typeof parsed.targetId === "string" ? parsed.targetId : ""
    };
  } catch {
    // 隐私模式、数据被改坏：按「没记住」处理即可，不该让应用起不来。
    return null;
  }
}

function writeLastSelection(value: LastSelection) {
  try {
    window.localStorage.setItem(LAST_SELECTION_KEY, JSON.stringify(value));
  } catch {
    /* 记不住而已，不是错误。 */
  }
}

function describe(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** 从候选里挑一个目标：优先还原，其次当前仍有效的，最后第一个可用的。 */
export function pickTarget(items: AiTarget[], preferred: string[]): string {
  for (const id of preferred) {
    if (id && items.some((target) => target.id === id)) return id;
  }
  return items.find((target) => target.available)?.id ?? items[0]?.id ?? "";
}

/** 把会话转成变更操作要的引用。带上 `filePath` 更准——后端会先按它做目录围栏校验。 */
export function toRefs(sessions: AiSession[], ids: string[]): SessionMutationRef[] {
  const wanted = new Set(ids);
  return sessions
    .filter((session) => wanted.has(session.id))
    .map((session) => ({ id: session.id, filePath: session.filePath }));
}

/** 侧栏当前显示哪一类列表。 */
export type SidebarPane = "sessions" | "quick" | "search";

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

export function useConsole() {
  const [providers, setProviders] = useState<AiProviderSummary[]>([]);
  /** 平台编号。`""` 表示还没选——**不要用 `null`**：界面各处都按字符串判空。 */
  const [providerId, setProviderId] = useState<AiProviderId | "">("");
  const [targets, setTargets] = useState<AiTarget[]>([]);
  const [targetId, setTargetId] = useState("");
  const [sessions, setSessions] = useState<AiSession[]>([]);
  const [view, setView] = useState<SessionView>("active");
  const [query, setQuery] = useState("");
  const [searching, setSearching] = useState(false);
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [selectedId, setSelectedId] = useState("");
  const [messages, setMessages] = useState<AiMessage[]>([]);
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
  const [globalResults, setGlobalResults] = useState<GlobalSearchResult[]>([]);
  const [globalSearching, setGlobalSearching] = useState(false);
  const globalSeqRef = useRef(0);

  const [restored] = useState(readLastSelection);
  const startedRef = useRef(false);

  /* ── 平台 ── */
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const items = await api.listProviders();
        if (cancelled) return;
        setProviders(items);
        const wanted =
          restored?.providerId && items.some((p) => p.id === restored.providerId)
            ? restored.providerId
            : (items[0]?.id ?? "");
        setProviderId(wanted);
      } catch (problem) {
        if (!cancelled) setError(describe(problem));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [restored]);

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
          setTargetId((current) => pickTarget(cached, [current, restored?.targetId ?? ""]));
        }
        const fresh = await api.listTargets(wantedProvider);
        setTargets(fresh);
        setTargetId((current) => pickTarget(fresh, [current, restored?.targetId ?? ""]));
        setError("");
      } catch (problem) {
        setError(describe(problem));
      } finally {
        setBusy(false);
      }
    },
    [restored]
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
    async (wantedTarget: string, wantedView: SessionView, wantedQuery: string) => {
      if (!wantedTarget) {
        setSessions([]);
        setSelectedId("");
        return;
      }
      const seq = ++requestSeqRef.current;
      const keyword = wantedQuery.trim();
      if (keyword) setSearching(true);
      try {
        if (!keyword) {
          const cached = await api.listCachedSessions(wantedTarget, wantedView);
          // 只有最新那次请求可以落地；否则旧结果会把新结果盖掉。
          if (seq === requestSeqRef.current && cached.length > 0) setSessions(cached);
        }
        const fresh = keyword
          ? await api.searchSessions(wantedTarget, wantedView, keyword)
          : wantedView === "trash"
            ? await api.listTrashSessions(wantedTarget)
            : await api.listSessions(wantedTarget);
        if (seq !== requestSeqRef.current) return;
        setSessions(fresh);
        setSelectedId((current) =>
          fresh.some((s) => s.id === current) ? current : (fresh[0]?.id ?? "")
        );
        setSelectedIds([]);
        setError("");
      } catch (problem) {
        if (seq === requestSeqRef.current) setError(describe(problem));
      } finally {
        if (seq === requestSeqRef.current) setSearching(false);
      }
    },
    []
  );

  // 搜索防抖：连续输入只发最后一次。
  useEffect(() => {
    const timer = window.setTimeout(() => {
      void loadSessions(targetId, view, query);
    }, query.trim() ? SEARCH_DEBOUNCE_MS : 0);
    return () => window.clearTimeout(timer);
  }, [targetId, view, query, loadSessions]);

  /* ── 消息：选中会话后取**最后一页**，再按需往前翻 ── */
  useEffect(() => {
    if (!targetId || !selectedId) {
      setMessages([]);
      setMessageStart(0);
      setMessageTotal(null);
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const page = await api.getSessionMessagesPage(targetId, selectedId, -1, MESSAGE_PAGE_SIZE);
        if (cancelled) return;
        setMessages(page.messages);
        setMessageStart(page.offset);
        // 0 表示后端也拿不到总数——那时界面回退到列表的下界。
        setMessageTotal(page.messageCount > 0 ? page.messageCount : null);
      } catch (problem) {
        if (!cancelled) setError(describe(problem));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [targetId, selectedId]);

  const loadEarlierMessages = useCallback(async () => {
    if (!targetId || !selectedId || messageStart <= 0 || loadingMore) return;
    setLoadingMore(true);
    try {
      const offset = Math.max(0, messageStart - MESSAGE_PAGE_SIZE);
      const page = await api.getSessionMessagesPage(targetId, selectedId, offset, MESSAGE_PAGE_SIZE);
      // **插到前面**——往前翻的是更早的消息，追加到末尾就全乱了。
      setMessages((current) => [...page.messages, ...current]);
      setMessageStart(page.offset);
      setError("");
    } catch (problem) {
      setError(describe(problem));
    } finally {
      setLoadingMore(false);
    }
  }, [targetId, selectedId, messageStart, loadingMore]);

  /* ── 选择变化就记下来（供下次启动还原） ── */
  useEffect(() => {
    if (providerId && targetId) writeLastSelection({ providerId, targetId });
  }, [providerId, targetId]);

  useEffect(() => {
    if (startedRef.current) return;
    startedRef.current = true;
    void api.logPerformance("ui.started", performance.now(), "ok");
  }, []);

  const selectProvider = useCallback((next: AiProviderId | "") => {
    setProviderId(next);
    // 平台变了，目标与会话都作废——留着会让界面显示另一个平台的会话。
    setTargets([]);
    setTargetId("");
    setSessions([]);
    setSelectedId("");
    setSelectedIds([]);
  }, []);

  const selectTarget = useCallback((next: string) => {
    setTargetId(next);
    setSessions([]);
    setSelectedId("");
    setSelectedIds([]);
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
      } catch (problem) {
        setError(describe(problem));
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
    [targetId]
  );

  const toggleSelected = useCallback((id: string) => {
    setSelectedIds((current) =>
      current.includes(id) ? current.filter((item) => item !== id) : [...current, id]
    );
  }, []);

  const toggleSelectAll = useCallback(() => {
    setSelectedIds((current) => (current.length === sessions.length ? [] : sessions.map((s) => s.id)));
  }, [sessions]);

  /** 批量操作的公共骨架：调用、刷新、清空选择、报错。 */
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
      } catch (problem) {
        setError(`${label}失败：${describe(problem)}`);
      } finally {
        setBusy(false);
      }
    },
    [targetId, sessions, selectedIds, view, query, loadSessions]
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
   * 还原。
   *
   * 后端**没有**批量还原命令（只有单条 `restore-session`），因此这里逐条发。
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
      setError(`还原完成：成功 ${refs.length - failed} 条，失败 ${failed} 条。`);
    } else {
      setError("");
    }
  }, [targetId, sessions, selectedIds, view, query, loadSessions]);

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
      } catch (problem) {
        setError(`重命名失败：${describe(problem)}`);
      }
    },
    [targetId, selectedId, view, query, loadSessions]
  );

  const duplicateCurrent = useCallback(async () => {
    if (!targetId || !selectedId) return;
    try {
      const copy = await api.duplicateSession(targetId, selectedId);
      await loadSessions(targetId, view, query);
      // 复制完直接选中副本——否则用户要在列表里找哪一条才是新的。
      setSelectedId(copy.id);
      setError("");
    } catch (problem) {
      setError(`复制失败：${describe(problem)}`);
    }
  }, [targetId, selectedId, view, query, loadSessions]);

  const exportCurrent = useCallback(
    async (format: ExportFormat) => {
      if (!targetId || !selectedId) return;
      try {
        // 不传路径：由后端挑一个确定的位置，再把实际路径回给我们显示。
        const result = await api.exportSession(targetId, selectedId, format, "");
        setError("");
        return result;
      } catch (problem) {
        setError(`导出失败：${describe(problem)}`);
        return null;
      }
    },
    [targetId, selectedId]
  );

  /**
   * 从某条消息处创建分支。
   *
   * `messageLine` 必须是**原始 JSONL 行号**：后端要靠它精确定位到原始记录。
   * 过滤后的可见序号会截错位置，因此界面上只有带 `sourceLine` 的消息才提供这个动作。
   */
  /**
   * 导出到**指定路径**（用户用原生对话框选了目录之后）。
   *
   * 与 `exportCurrent` 的区别：那个不传路径，由后端挑默认落点。两条路都要留着——
   * 默认落点让导出「一键可用」，指定路径让用户能放到自己想要的地方。
   */
  const exportCurrentTo = useCallback(
    async (format: ExportFormat, filePath: string) => {
      if (!targetId || !selectedId) return null;
      try {
        const result = await api.exportSession(targetId, selectedId, format, filePath);
        setError("");
        return result;
      } catch (problem) {
        setError(`导出失败：${describe(problem)}`);
        return null;
      }
    },
    [targetId, selectedId]
  );

  /**
   * 从某条消息处创建分支。
   *
   * `messageLine` 必须是**原始 JSONL 行号**：后端要靠它精确定位到原始记录。
   * 过滤后的可见序号会截错位置，因此界面上只有带 `sourceLine` 的消息才提供这个动作。
   */
  const branchFromMessage = useCallback(
    async (messageLine: number) => {
      if (!targetId || !selectedId) return;
      try {
        const branch = await api.branchSession(targetId, selectedId, messageLine);
        await loadSessions(targetId, view, query);
        setSelectedId(branch.id);
        setError("");
      } catch (problem) {
        setError(`分支失败：${describe(problem)}`);
      }
    },
    [targetId, selectedId, view, query, loadSessions]
  );

  /** 在系统文件管理器里打开当前会话所在的目录。 */
  const openCurrentFolder = useCallback(async () => {
    if (!targetId || !selectedId) return;
    try {
      await api.openSessionFolder(targetId, selectedId);
      setError("");
    } catch (problem) {
      setError(`打开目录失败：${describe(problem)}`);
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
      } catch (problem) {
        setError(`${permanent ? "彻底删除" : "移入回收站"}失败：${describe(problem)}`);
      }
    },
    [targetId, view, query, loadSessions]
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
      } catch (problem) {
        setError(`指定目录失败：${describe(problem)}`);
      }
    },
    [providerId, loadTargets]
  );

  const clearWslCodexHomeOverride = useCallback(
    async (distro: string) => {
      try {
        await api.clearWslCodexHome(distro);
        await loadTargets(providerId);
        setError("");
      } catch (problem) {
        setError(`清除目录覆盖失败：${describe(problem)}`);
      }
    },
    [providerId, loadTargets]
  );

  /* ── 快捷访问：只读缓存，进去就取一次 ── */
  const loadQuickAccess = useCallback(async () => {
    try {
      setQuickAccess(await api.listSessionQuickAccess());
      setError("");
    } catch (problem) {
      setError(describe(problem));
    }
  }, []);

  useEffect(() => {
    if (pane === "quick") void loadQuickAccess();
  }, [pane, loadQuickAccess]);

  /* ── 跨平台搜索 ──
   *
   * 后端一次只搜一个目标（`codex:search-sessions` 的 `targetId` 是必填的），因此
   * 跨平台 = 逐个目标发。并发发出去，并且**一个目标失败不拖垮整次搜索**——
   * 某台发行版进不去是常态，不该让本机的结果也看不见。
   */
  useEffect(() => {
    if (pane !== "search") return;
    const keyword = globalQuery.trim();
    if (!keyword) {
      setGlobalResults([]);
      return;
    }
    const seq = ++globalSeqRef.current;
    // 只对**范围内的**目标发请求：收窄筛选的价值一半在于少做无用功，
    // 全量搜完再在前端过滤掉，那些请求已经付过代价了。
    const scoped = searchTargetId
      ? targets.filter((target) => target.id === searchTargetId)
      : targets;
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
  }, [pane, globalQuery, targets, searchTargetId]);

  /**
   * 打开一条来自其它视图的结果。
   *
   * 先切到它所属的目标与平台，再选中会话，最后回到会话视图——否则用户会看到
   * 「选中了但列表里没有这条」。
   */
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
      void api.markSessionOpened(target.id, session.id).catch(() => undefined);
    },
    [providerId, targetId, selectProvider, selectTarget]
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
    messageTotal,
    hasEarlier: messageStart > 0,
    loadEarlierMessages,
    loadingMore,
    toggleFavorite,
    deleteSelected,
    purgeSelected,
    restoreSelected,
    renameSession,
    duplicateCurrent,
    exportCurrent,
    exportCurrentTo,
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
    globalResults,
    globalSearching,
    openAt,
    reloadTargets: () => loadTargets(providerId),
    reloadSessions: () => loadSessions(targetId, view, query)
  };
}
