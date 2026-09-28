/**
 * **校验 `vendor-audit.js` 这把尺子本身准不准。**
 *
 * 我正拿 `vendor-audit.js` 的读数（0 家）当作「用户没配供应商」的结论依据，
 * 那就必须先证明它**能测出非零**。否则「测出来是 0」可能只是探针写错了。
 *
 * 做法：存一家供应商 → 立刻列一次 → 断言能看到它 → 删掉它。
 * 全程只在**调用方指定的运行根**里操作（本次是临时目录之外的真实根会被要求显式确认）。
 */
(async () => {
  const p = window.__p;
  const out = { steps: [] };

  // **每一步都套 try/catch。** 上一版没套，异常只留下「求值结果抛异常」，
  // 看不出是哪一步——排查这种探针时，看不到原因比失败本身更费时间。
  const safe = async (name, action) => {
    try {
      return await action();
    } catch (error) {
      out[`${name}Error`] = String((error && error.message) || error);
      return null;
    }
  };

  const before = await safe("list", () => p.invoke("vendor:list", {}));
  out.before = (before || []).length;

  const saved = await safe("save", () =>
    p.invoke("vendor:save", {
      providerId: "codex",
      name: "尺子校验供应商",
      apiKey: "sk-calibration",
      apiBaseUrl: "http://127.0.0.1:1",
      enabled: true,
      sort: 99
    })
  );
  out.savedId = saved ? String(saved.id).slice(0, 8) : null;

  const after = await safe("list2", () => p.invoke("vendor:list", {}));
  out.after = (after || []).length;
  const found = (after || []).find((item) => item.name === "尺子校验供应商");
  out.found = Boolean(found);
  out.foundProvider = found ? found.providerId : null;
  // 只报长度，不打密钥本身。
  out.foundKeyLength = found && found.apiKey ? String(found.apiKey).length : 0;

  // 收尾：把校验用的那家删掉，别污染数据。
  //
  // 参数名是 **`id`** 而不是 `vendorId`（`vendor:delete` 走 `require_string(args.get("id"))`；
  // `vendorId` 是另外几条命令用的）。用错名字会得到「参数无效：id」——
  // 本探针第一次就是这么失败的，而它原本应该报「尺子准」。
  if (found) {
    await safe("delete", () => p.invoke("vendor:delete", { id: found.id }));
    const cleaned = await safe("list3", () => p.invoke("vendor:list", {}));
    out.afterCleanup = (cleaned || []).length;
  }

  out.verdict =
    out.found && out.after === out.before + 1 && out.afterCleanup === out.before
      ? "尺子准：存进去能列出来、删掉能回到原样"
      : "尺子不准：列不出来或清理失败，vendor-audit 的读数不可信";
  return out;
})();
