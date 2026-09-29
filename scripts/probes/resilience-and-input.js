/**
 * 第五批量验收：A6 三个韧性阈值 / A11 终端输入模式切换。
 *
 * # 为什么必须实测
 *
 * A6 的要害是「**四个函数真的通到命令面了**」。实现一直都在（含边界校验），
 * 只是从没登记进 `COMMANDS`——单测可以证明函数本身对，证明不了命令名字对得上：
 * 命令表里的字符串与路由里的字符串是两处手写的真值，写错一处就是「界面点了报未知命令」。
 *
 * A11 的要害是「**按钮点了真的换了模式**」，而且换完焦点跟着走。
 *
 * # 自证
 *
 * - A6：先读一次拿到当前值，改掉，**再读一次确认落库**（不是只看返回值）；
 *   再用越界值确认后端拒绝。
 * - A11：先确认按钮上的字是「终端输入」，点一下，再确认它变成「自管输入」——
 *   两次读数不同才算切换发生。
 */
(async () => {
  const p = window.__p;
  const out = { checks: [] };
  const check = (name, ok, detail) => out.checks.push({ name, ok: Boolean(ok), detail: detail ?? "" });
  const clean = (element) => p.clean(element.textContent);

  try {
    await p.waitForTargets();

    // ── A6：三个阈值 ─────────────────────────────────────
    //
    // 三条命令都在命令表里，因此这里直接打后端——它们此前**根本不存在**，
    // 未知命令会直接抛出来。
    const before = await p.invoke("gateway:get-port", {});
    out.before = before;
    check(
      "`gateway:get-port` 现在**带着三个阈值**（此前这个形状里没有它们）",
      typeof before?.failureThreshold === "number" &&
        typeof before?.circuitFailureThreshold === "number" &&
        typeof before?.circuitDurationSeconds === "number",
      JSON.stringify({
        failure: before?.failureThreshold,
        circuit: before?.circuitFailureThreshold,
        duration: before?.circuitDurationSeconds
      })
    );

    // 改异常切换阈值 → 再读一次确认落库。
    const wanted = before.failureThreshold === 7 ? 4 : 7;
    const afterSet = await p.invoke("gateway:set-failure-threshold", { value: wanted });
    out.afterSet = afterSet;
    check(
      "`gateway:set-failure-threshold` 改完回的是**完整状态**（不是只回一个数）",
      afterSet?.failureThreshold === wanted &&
        typeof afterSet?.baseUrl === "string" &&
        typeof afterSet?.configuredPort === "number",
      JSON.stringify(afterSet)
    );
    const reread = await p.invoke("gateway:get-port", {});
    check(
      "**重新读一次确认真的落库了**（不是返回值拼出来的）",
      reread?.failureThreshold === wanted,
      String(reread?.failureThreshold)
    );

    // 熔断次数与持续时间各改一次。
    const circuit = await p.invoke("gateway:set-circuit-failure-threshold", { value: 9 });
    check("`gateway:set-circuit-failure-threshold` 生效", circuit?.circuitFailureThreshold === 9, String(circuit?.circuitFailureThreshold));
    const duration = await p.invoke("gateway:set-circuit-duration-seconds", { value: 900 });
    check("`gateway:set-circuit-duration-seconds` 生效", duration?.circuitDurationSeconds === 900, String(duration?.circuitDurationSeconds));

    // 边界：三条各有各的范围，越界必须被拒（错误文案与旧平台一致）。
    const rejects = [
      ["gateway:set-failure-threshold", 11, "异常切换阈值必须是 1 到 10 之间的整数。"],
      ["gateway:set-failure-threshold", 0, "异常切换阈值必须是 1 到 10 之间的整数。"],
      ["gateway:set-circuit-failure-threshold", 21, "熔断次数必须是 1 到 20 之间的整数。"],
      ["gateway:set-circuit-duration-seconds", 9, "熔断持续时间必须是 10 到 86400 秒之间的整数。"]
    ];
    for (const [command, value, message] of rejects) {
      let got = "";
      try {
        await p.invoke(command, { value });
      } catch (error) {
        got = String(error);
      }
      check(`越界被拒：${command} value=${value}`, got.includes(message), got.slice(0, 90));
    }

    // 恢复成读到的原值——探针不该改用户的设置。
    await p.invoke("gateway:set-failure-threshold", { value: before.failureThreshold });
    await p.invoke("gateway:set-circuit-failure-threshold", {
      value: before.circuitFailureThreshold
    });
    await p.invoke("gateway:set-circuit-duration-seconds", {
      value: before.circuitDurationSeconds
    });
    const restored = await p.invoke("gateway:get-port", {});
    check(
      "探针把三项改回了原值（不污染用户设置）",
      restored?.failureThreshold === before.failureThreshold &&
        restored?.circuitFailureThreshold === before.circuitFailureThreshold &&
        restored?.circuitDurationSeconds === before.circuitDurationSeconds,
      JSON.stringify(restored)
    );

    // ── 界面：三个输入框真的在网关页上 ───────────────────
    const toolbox = [...document.querySelectorAll("button")].find(
      (item) => clean(item) === "工具箱"
    );
    if (toolbox) {
      toolbox.click();
      await p.sleep(500);
      const item = [...document.querySelectorAll("button")].find((sub) => clean(sub) === "网关");
      if (item) item.click();
      for (let index = 0; index < 12 && !document.querySelector(".overlay-body"); index += 1) {
        await p.sleep(400);
      }
    }
    await p.sleep(1500);
    const overlay = document.querySelector(".overlay-body");
    const labels = [
      "异常切换阈值",
      "熔断次数",
      "熔断持续时间"
    ];
    out.thresholdInputs = labels.map((label) =>
      Boolean(overlay?.querySelector(`input[aria-label="${label}"]`))
    );
    check(
      "**界面上有三个阈值的输入框**（此前它们完全不存在）",
      out.thresholdInputs.every(Boolean),
      JSON.stringify(out.thresholdInputs)
    );
    const overlayText = overlay ? clean(overlay) : "";
    check(
      "三条说明文案都在（这三个数字不直观）",
      overlayText.includes("连续请求失败达到该次数后") &&
        overlayText.includes("累计失败达到次数后进入熔断状态") &&
        overlayText.includes("到期后进入半开状态"),
      overlayText.slice(0, 200)
    );

    // 关掉覆盖层，下面要用终端。
    const close = [...document.querySelectorAll(".overlay-head button")].find(
      (item) => clean(item) === "关闭"
    );
    if (close) {
      close.click();
      await p.sleep(600);
    }

    // ── A11：终端输入模式切换 ───────────────────────────
    //
    // 先开一路系统终端（它会渲染输入框）。
    const shell = [...document.querySelectorAll("button")].find(
      (item) => clean(item) === "+ Shell"
    );
    if (shell) {
      shell.click();
      for (let index = 0; index < 40 && !document.querySelector(".composer-actions"); index += 1) {
        await p.sleep(500);
      }
    }
    await p.sleep(1500);

    const composerActions = document.querySelector(".composer-actions");
    out.composerPresent = Boolean(composerActions);
    check("系统终端下出现了自管输入框", Boolean(composerActions), String(Boolean(composerActions)));

    /** 工具条里那个切换按钮：字只可能是这两个之一。 */
    const toggleButton = () => {
      const buttons = [...(document.querySelector(".composer-actions")?.querySelectorAll("button") ?? [])];
      return buttons.find(
        (item) => clean(item) === "终端输入" || clean(item) === "自管输入"
      );
    };

    const first = toggleButton();
    out.firstLabel = first ? clean(first) : "";
    check(
      "**输入框工具条里有切换按钮**（旧平台这个按钮在状态栏上）",
      Boolean(first),
      out.firstLabel
    );
    check(
      "初始是「自管输入框」模式，因此按钮写着「终端输入」",
      out.firstLabel === "终端输入",
      out.firstLabel
    );
    check(
      "按钮的悬停说明是旧平台的原话",
      (first?.getAttribute("title") ?? "").includes("切换到终端原生输入"),
      first?.getAttribute("title") ?? ""
    );

    if (first) {
      first.click();
      await p.sleep(500);
      const second = toggleButton();
      out.secondLabel = second ? clean(second) : "";
      check(
        "**点一下之后字变成「自管输入」**（说明模式真的切了，不是个静态按钮）",
        out.secondLabel === "自管输入",
        out.secondLabel
      );
      check(
        "悬停说明也跟着变",
        (second?.getAttribute("title") ?? "").includes("切换到自管输入框"),
        second?.getAttribute("title") ?? ""
      );

      // 再点一次应当切回来——两个方向都要能走。
      second?.click();
      await p.sleep(500);
      const third = toggleButton();
      out.thirdLabel = third ? clean(third) : "";
      check(
        "再点一次切回「终端输入」（两个方向都能走）",
        out.thirdLabel === "终端输入",
        out.thirdLabel
      );
    }
  } catch (error) {
    out.threw = String((error && error.stack) || error).slice(0, 700);
    check("探针自身没有抛异常", false, out.threw);
  }

  const failed = out.checks.filter((item) => !item.ok);
  out.verdict =
    failed.length === 0
      ? `PASS：${out.checks.length} 项第五批量验收全部通过`
      : `FAIL：${failed.length}/${out.checks.length} 未通过 —— ${failed.map((f) => f.name).join("、")}`;
  return out;
})();
