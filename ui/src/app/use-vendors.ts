import { useEffect, useState } from "react";

import * as api from "../ipc/commands";
import { onVendorsChanged } from "../ipc/vendor-events";
import type { ApiVendor } from "../types";

/** 供应商名称和候选池来自同一份实时清单，丢弃变更前尚未返回的读取结果。 */
export function useVendors(providerId: string, refreshKey = "") {
  const [snapshot, setSnapshot] = useState({ providerId: "", vendors: [] as ApiVendor[] });
  useEffect(() => {
    let cancelled = false;
    let revision = 0;
    const load = async () => {
      const current = ++revision;
      if (!providerId) { setSnapshot({ providerId, vendors: [] }); return; }
      try {
        const list = await api.listVendors(providerId);
        if (!cancelled && current === revision) setSnapshot({ providerId, vendors: list });
      } catch {
        if (!cancelled && current === revision) setSnapshot({ providerId, vendors: [] });
      }
    };
    const stop = onVendorsChanged(() => { void load(); });
    void load();
    return () => { cancelled = true; stop(); };
  }, [providerId, refreshKey]);
  return snapshot.providerId === providerId ? snapshot.vendors : [];
}
