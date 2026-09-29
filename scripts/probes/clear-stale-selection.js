/**
 * 一次性维护：删掉 `aivc.ui.lastSelection` 这条**死数据**。
 *
 * # 它为什么是死数据
 *
 * 那个键原来存「上次选的平台 / 目标」，启动时被还原——用户报「打开就选中
 * Codex / 本机 Codex」正是它。修复时**写和读一起删了**，因此现在应用既不读
 * 也不写它，但**已经写在磁盘上的那条还在**。
 *
 * 留着它有害无益：它看起来像个设置项，以后有人翻到会以为「还原功能坏了」
 * 而去把它接回来。
 *
 * # 为什么要用探针删，而不是删 webview 目录
 *
 * localStorage 由 WebView2 自己管理（LevelDB）。删整个
 * `data/webview/` 会连带清掉其它存储与缓存——为了一个键付那个代价不值得。
 * 让应用在它自己的运行环境里 `removeItem` 是最小且正确的做法。
 *
 * # 用法（必须指向**真实运行根**）
 *
 * ```
 * .\scripts\app-probe.ps1 -Probe probes\clear-stale-selection.js -RunRoot D:\code\ai\aivc-rs
 * ```
 *
 * 跑完之后**再跑一次**（新进程、新 webview）确认它真的没回来——同一次会话里读到
 * `null` 只能证明内存里没了，不能证明落盘生效。
 */
(async () => {
  const p = window.__p;
  const out = { checks: [] };
  const check = (name, ok, detail) => out.checks.push({ name, ok: Boolean(ok), detail: detail ?? "" });
  /**
   * 要清的键。
   *
   * **两个**：`aivc.ui.lastSelection` 是当前常量的名字，
   * `aivc.lastSelection` 是更早那一版的——实测两个都在磁盘上
   * （第一版清理探针只删了前者，靠「剩余键」那条检查才发现后者还在）。
   * 现在没有任何代码读它们中的任何一个，两个都是死数据。
   */
  const KEYS = ["aivc.ui.lastSelection", "aivc.lastSelection"];

  try {
    await p.sleep(1500);

    out.before = Object.fromEntries(
      KEYS.map((key) => [key, window.localStorage.getItem(key)])
    );
    const present = KEYS.filter((key) => out.before[key] !== null);
    check(
      "至少有一个键存在（都已清过的话这一条会失败，属正常）",
      present.length > 0,
      JSON.stringify(out.before)
    );

    for (const key of KEYS) window.localStorage.removeItem(key);
    out.afterRemove = Object.fromEntries(KEYS.map((key) => [key, window.localStorage.getItem(key)]));
    check(
      "**两个键删完都读回 null**",
      KEYS.every((key) => out.afterRemove[key] === null),
      JSON.stringify(out.afterRemove)
    );

    // 顺便把存储里其它 `aivc.*` 键列出来，确认**只**删了这两个。
    out.remainingKeys = Object.keys(window.localStorage).filter((name) => name.startsWith("aivc"));
    check(
      "没有误删别的键",
      KEYS.every((key) => !out.remainingKeys.includes(key)),
      JSON.stringify(out.remainingKeys)
    );
  } catch (error) {
    out.threw = String((error && error.stack) || error).slice(0, 600);
    check("探针自身没有抛异常", false, out.threw);
  }

  const failed = out.checks.filter((item) => !item.ok);
  out.verdict =
    failed.length === 0
      ? `PASS：${out.checks.length} 项清理完成`
      : `DONE：${failed.length}/${out.checks.length} 项不成立（若已清过，属正常）`;
  return out;
})();
