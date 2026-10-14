import { useCallback, useEffect, useRef, useState } from "react";

import * as api from "../ipc/commands";
import type { AiMessage, AiSession, AiTarget } from "../types";
import { errorText } from "../components";

type DetailPage = {
  key: string;
  messages: AiMessage[];
  branchAnchors: number[];
  nextMessageRole?: AiMessage["role"];
  hasMore: boolean;
  total: number;
  totalKnown: boolean;
  offset: number;
  loading: boolean;
  error: string;
};

/** Load one page; discard responses after changing pages/sessions or closing. */
export function useSessionDetail(target: AiTarget | null, session: AiSession | null, pageSize: number) {
  const targetId = target?.id;
  const sessionId = session?.id;
  const identity = JSON.stringify([targetId, sessionId]);
  const [pagination, setPagination] = useState({ identity, page: 1 });
  const page = pagination.identity === identity ? pagination.page : 1;
  const [revision, setRevision] = useState(0);
  const key = JSON.stringify([identity, page, pageSize, revision]);
  const empty: DetailPage = {
    key, messages: [], branchAnchors: [], hasMore: false, total: 0,
    totalKnown: false, offset: (page - 1) * pageSize,
    loading: Boolean(targetId && sessionId), error: ""
  };
  const [result, setResult] = useState<DetailPage>(empty);
  const exactTotal = useRef<{ identity: string; revision: string; count: number } | undefined>(undefined);

  useEffect(() => {
    if (!targetId || !sessionId) return;
    let cancelled = false;
    const offset = (page - 1) * pageSize;
    void api.getSessionMessagesPage(targetId, sessionId, offset, pageSize + 1).then(
      (response) => {
        if (cancelled) return;
        const messages = response.messages.slice(0, pageSize);
        const known = response.messageCount > 0 || !response.hasMore;
        if (known && response.fileRevision) {
          exactTotal.current = { identity, revision: response.fileRevision, count: response.messageCount };
        }
        const cached = exactTotal.current;
        const matchingTotal = cached?.identity === identity && cached.revision === response.fileRevision &&
          cached.count >= response.offset + response.messages.length + (response.hasMore ? 1 : 0)
          ? cached.count : undefined;
        setResult({
          key, messages,
          branchAnchors: (response.branchAnchors ?? []).filter((line) => messages.some((message) => message.sourceLine === line)),
          nextMessageRole: response.messages[pageSize]?.role,
          // An extra item proves another page exists, including at EOF.
          hasMore: response.messages.length > pageSize || response.hasMore,
          total: known ? response.messageCount : matchingTotal ?? 0,
          totalKnown: known || matchingTotal !== undefined,
          offset: response.offset, loading: false, error: ""
        });
      },
      (problem) => {
        if (!cancelled) setResult({
          key, messages: [], branchAnchors: [], hasMore: false, total: 0,
          totalKnown: false, offset, loading: false, error: errorText(problem)
        });
      }
    );
    return () => { cancelled = true; };
  }, [targetId, sessionId, identity, page, pageSize, key]);

  // A new request must never render the previous page's branch buttons.
  const current = result.key === key ? result : empty;
  const setPage = useCallback((next: number) => {
    setPagination({ identity, page: Math.max(1, Math.floor(next)) });
  }, [identity]);
  return {
    ...current, page, pageSize, setPage,
    reload: () => setRevision((value) => value + 1),
    pageCount: Math.max(1, Math.ceil(current.total / pageSize), current.hasMore ? page + 1 : page)
  };
}
