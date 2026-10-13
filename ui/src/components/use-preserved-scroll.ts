import { useCallback, useLayoutEffect, useRef } from "react";

/** 在更新列表前记录位置，DOM 更新后恢复，防止整页换行时浏览器调整滚动锚点。 */
export function usePreservedScroll() {
  const ref = useRef<HTMLDivElement>(null);
  const snapshot = useRef<{ element: HTMLElement; top: number; left: number } | null>(null);
  const captureScroll = useCallback(() => {
    const element = ref.current?.closest<HTMLElement>(".settings-content");
    if (element) snapshot.current = { element, top: element.scrollTop, left: element.scrollLeft };
  }, []);

  useLayoutEffect(() => {
    const saved = snapshot.current;
    if (!saved) return;
    saved.element.scrollTop = saved.top;
    saved.element.scrollLeft = saved.left;
    snapshot.current = null;
  });

  return { ref, captureScroll };
}
