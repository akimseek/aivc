/**
 * 逐个验收这轮新增的**界面面**。
 *
 * # 为什么一个探针管这么多项
 *
 * 它们各自很浅（切个页签、点个按钮、看有没有渲染出来），但**每一项都要真的看一眼**——
 * 单元测试证明的是「给了这些 props 渲染出什么」，证明不了「它接在哪儿、能不能被点到」。
 * 上一轮 `queryGatewayLogs` 传错参数就是这么漏过去的：函数自己有测试、界面也调了它，
 * 唯独**参数名对不上**（后端静默忽略）。
 *
 * 每一项都有明确的判据，失败时给出**看到的是什么**而不是一句「失败」。
 */
(async () => {
  const p = window.__p;
  const out = { checks: [] };

  const safe = async (name, action) => {
    try {
      return await action();
    } catch (error) {
      out[`${name}Error`] = String((error && error.message) || error);
      return null;
    }
  };

  const check = (name, ok, detail) => {
    out.checks.push({ name, ok: Boolean(ok), detail });
  };

  const clickText = (root, text) => {
    const found = [...root.querySelectorAll("button")].find(
      (item) => (item.textContent || "").trim() === text
    );
    if (found) found.click();
    return Boolean(found);
  };

  // ── 前置：选中目标（让依赖目标的按钮可用） ──────────────
  const targets = await safe("targets", () => p.invoke("codex:list-targets", {}));
  const local = (targets || []).find(
    (item) => item.provider === "codex" && item.kind === "local" && item.available
  );
  if (!local) return { ...out, verdict: "SKIP：没有可用的本机 Codex 目标" };

  await safe("targetSelect", async () => {
    const select = await p.waitForTargets();
    const option = select && [...select.options].find((item) => item.value.endsWith(":local"));
    if (select && option) {
      p.selectValue(select, option.value);
      await p.sleep(2000);
    }
  });

  // ── 1. 侧栏一级导航里有「工作台」 ──────────────────────
  const sidebar = document.querySelector(".sidebar");
  const paneChips = [...document.querySelectorAll(".pane-nav .chip")].map((item) =>
    p.clean(item.textContent)
  );
  check("侧栏一级导航含工作台", paneChips.includes("工作台"), `导航=${JSON.stringify(paneChips)}`);

  // ── 2. 工作台能打开，并渲染出四块内容 ────────────────────
  await safe("workbench", async () => {
    const chip = [...document.querySelectorAll(".pane-nav .chip")].find(
      (item) => p.clean(item.textContent) === "工作台"
    );
    if (chip) chip.click();
    await p.sleep(2500);
  });
  const workbenchText = p.clean((document.querySelector(".rows") || {}).textContent);
  check(
    "工作台渲染运行上下文",
    workbenchText.includes("运行上下文"),
    workbenchText.slice(0, 200)
  );
  check("工作台渲染今日用量", workbenchText.includes("今日 Gateway 用量"), workbenchText.slice(0, 200));
  check("工作台渲染最近异常", workbenchText.includes("最近 Gateway 异常"), workbenchText.slice(0, 200));

  // 回到会话列表。
  await safe("back", async () => {
    const chip = [...document.querySelectorAll(".pane-nav .chip")].find(
      (item) => p.clean(item.textContent) === "会话"
    );
    if (chip) chip.click();
    await p.sleep(1500);
  });

  // ── 3. 侧栏折叠 / 展开 ─────────────────────────────────
  const beforeWidth = sidebar ? sidebar.getBoundingClientRect().width : 0;
  await safe("collapse", async () => {
    const button = document.querySelector('button[aria-label="折叠侧栏"]');
    if (button) button.click();
    await p.sleep(600);
  });
  const collapsedWidth = (document.querySelector(".sidebar") || { getBoundingClientRect: () => ({ width: 0 }) })
    .getBoundingClientRect().width;
  check(
    "侧栏折叠后变窄",
    collapsedWidth < beforeWidth && collapsedWidth > 0,
    `折叠前=${Math.round(beforeWidth)}px 折叠后=${Math.round(collapsedWidth)}px`
  );

  await safe("expand", async () => {
    const button = document.querySelector('button[aria-label="展开侧栏"]');
    if (button) button.click();
    await p.sleep(600);
  });
  const expandedWidth = (document.querySelector(".sidebar") || { getBoundingClientRect: () => ({ width: 0 }) })
    .getBoundingClientRect().width;
  check(
    "侧栏能再展开",
    expandedWidth > collapsedWidth,
    `展开后=${Math.round(expandedWidth)}px`
  );

  // ── 4. 会话详情弹层：能从会话头打开，且分页控件在 ────────
  //
  // 注意**必须切到会话标签**：点会话行会同时打开终端并让它到前台，而终端的
  // 正文区不渲染会话头（那是设计：终端在前台时不该被会话正文挤占）。
  // 上一版探针漏了这一步，于是「找不到详情按钮」——那是探针的问题，不是功能的问题。
  await safe("openDetail", async () => {
    const rows = [...document.querySelectorAll(".row")];
    if (rows[0]) rows[0].click();
    await p.sleep(3000);
  });

  const tabLabels = [...document.querySelectorAll(".tab")].map((item) => p.clean(item.textContent));
  out.tabs = tabLabels;
  await safe("switchToSessionTab", async () => {
    // 会话标签在前台之外的那一张：先看有没有已经带 `on` 的终端标签。
    const tabs = [...document.querySelectorAll(".tab")];
    const current = tabs.find((item) => item.className.includes("on"));
    // 点第一张（会话标签通常先建），再确认会话头出现了。
    if (tabs[0] && tabs[0] !== current) {
      tabs[0].click();
      await p.sleep(1500);
    }
  });
  out.sessionHeadPresent = Boolean(document.querySelector(".session-head"));

  await safe("detailButton", async () => {
    const head = document.querySelector(".session-head");
    if (head) clickText(head, "详情");
    await p.sleep(2000);
  });
  const detail = document.querySelector(".session-detail-dialog");
  check(
    "会话详情弹层能打开",
    Boolean(detail),
    detail
      ? "已打开"
      : `没有 .session-detail-dialog（会话头=${out.sessionHeadPresent}，标签=${JSON.stringify(out.tabs)}）`
  );
  if (detail) {
    const detailText = p.clean(detail.textContent);
    check("详情含分页控件（每页数量）", detailText.includes("每页数量"), detailText.slice(0, 160));
    check(
      "详情含四个翻页按钮",
      ["第一页", "上一页", "下一页", "最后一页"].every((label) =>
        [...detail.querySelectorAll("button")].some((item) => (item.textContent || "").trim() === label)
      ),
      detailText.slice(0, 160)
    );
  }
  await safe("closeDetail", async () => {
    if (detail) clickText(detail, "关闭");
    await p.sleep(800);
  });

  // ── 5. 配置覆盖层的新页签：用量 / 异常 / 日志 ───────────
  await safe("openConfig", async () => {
    // 走菜单：工具箱 → 网关。
    const toolbox = [...document.querySelectorAll("button")].find(
      (item) => (item.textContent || "").trim() === "工具箱"
    );
    if (toolbox) {
      toolbox.click();
      await p.sleep(500);
      const gateway = [...document.querySelectorAll("button")].find(
        (item) => (item.textContent || "").trim() === "网关"
      );
      if (gateway) gateway.click();
    }
    await p.sleep(2500);
  });
  const overlay = document.querySelector(".overlay-panel");
  check("配置覆盖层能打开", Boolean(overlay), overlay ? "已打开" : "没有 .overlay-panel");
  if (overlay) {
    const tabs = [...overlay.querySelectorAll(".overlay-tabs .chip")].map((item) =>
      p.clean(item.textContent)
    );
    check("配置页签含用量", tabs.includes("用量"), `页签=${JSON.stringify(tabs)}`);
    check("配置页签含异常", tabs.includes("异常"), `页签=${JSON.stringify(tabs)}`);
    check("配置页签含日志", tabs.includes("日志"), `页签=${JSON.stringify(tabs)}`);

    // 切到「用量」页，确认它真的渲染出报表（而不是空白）。
    const usageChip = [...overlay.querySelectorAll(".overlay-tabs .chip")].find(
      (item) => p.clean(item.textContent) === "用量"
    );
    if (usageChip) usageChip.click();
    await p.sleep(2500);
    const body = p.clean((document.querySelector(".overlay-body") || {}).textContent);
    check(
      "用量页渲染报表（按供应商/按模型）",
      body.includes("按供应商") && body.includes("按模型"),
      body.slice(0, 200)
    );

    // 切到「日志」，确认有查询范围与结果类型两个筛选。
    const logChip = [...overlay.querySelectorAll(".overlay-tabs .chip")].find(
      (item) => p.clean(item.textContent) === "日志"
    );
    if (logChip) logChip.click();
    await p.sleep(2500);
    const logBody = p.clean((document.querySelector(".overlay-body") || {}).textContent);
    check(
      // 标签从「查询范围」改成「时间范围」：三个观测面板现在**共用同一个**
      // `GatewayRangePicker`，那个组件的字段名就是「时间范围」。
      // 统一是刻意的——旧平台是三处各写一遍，提示语与默认值都略有不同。
      "日志页有时间范围与结果类型",
      logBody.includes("时间范围") && logBody.includes("结果类型"),
      logBody.slice(0, 200)
    );
    check("日志页提示只搜回滚之外还有全选/删除", logBody.includes("全选当前页") || logBody.includes("删除选中"), logBody.slice(0, 200));
  }

  // 关掉配置。
  await safe("closeConfig", async () => {
    if (overlay) clickText(overlay, "关闭");
    await p.sleep(800);
  });

  const failed = out.checks.filter((item) => !item.ok);
  out.verdict =
    failed.length === 0
      ? `PASS：${out.checks.length} 项界面验收全部通过`
      : `FAIL：${failed.length}/${out.checks.length} 项未通过 —— ${failed
          .map((item) => item.name)
          .join("、")}`;
  return out;
})();
