/**
 * 「切换目标 → 加载历史会话」这条链的**逐步取证**。
 *
 * # 为什么要逐步记
 *
 * 用户报「就算（默认）选了第一项目标，也不会加载历史会话」。要判断是链断了还是别的，
 * 得看清每一步之后各自是什么状态：平台选了没有、目标列表有没有回来、目标选中没有、
 * 会话请求发了没有、行数是多少。
 *
 * 只报最终「没有会话」是不够的——那有好几种完全不同的原因。
 */
(async () => {
  const p = window.__p;
  const out = { steps: [] };
  const step = (name, detail) => out.steps.push({ name, detail: detail ?? "" });
  const clean = (element) => p.clean(element.textContent ?? "");
  const pickers = () => [...document.querySelectorAll(".sidebar .pickers select")];
  // 批次三：主列表的行是 `.session-card`（`features/sessions/session-card.tsx`）。
  const rows = () => document.querySelectorAll(".sidebar .rows .session-card").length;
  const setSelect = async (element, value) => {
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value").set.call(element, value);
    element.dispatchEvent(new Event("change", { bubbles: true }));
  };

  try {
    await p.waitForTargets();
    await p.sleep(2500);

    step("0. 启动后", JSON.stringify({
      providerDisplay: pickers()[0]?.selectedOptions?.[0]?.textContent ?? "",
      providerOptions: [...(pickers()[0]?.options ?? [])].map((o) => o.value),
      targetValue: pickers()[1]?.value ?? "",
      targetDisabled: pickers()[1]?.disabled,
      targetOptions: [...(pickers()[1]?.options ?? [])].map((o) => o.value),
      rows: rows(),
      sidebar: clean(document.querySelector(".sidebar") ?? document.body).slice(0, 150)
    }));

    // ① 选平台
    const codex = [...(pickers()[0]?.options ?? [])].find((o) => o.value === "codex");
    if (codex) await setSelect(pickers()[0], "codex");
    await p.sleep(3500);
    step("1. 选了平台 codex 之后", JSON.stringify({
      providerDisplay: pickers()[0]?.selectedOptions?.[0]?.textContent ?? "",
      targetDisabled: pickers()[1]?.disabled,
      targetOptions: [...(pickers()[1]?.options ?? [])].map((o) => o.value),
      targetValue: pickers()[1]?.value ?? "",
      rows: rows()
    }));

    // ② 选目标（第一个真实目标）
    const target = [...(pickers()[1]?.options ?? [])].find((o) => o.value !== "");
    if (target) await setSelect(pickers()[1], target.value);
    await p.sleep(4000);
    step("2. 选了目标之后", JSON.stringify({
      picked: target?.value ?? "(没有真实目标 ↯ 这一步没执行)",
      targetValue: pickers()[1]?.value ?? "",
      rows: rows(),
      sessionHead: Boolean(document.querySelector(".history-head")),
      sidebar: clean(document.querySelector(".sidebar") ?? document.body).slice(0, 180)
    }));

    // ③ 再等一会儿，看是不是只是慢
    await p.sleep(4000);
    step("3. 再等 4 秒之后", JSON.stringify({ rows: rows(), sidebar: clean(document.querySelector(".sidebar") ?? document.body).slice(0, 150) }));

    out.verdict = `rows=${rows()}（目标=${pickers()[1]?.value || "(空)"}）`;
  } catch (error) {
    out.threw = String((error && error.stack) || error).slice(0, 700);
  }
  return out;
})();
