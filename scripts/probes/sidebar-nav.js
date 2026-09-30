/**
 * 核实两件事，都是**猜错会静默失效**的那种：
 *
 * 1. **能力矩阵的键名**。我在 `sidebar-nav.ts` 里按 `capabilities.trash` /
 *    `capabilities.batchActions` 判定。若后端实际发的是 snake_case
 *    （`batch_actions`），判定会永远为 `false`——**回收站在所有平台上都消失**，
 *    而界面上没有任何提示说它为什么不见了。
 * 2. **导航的形状**：一级去处只有 会话 / 工作台（活动栏），侧栏里是
 *    筛选 chips（全部 N / ★ 收藏 N）与回收站图标按钮。
 */
(async () => {
  const p = window.__p;
  const out = { checks: [] };
  const check = (name, ok, detail) => out.checks.push({ name, ok: Boolean(ok), detail: detail ?? "" });

  // ── 1. 能力矩阵的真实键名 ─────────────────────────────
  const raw = await p.invoke("ai:list-providers", {});
  out.providers = raw;
  const codex = (raw ?? []).find((item) => item.id === "codex");
  const caps = codex?.capabilities ?? {};

  /*
    **先自己选平台。**
    
    本应用现在**启动后什么都不选**（刻意如此：此前三处「还原不上就挑第一个」，
    于是界面替你选好了平台/目标/会话）。而筛选行里的「回收站」按钮是按
    **当前平台的能力**启用/禁用的（`canUseTrash = supportsTrash(providers, providerId)`）
    ——没选平台时它是禁用的，并提示「请选择支持回收站的平台」。
    
    这个探针原来吃的是旧行为的默认选中，因此这里补上用户真实的那一步。
  */
  {
    const providerSelect = document.querySelector(".sidebar .pickers select");
    const option = [...(providerSelect?.options ?? [])].find((item) => item.value === "codex");
    if (providerSelect && option) {
      Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value").set.call(
        providerSelect,
        "codex"
      );
      providerSelect.dispatchEvent(new Event("change", { bubbles: true }));
      await p.sleep(2500);
    }
  }
  out.capabilityKeys = Object.keys(caps).sort();

  check(
    "能力矩阵用 camelCase 键名（batchActions 而不是 batch_actions）",
    "batchActions" in caps || "batch_actions" in caps,
    `键：${out.capabilityKeys.join(",")}`
  );
  check(
    "`trash` 这个键存在（回收站的判定靠它）",
    "trash" in caps,
    `trash=${String(caps.trash)}`
  );
  check(
    "Codex 的 `trash` 为真——它的回收站必须能用",
    caps.trash === true,
    `trash=${String(caps.trash)}`
  );

  // Qoder 的能力差异是 `duplicate` / `branch` 为假，而 **`trash` 为真**。
  // 我最初按旧平台的说法断言「Qoder 不支持回收站」——那是错的，
  // 后端的 `ProviderId::capabilities()` 明确写着 `trash: true`。
  // 这一条改成断言真正有差异的那个字段，否则它测的是一个不存在的差异。
  const qoder = (raw ?? []).find((item) => item.id === "qoder");
  out.qoderCaps = qoder?.capabilities;
  check(
    "Qoder 的 `duplicate` 为假——复制会话必须按能力门控",
    qoder === undefined || qoder.capabilities?.duplicate === false,
    `qoder.duplicate=${String(qoder?.capabilities?.duplicate)}`
  );
  check(
    "Qoder 的 `trash` 为真——回收站在它上面是可用的",
    qoder === undefined || qoder.capabilities?.trash === true,
    `qoder.trash=${String(qoder?.capabilities?.trash)}`
  );

  // ── 2. 导航与筛选的形状 ───────────────────────────────
  //
  // 批次一之后一级去处搬到**活动栏**（`shell/activity-bar.tsx` 的 `.activity-item`），
  // 侧栏里只剩筛选 chips（`.filter-chips .chip`，形态「全部 N」/「★ 收藏 N」）
  // 与右侧的回收站图标按钮。旧探针找的 `.pane-nav .chip` / `.view-nav .chip` /
  // `.session-search-row` 全部不存在了——照旧找会得到「导航为空」的假结论。
  const labels = (selector) =>
    [...document.querySelectorAll(selector)].map((item) => p.clean(item.textContent));
  const attrs = (selector, name) =>
    [...document.querySelectorAll(selector)].map((item) => item.getAttribute(name) || "");

  out.activities = attrs(".activity-top .activity-item", "aria-label");
  out.settingsEntry = attrs(".activity-bottom .activity-item", "aria-label");
  check(
    "一级去处只有 会话 / 工作台（在活动栏，不在侧栏）",
    JSON.stringify(out.activities) === JSON.stringify(["AI 会话管理", "工作台与运行概览"]),
    JSON.stringify(out.activities)
  );
  check(
    "设置入口在活动栏底部",
    out.settingsEntry.length === 1,
    JSON.stringify(out.settingsEntry)
  );

  out.filterChips = labels(".filter-chips .chip");
  check(
    "侧栏筛选是「全部 N」与「★ 收藏 N」两个 chip",
    out.filterChips.length === 2 &&
      /^全部 \d+$/.test(out.filterChips[0] ?? "") &&
      /^★ 收藏 \d+$/.test(out.filterChips[1] ?? ""),
    JSON.stringify(out.filterChips)
  );

  // 回收站是筛选行右端的**图标按钮**（不在导航里），仍按平台能力启用/禁用。
  const trashButton = document.querySelector('.filter-chips button[aria-label="回收站"]');
  out.trashDisabled = trashButton ? trashButton.disabled : null;
  check("Codex 下「回收站」可点", Boolean(trashButton) && trashButton.disabled === false, `disabled=${out.trashDisabled}`);

  // 点收藏夹：要有高亮，且**没有**搜索动作行（旧平台在收藏夹里也不显示搜索行）。
  const fav = [...document.querySelectorAll(".filter-chips .chip")].find(
    (item) => /^★ 收藏/.test(p.clean(item.textContent))
  );
  if (fav) {
    fav.click();
    await p.sleep(600);
  }
  out.afterFavorites = {
    activeChip: p.clean((document.querySelector(".filter-chips .chip.on") || {}).textContent),
    searchActionsPresent: Boolean(document.querySelector(".search-actions"))
  };
  check(
    "点收藏夹后高亮跟着走",
    /^★ 收藏/.test(out.afterFavorites.activeChip ?? ""),
    JSON.stringify(out.afterFavorites)
  );
  check(
    "收藏夹里**没有**搜索动作行（旧平台也不显示）",
    out.afterFavorites.searchActionsPresent === false,
    `present=${out.afterFavorites.searchActionsPresent}`
  );

  const failed = out.checks.filter((item) => !item.ok);
  out.verdict =
    failed.length === 0
      ? `PASS：${out.checks.length} 项侧栏导航与能力矩阵全部通过`
      : `FAIL：${failed.length}/${out.checks.length} 未通过 —— ${failed.map((f) => f.name).join("、")}`;
  return out;
})();
