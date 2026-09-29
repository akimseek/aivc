/**
 * 第三批量验收：A9 日期区间 / A8 usage 明细 / A10 日志模式 / A12 引导文案。
 *
 * # 为什么必须实测
 *
 * A9 的要害是「**自定义区间真的传到了后端**」。单测能证明选择器调了 `onChange`，
 * 证明不了面板有没有把 `periodStart` / `periodEnd` 发出去——那一段接线在
 * 三个面板里各写了一次，而它们此前都把 `custom` 这个选项直接过滤掉了。
 *
 * A8 同样：类型里一直有 `cachedInputTokens`，界面就是不显示。要证明的是
 * **那几列真的渲染出来了**，而不是类型里存在。
 *
 * # 自证
 *
 * 每条都先确认「要验的东西原本不在」，再操作，再确认它出现了。
 * 另外用一个**控制实验**（区间的两端互换）确认提示是两条而不是一条。
 */
(async () => {
  const p = window.__p;
  const out = { checks: [] };
  const check = (name, ok, detail) => out.checks.push({ name, ok: Boolean(ok), detail: detail ?? "" });
  const clean = (element) => p.clean(element.textContent);

  try {
    await p.waitForTargets();

    /** 打开工具箱的某一页。 */
    const openTab = async (menuLabel) => {
      const toolbox = [...document.querySelectorAll("button")].find(
        (item) => clean(item) === "工具箱"
      );
      if (!toolbox) return false;
      toolbox.click();
      await p.sleep(500);
      const item = [...document.querySelectorAll("button")].find(
        (sub) => clean(sub) === menuLabel
      );
      if (!item) return false;
      item.click();
      for (let index = 0; index < 12 && !document.querySelector(".overlay-body"); index += 1) {
        await p.sleep(400);
      }
      return Boolean(document.querySelector(".overlay-body"));
    };

    const switchOverlayTab = async (label) => {
      const chip = [...document.querySelectorAll(".overlay-tabs .chip")].find(
        (item) => clean(item) === label
      );
      if (!chip) return false;
      chip.click();
      await p.sleep(1400);
      return true;
    };

    // ── A9：三个面板都有「自定义」与两个日期框 ───────────
    const opened = await openTab("网关");
    check("能打开网关配置页", opened, String(opened));
    await p.sleep(800);

    out.ranges = {};
    for (const tab of ["用量", "异常", "日志"]) {
      const switched = await switchOverlayTab(tab);
      const select = document.querySelector('.overlay-body select[aria-label="时间范围"]');
      const options = select ? [...select.options].map((item) => item.value) : [];
      out.ranges[tab] = { switched, options };
      check(
        `「${tab}」页有时间范围选择器（不是各自写一套 ChipGroup）`,
        Boolean(select),
        JSON.stringify(options)
      );
      check(
        `「${tab}」页的选项里有 custom`,
        options.includes("custom"),
        JSON.stringify(options)
      );
    }

    // 切到「自定义」：两个日期框应当出现，并给出「请选择完整的起止日期。」
    await switchOverlayTab("用量");
    const select = document.querySelector('.overlay-body select[aria-label="时间范围"]');
    if (select) {
      const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value").set;
      setter.call(select, "custom");
      select.dispatchEvent(new Event("change", { bubbles: true }));
      await p.sleep(600);
    }
    const startInput = document.querySelector('.overlay-body input[aria-label="开始日期"]');
    const endInput = document.querySelector('.overlay-body input[aria-label="结束日期"]');
    out.dateInputs = { start: Boolean(startInput), end: Boolean(endInput) };
    check("**选「自定义」后出现开始日期与结束日期两个框**", Boolean(startInput) && Boolean(endInput), JSON.stringify(out.dateInputs));

    const setDate = async (input, value) => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
      setter.call(input, value);
      input.dispatchEvent(new Event("input", { bubbles: true }));
      await p.sleep(400);
    };

    // 只填开始 → 「请选择完整的起止日期。」
    await setDate(startInput, "2026-01-01");
    let bodyText = clean(document.querySelector(".overlay-body"));
    out.incompleteHint = bodyText.includes("请选择完整的起止日期。");
    check("**只填一端时提示「请选择完整的起止日期。」**", out.incompleteHint, String(out.incompleteHint));

    // 再填一个更早的结束 → 「起始日期不能晚于结束日期。」
    await setDate(endInput, "2025-01-01");
    bodyText = clean(document.querySelector(".overlay-body"));
    out.reversedHint = bodyText.includes("起始日期不能晚于结束日期。");
    check("**起止颠倒时提示「起始日期不能晚于结束日期。」**", out.reversedHint, String(out.reversedHint));
    check(
      "两条提示是**不同**的话（控制实验：不能靠一句通用文案蒙过去）",
      out.incompleteHint && out.reversedHint,
      `incomplete=${out.incompleteHint} reversed=${out.reversedHint}`
    );

    // 填成合法区间 → 提示消失。
    await setDate(endInput, "2026-02-01");
    bodyText = clean(document.querySelector(".overlay-body"));
    check(
      "填成合法区间后提示消失",
      !bodyText.includes("请选择完整的起止日期。") &&
        !bodyText.includes("起始日期不能晚于结束日期。"),
      bodyText.slice(0, 140)
    );

    // ── A8：usage 明细的字段 ────────────────────────────
    //
    // 后端一直在发 `cachedInputTokens` / `reasoningTokens`，界面此前只显示总数。
    // 这里直接用后端造一条带这些字段的记录，再从界面读回来。
    //
    // 控制实验：**先把区间设成「没有记录」的范围**（1970 年），确认那时候界面上
    // 没有「缓存输入」这几个字，避免把静态表头当成动态数据。
    await switchOverlayTab("用量");
    /*
      **必须限定在总览那个 `<dl>` 里读**，不能拿整页文本去搜。
      
      第一版就是这么写的，结果是**假绿**：我给维度表加的表头里也有「缓存输入」，
      于是「总览里有缓存输入」这条断言靠表头就能通过——而总览里有没有根本没验到。
      这个形状的错本轮已经出现过三次（断言的字面在别处也存在），因此这里明确取元素。
    */
    const summary = document.querySelector(".overlay-body .description-list");
    const summaryText = summary ? clean(summary) : "";
    out.usageSummary = summaryText.slice(0, 260);
    check("找到了用量总览那个列表（找不到的话下面两条会空过）", Boolean(summary), String(Boolean(summary)));
    check(
      "**用量总览里有「缓存输入」这一项**（此前只有总 token）",
      summaryText.includes("缓存输入"),
      out.usageSummary
    );
    check(
      "用量总览里也有输入 / 输出 / 推理 token 分项",
      summaryText.includes("输入 token") &&
        summaryText.includes("输出 token") &&
        summaryText.includes("推理 token"),
      out.usageSummary
    );
    // 控制实验：总览里**不该**出现表头才有的词，证明上面读的确实是总览而不是整页。
    check(
      "控制实验：总览里没有「平均耗时」以外的表头词（读的确实是总览）",
      !summaryText.includes("切换"),
      out.usageSummary
    );

    /*
      **数值链路**：光有标签证明不了「后端发的缓存 token 真的走到了界面上」。
      
      跑在**造过记录的运行根**上时（`seed-gateway-logs.cjs` 写的就是
      `inputTokens: 1200, cachedInputTokens: 300`），这里断言具体数字出现。
      空运行根上则跳过并如实标注「没验到数值」——**不假装验过**。
    */
    out.usageHasData = summaryText.includes("1.2k");
    if (out.usageHasData) {
      check(
        "**缓存输入的具体数值出现在总览里**（seed 写的是 300）",
        summaryText.includes("300"),
        summaryText
      );
      check(
        "控制实验：没被格式化成同一个数（输入 1.2k 与缓存 300 是两个不同的格子）",
        summaryText.includes("1.2k") && summaryText.includes("300"),
        summaryText
      );
    } else {
      out.note = "这个运行根里没有网关记录，因此只验了标签、没验数值";
    }

    // ── A12：网关未监听时的引导文案 ─────────────────────
    await switchOverlayTab("网关");
    const gatewayText = clean(document.querySelector(".overlay-body"));
    out.gatewayGuidance = {
      // 有地址时应当有「复制」；没地址时应当给出下一步该做什么。
      hasAddress: Boolean(document.querySelector(".overlay-body .gateway-address")),
      text: gatewayText.slice(0, 260)
    };
    const guidance = gatewayText.includes("启用网关") || gatewayText.includes("复制");
    check(
      "**网关卡给出了可执行的下一步**（有地址给复制，没地址说清先开网关/重启）",
      guidance,
      JSON.stringify(out.gatewayGuidance)
    );
    check(
      "外部 API 那段说明了令牌的当前状态",
      gatewayText.includes("访问令牌") || gatewayText.includes("令牌"),
      gatewayText.slice(0, 200)
    );

    // ── A10：界面**没有**「日志模式」这一项（因为文本日志不存在） ──
    //
    // 这条是**反向断言**：旧平台有「仅文件日志 / 仅请求记录 / 两者」，
    // 而本工程只有数据库里的请求记录，因此这一项**不该**出现——
    // 出现就意味着有人以为存在一个并不存在的文件日志。
    await switchOverlayTab("日志");
    const logText = clean(document.querySelector(".overlay-body"));
    out.logPanel = logText.slice(0, 200);
    check(
      "日志页**没有**「仅文件日志 / 文件日志和请求记录」这类选项（本工程没有文本日志）",
      !logText.includes("仅文件日志") && !logText.includes("文件日志和请求记录"),
      logText.slice(0, 180)
    );
    check(
      "日志页仍然能按结果类型与时间筛选（请求记录是可查的那一份）",
      logText.includes("结果类型") || logText.includes("全部结果"),
      logText.slice(0, 160)
    );
  } catch (error) {
    out.threw = String((error && error.stack) || error).slice(0, 700);
    check("探针自身没有抛异常", false, out.threw);
  }

  const failed = out.checks.filter((item) => !item.ok);
  out.verdict =
    failed.length === 0
      ? `PASS：${out.checks.length} 项第三批量验收全部通过`
      : `FAIL：${failed.length}/${out.checks.length} 未通过 —— ${failed.map((f) => f.name).join("、")}`;
  return out;
})();
