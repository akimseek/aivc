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

  /**
   * 标签栏动作区的「消息与分支」按钮（`features/workspace/tab-bar.tsx`）。
   *
   * **不能按整页文本精确匹配**：这颗按钮里除了 `<span>消息与分支</span>` 还挂着一颗
   * 条数徽章，选中会话之后 `textContent` 变成「消息与分支0」这种带数字的形式，
   * 精确相等永远匹配不到。按类名 + 前缀定位，且把范围收在标签栏里
   * （会话卡片上另有一颗同名的动作按钮，整页找会点错）。
   */
  const messagesAction = () =>
    [...document.querySelectorAll(".tab-bar-action")].find((item) =>
      (item.textContent || "").trim().startsWith("消息与分支")
    );

  // ── 前置：选中目标（让依赖目标的按钮可用） ──────────────
  const targets = await safe("targets", () => p.invoke("codex:list-targets", {}));
  const local = (targets || []).find(
    (item) => item.provider === "codex" && item.kind === "local" && item.available
  );
  if (!local) return { ...out, verdict: "SKIP：没有可用的本机 Codex 目标" };

  await safe("targetSelect", async () => {
    /*
      **先自己选平台，再选目标。**
      
      本应用现在**启动后什么都不选**（刻意如此：此前三处「还原不上就挑第一个」，
      界面会替你选好）。那时目标下拉是空的，会话行也不存在，于是下面
      「打开详情」那一段找不到任何可点的行——看起来像功能坏了。
      
      这里补上用户真实的两步。用**显式设值 + change 事件**，与另外几个探针一致。
    */
    const setSelect = async (element, value) => {
      Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value").set.call(element, value);
      element.dispatchEvent(new Event("change", { bubbles: true }));
      await p.sleep(1400);
    };
    const providerSelect = document.querySelector(".sidebar .pickers select");
    const providerOption = [...(providerSelect?.options ?? [])].find(
      (item) => item.value === "codex"
    );
    if (providerSelect && providerOption) await setSelect(providerSelect, "codex");

    const selects = [...document.querySelectorAll(".sidebar .pickers select")];
    const targetSelect = selects[1] ?? (await p.waitForTargets());
    // **用「第一个非空选项」**，不要 `endsWith(":local")`：本机目标的编号形如
    // `codex:local:C:\Users\akim\.codex`，它不以 `:local` 结尾——原来的过滤器
    // 一个也匹配不到，于是目标从未被选中，会话行始终为 0，
    // 表现成「详情功能坏了」。
    const option =
      targetSelect && [...targetSelect.options].find((item) => item.value !== "");
    if (targetSelect && option) await setSelect(targetSelect, option.value);

    // 等会话行真的出现：不等的话下一步点的是空列表，表现成「详情功能坏了」。
    //
    // 锚点是 `.session-card` 而不是 `.row`：主列表默认渲染的是会话卡片
    // （`features/sessions/session-card.tsx`），`.row` 只有收藏夹与全局搜索里才有，
    // 照旧口径等它**永远是 0**——白等 12 秒，还让下面「打开详情」点不到东西。
    // 用有界等待（命中即返回），不再写死 24×500ms 的空转。
    await p.waitFor(".sidebar .rows .session-card");
    out.rowCount = document.querySelectorAll(".sidebar .rows .session-card").length;
  });

  // ── 1. 活动栏里有「工作台」 ────────────────────────────
  //
  // 批次一之后一级去处从侧栏 chips 搬到了活动栏（`.activity-item`，
  // `shell/activity-bar.tsx`），旧探针找的 `.pane-nav .chip` 不存在了。
  const activityLabels = [...document.querySelectorAll(".activity-top .activity-item")].map(
    (item) => item.getAttribute("aria-label") || ""
  );
  check(
    "活动栏一级去处含工作台",
    activityLabels.includes("工作台与运行概览"),
    `去处=${JSON.stringify(activityLabels)}`
  );

  // ── 2. 工作台能打开，并渲染出四块内容 ────────────────────
  await safe("workbench", async () => {
    const item = [...document.querySelectorAll(".activity-top .activity-item")].find(
      (x) => (x.getAttribute("aria-label") || "") === "工作台与运行概览"
    );
    if (item) item.click();
    await p.sleep(2500);
  });
  // 工作台侧栏的类是 `.workbench-body`（`features/workbench/workbench-sidebar.tsx`），
  // 旧的 `.rows` 是会话列表容器，工作台里根本没有。
  const workbenchText = p.clean((document.querySelector(".workbench-body") || {}).textContent);
  check(
    "工作台渲染运行上下文",
    workbenchText.includes("运行上下文"),
    workbenchText.slice(0, 200)
  );
  check("工作台渲染今日用量", workbenchText.includes("今日 Gateway 用量"), workbenchText.slice(0, 200));
  check("工作台渲染最近异常", workbenchText.includes("最近 Gateway 异常"), workbenchText.slice(0, 200));

  // 回到会话列表。
  await safe("back", async () => {
    const item = [...document.querySelectorAll(".activity-top .activity-item")].find(
      (x) => (x.getAttribute("aria-label") || "") === "AI 会话管理"
    );
    if (item) item.click();
    await p.sleep(1500);
  });

  // ── 3. 侧栏折叠 / 展开 ─────────────────────────────────
  //
  // 宽度必须**现查现量**：侧栏会换元素——切到工作台是整块替换，切回来是重新挂载，
  // 在切页之前抓的引用到这里已经**脱离文档**，`getBoundingClientRect()` 一律返回 0。
  // 旧写法抓了一次引用量「折叠前」，得到的 0px 让这一条变成假 FAIL（实测打到过）。
  const sidebarWidth = () =>
    (document.querySelector(".sidebar") || { getBoundingClientRect: () => ({ width: 0 }) })
      .getBoundingClientRect().width;
  const beforeWidth = sidebarWidth();
  await safe("collapse", async () => {
    const button = document.querySelector('button[aria-label="折叠侧栏"]');
    if (button) button.click();
    await p.sleep(600);
  });
  const collapsedWidth = sidebarWidth();
  // 折叠的设计是 `.sidebar.hidden`：**宽度归零**、节点留在流内占住那一列
  // （`app.css:307`，理由在 `app.tsx` 的 `sidebar=` 分支——`display:none` 会让主区
  // 掉进第 2 列）。因此旧口径里的「宽度 > 0」在这个设计下**永远假 FAIL**；
  // 改成「折叠态确实挂上了 + 宽度严格变窄」，两个信号都来自当前 DOM。
  const collapsedMarked = Boolean(document.querySelector(".sidebar.hidden"));
  check(
    "侧栏折叠后变窄",
    collapsedMarked && collapsedWidth < beforeWidth,
    `折叠前=${Math.round(beforeWidth)}px 折叠后=${Math.round(collapsedWidth)}px 折叠态=${collapsedMarked}`
  );

  await safe("expand", async () => {
    // 展开入口**不是**一个「展开侧栏」按钮（按 aria-label 找它永远是 null）：
    // 折叠后点活动栏里的「会话」或「工作台」把侧栏叫回来
    // （`features/sessions/sessions-sidebar.tsx` 文件头与 `app.tsx` 的 onActivity）。
    const item = [...document.querySelectorAll(".activity-top .activity-item")].find(
      (x) => (x.getAttribute("aria-label") || "") === "AI 会话管理"
    );
    if (item) item.click();
    await p.sleep(800);
  });
  const expandedWidth = sidebarWidth();
  check(
    "侧栏能再展开",
    expandedWidth > collapsedWidth,
    `展开后=${Math.round(expandedWidth)}px`
  );

  // ── 4. 会话详情弹层：能从标签栏打开，且分页控件在 ────────
  //
  // 批次三：主列表的行是 `.session-card`（`features/sessions/session-card.tsx`），
  // 单击是「打开历史正文」；详情入口在**标签栏**的动作区（「消息与分支」，
  // `features/workspace/tab-bar.tsx`）——不是在正文头部：进了终端之后正文不再渲染，
  // 放在那里等于「进了终端就再也看不到详情」。
  await safe("openDetail", async () => {
    const card = document.querySelector(".sidebar .rows .session-card");
    if (card) card.click();
    for (let index = 0; index < 12 && !document.querySelector(".history-head"); index += 1) {
      await p.sleep(500);
    }
    await p.sleep(1500);
  });

  const tabLabels = [...document.querySelectorAll(".tab-pill")].map((item) => p.clean(item.textContent));
  out.tabs = tabLabels;
  out.historyHeadPresent = Boolean(document.querySelector(".history-head"));

  await safe("detailButton", async () => {
    // 还没打开时才点：重复点会先关再开，白挨一次闪烁。
    const action = messagesAction();
    if (!document.querySelector(".messages-panel") && action) {
      action.click();
    }
    await p.sleep(2000);
  });
  // 会话详情弹层的类是 `.messages-panel`（`features/workspace/messages-modal.tsx`）；
  // 旧的 `.session-detail-dialog` / `.session-detail-modal` 都不存在了。
  const detail = document.querySelector(".messages-panel");
  check(
    "会话详情弹层能打开",
    Boolean(detail),
    detail
      ? "已打开"
      : `没有 .messages-panel（会话头=${out.historyHeadPresent}，标签=${JSON.stringify(out.tabs)}）`
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
    check(
      "详情标题写的是消息与分支（不是空壳）",
      detailText.includes("会话消息历史与分支演进"),
      detailText.slice(0, 160)
    );
  }
  await safe("closeDetail", async () => {
    // 关闭是**图标按钮**（没有文字）：按无障碍名找。
    const close = [...document.querySelectorAll(".messages-head button")].find(
      (item) => (item.getAttribute("aria-label") || "").trim() === "关闭"
    );
    if (close) close.click();
    await p.sleep(800);
  });

  // ── 5. 设置中心的新导航项：用量报表 / 异常诊断 / 请求日志 ───────────
  await safe("openConfig", async () => {
    // 入口是活动栏底部的设置按钮——批次二移除了菜单栏，旧的「工具箱 → 网关」不存在了。
    const settings = document.querySelector(".activity-bottom button");
    if (settings) settings.click();
    await p.sleep(2500);
  });
  const overlay = document.querySelector(".settings-panel");
  check("设置中心能打开", Boolean(overlay), overlay ? "已打开" : "没有 .settings-panel");
  if (overlay) {
    const tabs = [...overlay.querySelectorAll(".settings-nav-item")].map((item) =>
      p.clean(item.textContent)
    );
    check("设置导航含用量报表", tabs.includes("用量报表"), `导航=${JSON.stringify(tabs)}`);
    check("设置导航含异常诊断", tabs.includes("异常诊断"), `导航=${JSON.stringify(tabs)}`);
    check("设置导航含请求日志", tabs.includes("请求日志"), `导航=${JSON.stringify(tabs)}`);

    // 切到「用量报表」页，确认它真的渲染出报表（而不是空白）。
    const usageChip = [...overlay.querySelectorAll(".settings-nav-item")].find(
      (item) => p.clean(item.textContent) === "用量报表"
    );
    if (usageChip) usageChip.click();
    await p.sleep(2500);
    const body = p.clean((document.querySelector(".settings-content") || {}).textContent);
    check(
      "用量页渲染报表（按供应商/按模型）",
      body.includes("按供应商") && body.includes("按模型"),
      body.slice(0, 200)
    );

    // 切到「请求日志」，确认有查询范围与结果类型两个筛选。
    const logChip = [...overlay.querySelectorAll(".settings-nav-item")].find(
      (item) => p.clean(item.textContent) === "请求日志"
    );
    if (logChip) logChip.click();
    await p.sleep(2500);
    const logBody = p.clean((document.querySelector(".settings-content") || {}).textContent);
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
    // 关闭是**图标按钮**（没有文字），按无障碍名找。
    const close = [...document.querySelectorAll(".settings-head button")].find(
      (item) => (item.getAttribute("aria-label") || "").trim() === "关闭"
    );
    if (close) close.click();
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
