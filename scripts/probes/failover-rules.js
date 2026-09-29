/**
 * 第二批量验收：A1 规则自定义返回 / A2 试规则。
 *
 * # 为什么必须实测
 *
 * 这一批有一条**跨语言的同源要求**：`gateway:test-failover-rule` 判断命中与否，
 * 必须与**转发路径**用的是同一份逻辑（core 里的 `match_rule`）。单测只能证明
 * 「命令被调了、返回值被渲染了」，证明不了「它和真实转发是一条路」。
 *
 * 因此这里做的是：**用同一组输入分别问「转发路径」与「试规则命令」**，
 * 看两者是否一致。转发路径没法在探针里真的发一次上游请求，但可以退一步——
 * 用**保存后的规则**与**草稿规则**问同一个后端，两者必须给一样的答案，
 * 且 `matchedBy` 要与手工推导一致。
 *
 * # 自证
 *
 * 每条都先算出**期望值**（按规则语义手推），再与后端返回值比对；
 * 一条全部不命中的用例也在里面，否则「都返回 true」也能过。
 */
(async () => {
  const p = window.__p;
  const out = { checks: [] };
  const check = (name, ok, detail) => out.checks.push({ name, ok: Boolean(ok), detail: detail ?? "" });
  const clean = (element) => p.clean(element.textContent);

  try {
    await p.waitForTargets();

    /**
     * 试一条规则。
     *
     * 这里**直接打后端**（与界面走同一条命令），因为要验的是命令语义；
     * 界面接线由单测覆盖，最后再用界面点一次（见下）。
     */
    const test = (rule, statusCode, message) =>
      p.invoke("gateway:test-failover-rule", {
        rule,
        ...(statusCode === undefined ? {} : { statusCode }),
        ...(message === undefined ? {} : { message })
      });

    // ── 状态码命中 ──────────────────────────────────────
    const byStatus = await test(
      { scope: "global", pattern: "never-matches-this", statusCodes: [429] },
      429
    );
    out.byStatus = byStatus;
    check(
      "状态码命中 → matched / matchedBy=status",
      byStatus?.matched === true && byStatus?.matchedBy === "status",
      JSON.stringify(byStatus)
    );

    // ── 文本命中（不区分大小写） ─────────────────────────
    const byPattern = await test(
      { scope: "global", pattern: "Rate Limit" },
      undefined,
      "upstream said RATE LIMIT exceeded"
    );
    out.byPattern = byPattern;
    check(
      "**文本包含命中且不区分大小写**",
      byPattern?.matched === true && byPattern?.matchedBy === "pattern",
      JSON.stringify(byPattern)
    );

    // ── 都不命中 ────────────────────────────────────────
    //
    // 这一条是**控制实验**：没有它，「一律返回 true」也能骗过上面两条。
    const noHit = await test(
      { scope: "global", pattern: "rate limit", statusCodes: [429] },
      200,
      "all good"
    );
    out.noHit = noHit;
    check(
      "都不命中 → matched=false / matchedBy=none（控制实验）",
      noHit?.matched === false && noHit?.matchedBy === "none",
      JSON.stringify(noHit)
    );

    // ── 状态码与文本都命中时优先报状态码 ─────────────────
    //
    // 因为状态码是**精确**匹配，对用户更有用。
    const both = await test(
      { scope: "global", pattern: "boom", statusCodes: [400] },
      400,
      "boom happened"
    );
    out.both = both;
    check(
      "两者都命中时报状态码（精确匹配优先）",
      both?.matched === true && both?.matchedBy === "status",
      JSON.stringify(both)
    );

    // ── 空模式不能只靠文本命中 ───────────────────────────
    const emptyPattern = await test({ scope: "global", pattern: "", statusCodes: [500] }, 200, "x");
    out.emptyPattern = emptyPattern;
    check(
      "空模式 + 状态码不匹配 → 不命中",
      emptyPattern?.matched === false,
      JSON.stringify(emptyPattern)
    );

    // ── 校验与保存一致 ──────────────────────────────────
    //
    // 试规则如果放宽了校验，就会出现「测试通过但保存不了」。
    let rejected = "";
    try {
      await test({ scope: "global", pattern: "" }, 429);
    } catch (error) {
      rejected = String(error);
    }
    out.rejected = rejected.slice(0, 120);
    check(
      "**缺匹配条件被拒，且与保存同一条消息**",
      rejected.includes("请至少配置错误文本或 HTTP 状态码"),
      rejected.slice(0, 120)
    );

    let customRejected = "";
    try {
      await test({ scope: "global", pattern: "x", customResponseStatus: 503 }, 429);
    } catch (error) {
      customRejected = String(error);
    }
    out.customRejected = customRejected.slice(0, 120);
    check(
      "**自定义返回只配一半也被拒**（同一套校验）",
      customRejected.includes("必须同时配置"),
      customRejected.slice(0, 120)
    );

    // ── 试规则**不落库** ────────────────────────────────
    const before = (await p.invoke("gateway:list-failover-rules", {}) ?? []).length;
    await test({ scope: "global", pattern: "should-not-persist", statusCodes: [418] }, 418);
    const after = (await p.invoke("gateway:list-failover-rules", {}) ?? []).length;
    out.ruleCounts = { before, after };
    check("试规则不写库", before === after, JSON.stringify(out.ruleCounts));

    // ── A1：自定义返回能存能读 ───────────────────────────
    const saved = await p.invoke("gateway:save-failover-rule", {
      scope: "global",
      pattern: "aivc-probe-custom",
      statusCodes: [599],
      customResponseStatus: 503,
      customResponseBody: '{"error":"busy"}',
      enabled: true
    });
    out.saved = saved;
    check(
      "**自定义返回按真实字段存住了**",
      saved?.customResponseStatus === 503 && saved?.customResponseBody === '{"error":"busy"}',
      JSON.stringify({
        status: saved?.customResponseStatus,
        body: saved?.customResponseBody
      })
    );
    const listed = (await p.invoke("gateway:list-failover-rules", {}) ?? []).find(
      (item) => item.id === saved?.id
    );
    check(
      "从列表读回来自定义返回还在（不是返回值拼出来的）",
      listed?.customResponseStatus === 503,
      JSON.stringify(listed?.customResponseStatus)
    );

    // 用**保存后的那条规则**再试一次：与草稿试的结果必须一致。
    const savedTest = await test(
      {
        scope: "global",
        pattern: listed.pattern,
        statusCodes: listed.statusCodes,
        customResponseStatus: listed.customResponseStatus,
        customResponseBody: listed.customResponseBody
      },
      599
    );
    out.savedTest = savedTest;
    check(
      "保存后的规则试出来同样命中（草稿与已存一致）",
      savedTest?.matched === true && savedTest?.matchedBy === "status",
      JSON.stringify(savedTest)
    );

    // 清理。
    await p.invoke("gateway:delete-failover-rule", { id: saved.id });
    const cleaned = (await p.invoke("gateway:list-failover-rules", {}) ?? []).some(
      (item) => item.id === saved?.id
    );
    check("探针造的规则已删掉（不污染真实数据）", !cleaned, String(cleaned));

    // ── 界面：表单里真的有这两个输入与「试一下」 ─────────
    const toolbox = [...document.querySelectorAll("button")].find(
      (item) => clean(item) === "工具箱"
    );
    if (toolbox) {
      toolbox.click();
      await p.sleep(500);
      const item = [...document.querySelectorAll("button")].find(
        (sub) => clean(sub) === "网关"
      );
      if (item) item.click();
      for (let index = 0; index < 10 && !document.querySelector(".overlay-body"); index += 1) {
        await p.sleep(400);
      }
    }
    await p.sleep(1500);
    const newRule = [...document.querySelectorAll(".overlay-body button")].find(
      (item) => clean(item) === "新增规则"
    );
    check("网关页里有「新增规则」", Boolean(newRule), String(Boolean(newRule)));
    if (newRule) {
      newRule.click();
      await p.sleep(700);
    }
    const overlay = document.querySelector(".overlay-body");
    const overlayText = overlay ? clean(overlay) : "";
    const placeholders = overlay
      ? [...overlay.querySelectorAll("input")].map((item) => item.placeholder)
      : [];
    out.ruleForm = { hasCustomStatus: placeholders.includes("例如 503"), placeholders };
    check(
      "**表单里有「自定义返回状态码」输入框**",
      placeholders.includes("例如 503"),
      JSON.stringify(placeholders)
    );
    check(
      "表单里有「自定义返回内容（JSON）」输入框",
      placeholders.some((value) => value.includes("上游不可用")),
      JSON.stringify(placeholders)
    );
    check(
      "表单里有「试一下」按钮（A2 的入口）",
      Boolean(
        [...(overlay?.querySelectorAll("button") ?? [])].find(
          (item) => clean(item) === "试一下"
        )
      ),
      overlayText.slice(0, 200)
    );

    // 点一次「试一下」：默认的试输入（429）与默认状态码应命中。
    const testButton = [...(overlay?.querySelectorAll("button") ?? [])].find(
      (item) => clean(item) === "试一下"
    );
    if (testButton) {
      // 先补一个能命中的模式。
      const patternInput = [...(overlay?.querySelectorAll("input") ?? [])].find((item) =>
        (item.placeholder ?? "").includes("insufficient_quota")
      );
      if (patternInput) {
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
        setter.call(patternInput, "insufficient_quota");
        patternInput.dispatchEvent(new Event("input", { bubbles: true }));
        await p.sleep(200);
      }
      const statusInput = [...(overlay?.querySelectorAll("input") ?? [])].find(
        (item) => (item.placeholder ?? "") === "429,500"
      );
      if (statusInput) {
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
        setter.call(statusInput, "429");
        statusInput.dispatchEvent(new Event("input", { bubbles: true }));
        await p.sleep(200);
      }
      testButton.click();
      let verdict = "";
      for (let index = 0; index < 16; index += 1) {
        await p.sleep(300);
        const text = clean(document.querySelector(".overlay-body") ?? document.body);
        if (text.includes("会命中") || text.includes("不会命中")) {
          verdict = text.includes("会命中：状态码匹配")
            ? "status"
            : text.includes("会命中：错误文本包含")
              ? "pattern"
              : "none";
          break;
        }
      }
      out.uiVerdict = verdict;
      check(
        "**点「试一下」后界面上给出了结论**（这条才证明 A2 真的能用）",
        verdict !== "",
        `verdict=${verdict}`
      );
      check("结论是被状态码命中（429 与配置一致）", verdict === "status", verdict);
    }
  } catch (error) {
    out.threw = String((error && error.stack) || error).slice(0, 700);
    check("探针自身没有抛异常", false, out.threw);
  }

  const failed = out.checks.filter((item) => !item.ok);
  out.verdict =
    failed.length === 0
      ? `PASS：${out.checks.length} 项第二批量验收全部通过`
      : `FAIL：${failed.length}/${out.checks.length} 未通过 —— ${failed.map((f) => f.name).join("、")}`;
  return out;
})();
