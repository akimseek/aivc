/**
 * 复核用量 / 异常 / 日志三个面板的**「有数据」渲染路径**。
 *
 * # 为什么非要专门跑这一次
 *
 * 上一轮验收时运行根里一条网关记录都没有，三张表走的**全是空状态**。
 * 空状态与有数据是两条代码路径：表头、分组、金额与耗时的格式化、
 * 错误代码列——这些在有数据之前一行都没被执行过。
 *
 * # 判据是**具体的数字与文字**，不是「表格存在」
 *
 * 造进去的数据是已知的（见 `scripts/seed-gateway-logs.cjs`）：
 *
 * | 断言 | 它证明什么 |
 * | --- | --- |
 * | 请求数 = 4 | 计数用的是这个区间内的全部记录 |
 * | 成功 2 / 失败 2 | `outcome` 分类正确，而不是全算成功 |
 * | 平均耗时 8041 ms | 求平均而不是求和（1840+320+30000+3 除以 4） |
 * | 总 token 1.7k | **`usage_json` 真的被解析出来了**——字段名写错时这里会是「—」 |
 * | 费用 $0.0123 | 同上，且是 4 位小数 |
 * | 按供应商表里有 seed-vendor-a | 分组维度真的分开了 |
 * | 异常里有 rate_limited / HTTP 429 | 错误代码与上游状态都渲染出来 |
 * | **日志里有 `route-missing`** | 最要紧的一条：`outcome=ok` 但带错误代码的记录也显示得出来 |
 */
(async () => {
  const p = window.__p;
  const out = { checks: [] };

  const check = (name, ok, detail) => {
    out.checks.push({ name, ok: Boolean(ok), detail: detail ?? "" });
  };

  const safe = async (name, action) => {
    try {
      return await action();
    } catch (error) {
      out[`${name}Error`] = String((error && error.message) || error);
      return null;
    }
  };

  /** 打开配置覆盖层并切到某一页。 */
  const openTab = async (label) => {
    if (!document.querySelector(".overlay-panel")) {
      const toolbox = [...document.querySelectorAll("button")].find(
        (item) => (item.textContent || "").trim() === "工具箱"
      );
      if (toolbox) {
        toolbox.click();
        await p.sleep(400);
        const gateway = [...document.querySelectorAll("button")].find(
          (item) => (item.textContent || "").trim() === "网关"
        );
        if (gateway) gateway.click();
      }
      await p.sleep(2500);
    }
    const chip = [...document.querySelectorAll(".overlay-tabs .chip")].find(
      (item) => p.clean(item.textContent) === label
    );
    if (!chip) return false;
    chip.click();
    await p.sleep(2500);
    return true;
  };

  const bodyText = () => p.clean((document.querySelector(".overlay-body") || {}).textContent);

  // ── 用量 ───────────────────────────────────────────────
  out.openedUsage = await openTab("用量");
  const usage = bodyText();
  out.usageText = usage.slice(0, 400);

  check("用量：请求数 4", /请求数\s*4/.test(usage), usage.slice(0, 120));
  check("用量：成功 2", /成功\s*2/.test(usage), usage.slice(0, 120));
  check("用量：失败 2", /失败\s*2/.test(usage), usage.slice(0, 120));
  // 平均 != 求和：求和是 32163，平均是 8041。这一条能区分两种实现。
  check(
    "用量：平均耗时 8041 ms（是平均不是求和）",
    usage.includes("8041 ms") && !usage.includes("32163"),
    usage.slice(0, 160)
  );
  check(
    "用量：总 token 1.7k（usage_json 真的被解析了）",
    usage.includes("1.7k"),
    usage.slice(0, 200)
  );
  check("用量：费用 4 位小数", usage.includes("$0.0123"), usage.slice(0, 200));
  check(
    "用量：按供应商分组显示真实的供应商名",
    usage.includes("供应商甲") && usage.includes("供应商乙"),
    usage.slice(0, 400)
  );
  check(
    // 这两条配对：**存在的**供应商显示真名，**不存在的**才走占位文案。
    //
    // 第三行的 `vendor_id` 是 `(none)`（那条「没有可用供应商」的记录），
    // 它确实不在供应商表里，因此占位文案出现**一次**是对的。
    // 第一版探针断言「一次都不该出现」，那是把两种处境混成了一种。
    "用量：占位文案只出现一次（仅那条没有供应商的记录）",
    (usage.match(/已删除供应商/g) ?? []).length === 1,
    `出现 ${(usage.match(/已删除供应商/g) ?? []).length} 次：${usage.slice(0, 400)}`
  );
  check(
    "用量：按模型分组有 gpt-5-codex",
    usage.includes("gpt-5-codex"),
    usage.slice(0, 400)
  );
  check(
    "用量：没有模型名的那条归到「未标注模型」",
    usage.includes("未标注模型"),
    usage.slice(0, 400)
  );

  // ── 异常 ───────────────────────────────────────────────
  out.openedFailures = await openTab("异常");
  const failures = bodyText();
  out.failureText = failures.slice(0, 400);

  check("异常：有错误代码 rate_limited", failures.includes("rate_limited"), failures.slice(0, 240));
  check(
    "异常：有错误代码 upstream_timeout",
    failures.includes("upstream_timeout"),
    failures.slice(0, 240)
  );
  check("异常：上游状态 429 渲染出来", failures.includes("429"), failures.slice(0, 240));
  check("异常：耗时 30 秒的那条没被截断", failures.includes("30000 ms"), failures.slice(0, 240));
  check(
    "异常：显示总数而不是只列几行",
    /共\s*\d+\s*条/.test(failures),
    failures.slice(0, 160)
  );

  // ── 日志 ───────────────────────────────────────────────
  out.openedLogs = await openTab("日志");
  const logs = bodyText();
  out.logText = logs.slice(0, 400);

  // **这一条最要紧**：`outcome=ok` 但带错误代码的记录。
  // 面板不显示错误代码列的话，「路由不存在 / 鉴权失败」在日志里看起来就是一次成功。
  check(
    "日志：`route-missing`（outcome=ok 但带错误代码）显示出来了",
    logs.includes("route-missing"),
    logs.slice(0, 300)
  );
  check("日志：列出了 claude 平台那条", logs.includes("claude"), logs.slice(0, 300));
  check("日志：结果分类译成中文（成功/失败/超时）", /成功|失败|超时/.test(logs), logs.slice(0, 200));
  check(
    "日志：结果条数与库内一致（4 条）",
    logs.includes("4 条") || logs.includes("（4"),
    logs.slice(0, 200)
  );

  const failed = out.checks.filter((item) => !item.ok);
  out.verdict =
    failed.length === 0
      ? `PASS：${out.checks.length} 项有数据渲染全部通过`
      : `FAIL：${failed.length}/${out.checks.length} 项未通过 —— ${failed
          .map((item) => item.name)
          .join("、")}`;
  return out;
})();
