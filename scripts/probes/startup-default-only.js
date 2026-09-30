/**
 * 启动默认：**只读初始状态，不点任何东西**。
 *
 * # 为什么要单独一条最小探针
 *
 * 用户报「打开还是选中 Codex 平台、本机 Codex 目标」。查出来的原因是
 * localStorage 里存着上次的选择（`aivc.ui.lastSelection`），启动时被**还原**了
 * ——在用户眼里「还原」和「挑第一个」是同一件事：都不是他选的。
 *
 * 这条探针因此要在**用户的真实运行根**上跑（那里才有那个 localStorage），
 * 而且**只读不点**：不选目标、不开会话、不启动 CLI。
 */
(async () => {
  const p = window.__p;
  const out = { checks: [] };
  const check = (name, ok, detail) => out.checks.push({ name, ok: Boolean(ok), detail: detail ?? "" });
  const clean = (element) => p.clean(element.textContent ?? "");

  try {
    await p.waitForTargets();
    // 等首屏那次 listProviders / listTargets 落地。
    await p.sleep(3000);

    const pickers = [...document.querySelectorAll(".sidebar .pickers select")];
    out.localStorage = window.localStorage.getItem("aivc.ui.lastSelection");
    out.providerOptions = [...(pickers[0]?.options ?? [])].map((item) => item.value);
    out.targetOptions = [...(pickers[1]?.options ?? [])].map((item) => item.value);
    out.sidebarText = clean(document.querySelector(".sidebar") ?? document.body).slice(0, 240);
    // 批次三：主列表的行是 `.session-card`，正文头部是 `.history-head`。
    out.sessionRows = document.querySelectorAll(".sidebar .rows .session-card").length;
    out.sessionHead = Boolean(document.querySelector(".history-head"));

    // 平台：**下拉显示的文字应当是「请选择平台」**——这是用户直接看到的东西。
    //
    // 不能读 `.value`／`selected`：`value=""` 而没有匹配 option 时浏览器会把
    // 第一项标成 selected，看起来像「选了 Codex」。占位项修的就是这件事。
    out.providerDisplay = pickers[0]?.selectedOptions?.[0]?.textContent ?? "";
    check(
      "**平台下拉显示「请选择平台」**（不是第一个平台的名字）",
      out.providerDisplay.includes("请选择平台"),
      `显示="${out.providerDisplay}"`
    );

    // 再用可证明的推论确认应用状态确实是空的：平台真选了的话目标列表会被拉取。
    out.targetDisabled = pickers[1]?.disabled ?? null;
    out.realOptionCount = [...(pickers[1]?.options ?? [])].filter(
      (item) => item.value !== ""
    ).length;
    check(
      "**平台没有被选中**（因此目标没有被拉取：目标下拉禁用、没有任何真实目标）",
      out.targetDisabled === true &&
        out.realOptionCount === 0 &&
        /暂无可用目标/.test(out.sidebarText),
      `disabled=${out.targetDisabled} 真实目标=${out.realOptionCount}`
    );

    // 目标：下拉里只有一个空值占位项（「暂无可用目标」），值也是空的。
    check(
      "**目标没有被选中**（下拉值是空，只有占位项）",
      (pickers[1]?.value ?? "x") === "" && out.realOptionCount === 0,
      `value="${pickers[1]?.value}" 真实目标=${out.realOptionCount}`
    );
    check(
      "侧栏显示「暂无可用目标」/「请先在上面选择目标。」",
      /暂无可用目标|请先在上面选择目标/.test(out.sidebarText),
      out.sidebarText
    );
    check(
      "**没有自动选中会话**（没有会话行，也没有会话正文头部）",
      out.sessionRows === 0 && !out.sessionHead,
      `rows=${out.sessionRows} head=${out.sessionHead}`
    );
    /*
      关于 `aivc.ui.lastSelection`：
      
      **不能断言它不存在**——删掉写入方并不会删掉**已经存在**的那条旧数据。
      实测它还在（正是它导致了「打开就选中 Codex」）。
      
      应用现在既不读也不写它。这里只把它**读出来如实报告**，不断言。
    */
    out.staleSelection = out.localStorage;
  } catch (error) {
    out.threw = String((error && error.stack) || error).slice(0, 600);
    check("探针自身没有抛异常", false, out.threw);
  }

  const failed = out.checks.filter((item) => !item.ok);
  out.verdict =
    failed.length === 0
      ? `PASS：${out.checks.length} 项启动默认（真实运行根）全部通过`
      : `FAIL：${failed.length}/${out.checks.length} 未通过 —— ${failed.map((f) => f.name).join("、")}`;
  return out;
})();
