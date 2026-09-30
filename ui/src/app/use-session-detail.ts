import { useCallback, useEffect, useRef, useState } from "react";

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
 *    然后第二次的响应把它盖掉。每次 `load` 取一个自增的请求号，`await` 回来后
 *    比对：不是最新一次就整段丢弃（与 `use-console` 里搜索同一手法）；
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
  /** 请求自增号：每次 `load` 领取一个，只有最新一次允许把结果写进状态。 */
  const seqRef = useRef(0);

  /** 换了会话就回到第 1 页并清空——否则会停在一条不存在的页上。 */
  useEffect(() => {
    setPage(1);
    setMessages([]);
    setTotal(0);
    setError("");
  }, [session?.id, target?.id]);

  const load = useCallback(async () => {
    if (!target || !session) {
      /*
        清空也要作废在途请求：会话切换后旧响应才回来时，它会把**上一个会话**的
        消息塞回已清空的界面。顺带收掉 loading——这次调用之后没有更新的请求，
        在途那次已经作废，不会再有人去关它。
      */
      seqRef.current += 1;
      setMessages([]);
      setTotal(0);
      setLoading(false);
      return;
    }
    const seq = ++seqRef.current;
    setLoading(true);
    try {
      const offset = (page - 1) * pageSize;
      const result = await api.getSessionMessagesPage(
        target.id,
        session.id,
        offset,
        pageSize
      );
      /*
        后发的请求可能先回：不是最新一次就整段丢弃，**连收尾都不做**。
        此刻的 loading 属于更新的那次请求，关掉它界面会闪一下「没在加载」；
        messages / total / error 同理，旧响应一律不许再碰。
      */
      if (seq !== seqRef.current) return;
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
      // 失败按同一规则丢弃：旧请求的失败不该报错，更不该清掉已落地的新结果。
      if (seq !== seqRef.current) return;
      setMessages([]);
      setError(problem instanceof Error ? problem.message : String(problem));
    } finally {
      if (seq === seqRef.current) setLoading(false);
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
