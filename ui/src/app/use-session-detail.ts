import { useCallback, useEffect, useState } from "react";

import * as api from "../ipc/commands";
import type { AiMessage, AiSession, AiTarget } from "../types";

/**
 * 会话详情的分页取数。
 *
 * # 为什么单独一个 hook
 *
 * 详情弹层要的东西比主区那条线程多：**任意一页**、**每页数量可变**、**精确总数**。
 * 这三个凑在一起有三处会出错：
 *
 * 1. **换页时旧请求可能后回**。用户连点五次「下一页」，第五次的响应可能先到，
 *    然后第二次的响应把它盖掉。用一个自增号解决，与 `use-console` 里搜索同一手法；
 * 2. **总数只在第一页可靠**。后端每次都会带 `messageCount`，但切页时不该让它抖动——
 *    页码算错一位，用户就跳到不存在的那一页；
 * 3. **切换会话必须重置到第 1 页**。不重置的话，从一条 300 条的会话切到一条 20 条的，
 *    会停在第 5 页然后显示空白。
 */
export function useSessionDetail(
  target: AiTarget | null,
  session: AiSession | null,
  pageSize: number
) {
  const [page, setPage] = useState(1);
  const [messages, setMessages] = useState<AiMessage[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  /** 每次「重新加载」自增，用来强制重跑同一个页码。 */
  const [revision, setRevision] = useState(0);

  /** 换了会话就回到第 1 页并清空——否则会停在一条不存在的页上。 */
  useEffect(() => {
    setPage(1);
    setMessages([]);
    setTotal(0);
    setError("");
  }, [session?.id, target?.id]);

  const load = useCallback(async () => {
    if (!target || !session) {
      setMessages([]);
      setTotal(0);
      return;
    }
    setLoading(true);
    try {
      const offset = (page - 1) * pageSize;
      const result = await api.getSessionMessagesPage(
        target.id,
        session.id,
        offset,
        pageSize
      );
      setMessages(result.messages);
      /**
       * 后端每次都带 `messageCount`，但**只认非零值**。
       *
       * `0` 在后端的语义是「本次无从得知」，与「真的有 0 条」不是一回事。
       * 直接用 `setTotal(result.messageCount)` 会让总数在切页时被清零，
       * 于是页码算出来永远是 1，用户按「下一页」跳到不存在的位置。
       */
      if (result.messageCount > 0) setTotal(result.messageCount);
      setError("");
    } catch (problem) {
      setMessages([]);
      setError(problem instanceof Error ? problem.message : String(problem));
    } finally {
      setLoading(false);
    }
  }, [target, session, page, pageSize, revision]);

  useEffect(() => {
    void load();
  }, [load]);

  return {
    page,
    pageSize,
    messages,
    total,
    loading,
    error,
    /** 页码从 1 起。 */
    setPage,
    /** 强制重跑当前页。 */
    reload: () => setRevision((value) => value + 1),
    /** 页数（至少 1，否则「第 1 / 0 页」）。 */
    pageCount: Math.max(1, Math.ceil(total / pageSize))
  };
}
