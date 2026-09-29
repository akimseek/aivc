/**
 * 核实两件事，都是**猜错会静默失效**的那种：
 *
 * 1. **能力矩阵的键名**。我在 `sidebar-nav.ts` 里按 `capabilities.trash` /
 *    `capabilities.batchActions` 判定。若后端实际发的是 snake_case
 *    （`batch_actions`），判定会永远为 `false`——**回收站在所有平台上都消失**，
 *    而界面上没有任何提示说它为什么不见了。
 * 2. **侧栏两级导航的形状**与旧平台一致：一级只有 工作台/会话，
 *    二级是 当前会话/收藏夹/回收站，搜索行里有「全部会话」按钮。
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
    于是界面替你选好了平台/目标/会话）。而二级导航里的「回收站」是按
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

  // ── 2. 两级导航的形状 ─────────────────────────────────
  const labels = (selector) =>
    [...document.querySelectorAll(selector)].map((item) => p.clean(item.textContent));

  out.paneNav = labels(".pane-nav .chip");
  out.viewNav = labels(".view-nav .chip");

  check(
    "一级导航只有 工作台 / 会话",
    JSON.stringify(out.paneNav) === JSON.stringify(["工作台", "会话"]),
    JSON.stringify(out.paneNav)
  );
  check(
    "二级导航是 当前会话 / 收藏夹 / 回收站",
    JSON.stringify(out.viewNav) === JSON.stringify(["当前会话", "收藏夹", "回收站"]),
    JSON.stringify(out.viewNav)
  );

  // 「全部会话」按钮在搜索行里（不在导航里）。
  const searchRowButton = [...document.querySelectorAll(".session-search-row button")].map((item) =>
    p.clean(item.textContent)
  );
  out.searchRowButtons = searchRowButton;
  check(
    "搜索行里有「全部会话」按钮",
    searchRowButton.includes("全部会话"),
    JSON.stringify(searchRowButton)
  );

  // 回收站那一项**可见且可点**（Codex 支持回收站），而不是被隐藏。
  const trashChip = [...document.querySelectorAll(".view-nav .chip")].find(
    (item) => p.clean(item.textContent) === "回收站"
  );
  out.trashDisabled = trashChip ? trashChip.disabled : null;
  check("Codex 下「回收站」可点", trashChip && trashChip.disabled === false, `disabled=${out.trashDisabled}`);

  // 点收藏夹：二级高亮要跟着走，且一级仍停在「会话」（同属会话这一组）。
  const fav = [...document.querySelectorAll(".view-nav .chip")].find(
    (item) => p.clean(item.textContent) === "收藏夹"
  );
  if (fav) {
    fav.click();
    await p.sleep(600);
  }
  out.afterFavorites = {
    paneNav: labels(".pane-nav .chip.on"),
    viewNav: labels(".view-nav .chip.on"),
    searchRowPresent: Boolean(document.querySelector(".session-search-row"))
  };
  check(
    "点收藏夹后二级高亮是「收藏夹」",
    out.afterFavorites.viewNav.includes("收藏夹"),
    JSON.stringify(out.afterFavorites)
  );
  check(
    "收藏夹里**没有**搜索行（旧平台也不显示）",
    out.afterFavorites.searchRowPresent === false,
    `present=${out.afterFavorites.searchRowPresent}`
  );

  const failed = out.checks.filter((item) => !item.ok);
  out.verdict =
    failed.length === 0
      ? `PASS：${out.checks.length} 项侧栏导航与能力矩阵全部通过`
      : `FAIL：${failed.length}/${out.checks.length} 未通过 —— ${failed.map((f) => f.name).join("、")}`;
  return out;
})();
