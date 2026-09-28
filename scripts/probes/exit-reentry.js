/**
 * 复现并验收「`/exit` 退出会话后，再次进入被锁住」。
 *
 * # 上一版为什么得出的是**假结论**
 *
 * 它把 `/exit\r` 放在**一个数据块**里发出去。TUI 把带换行的整块当成「粘贴」，
 * 那个 `\r` 成了输入框里的换行符而**不是提交**——`/exit` 从来没执行过。
 * 于是探针看到的「退出事件」「标签收掉了」全部来自**用户手动退出**，
 * 而它却把这个当成了「我修的东西生效了」。
 *
 * 两条规则由此固定下来：
 *
 * 1. **打字与回车必须分两次写**，中间留时间让输入进到输入框；
 * 2. **探针必须自证这个退出是它造成的**——按下回车之前先读屏幕确认 `/exit`
 *    确实出现在输入框里，并且确认**此前没有**这一路的退出事件。
 *    任何一条不成立就判 INVALID，而不是继续往下走。
 *
 * # 判据
 *
 * 干净退出（退出码 0）之后：标签被收掉、那时已无终端；再次进入同一条会话时
 * **起了一路新的终端**（新的 `terminalId`）。这就是「没被锁住」。
 */
(async () => {
  const p = window.__p;
  const out = { steps: [] };

  const safe = async (name, action) => {
    try {
      return await action();
    } catch (error) {
      out[`${name}Error`] = String((error && error.message) || error);
      return null;
    }
  };

  const snapshot = (label) => {
    const tabs = [...document.querySelectorAll(".tab")];
    const active = tabs.find((item) => item.className.includes("on"));
    return {
      label,
      tabs: tabs.length,
      tabLabels: tabs.map((item) => p.clean(item.textContent)),
      activeTab: active ? p.clean(active.textContent) : null,
      xterms: document.querySelectorAll(".terminal-stack .xterm").length,
      rows: document.querySelectorAll(".row").length,
      screen: p.rows().slice(0, 70)
    };
  };

  /** 全部退出事件，带到达时间——用来判断「这个退出是谁造成的」。 */
  const exits = [];
  await window.__TAURI__.event.listen("terminal:exit", (event) => {
    exits.push({
      terminalId: event.payload && event.payload.terminalId,
      exitCode: event.payload && event.payload.exitCode,
      at: Date.now()
    });
  });

  // ── 前置：拿目标与会话 ─────────────────────────────────────
  const targets = await safe("targets", () => p.invoke("codex:list-targets", {}));
  const local = (targets || []).find(
    (item) => item.provider === "codex" && item.kind === "local" && item.available
  );
  if (!local) return { ...out, verdict: "SKIP：没有可用的本机 Codex 目标" };

  const sessions = await safe("sessions", () =>
    p.invoke("codex:list-sessions", { targetId: local.id })
  );
  const pick = (sessions || []).find((item) => item.cwd && item.messageCount >= 2);
  if (!pick) return { ...out, verdict: "SKIP：没有可恢复的历史会话" };
  out.sessionTitle = pick.title;

  /**
   * 把左侧摆回有行可点的状态，并**记下它为什么没有行**。
   *
   * 这一版比上一版多做两件事，因为上一次它只是反复失败、说不出原因：
   *
   * 1. 逐个点 `.pane-nav` 里的 chip（不再靠文本精确匹配那一个）；
   * 2. 每轮都把左侧的真实状态记下来（有哪些 chip、侧栏文本、行数），
   *    这样失败时看到的是**证据**而不是一句「找不到行」。
   */
  const sidebarState = () => ({
    chips: [...document.querySelectorAll(".pane-nav .chip")].map((item) =>
      p.clean(item.textContent)
    ),
    activeChip: p.clean(
      (document.querySelector(".pane-nav .chip.on") || {}).textContent
    ),
    rows: document.querySelectorAll(".row").length,
    sidebarText: p.clean((document.querySelector(".sidebar") || {}).textContent).slice(0, 80)
  });

  const ensureRows = async (attempts = 24) => {
    for (let round = 0; round < attempts; round += 1) {
      if (document.querySelectorAll(".row").length > 0) return true;
      const chips = [...document.querySelectorAll(".pane-nav .chip")];
      // 轮流点：哪一个是「会话」由界面决定，探针不猜文本。
      const chip = chips[round % Math.max(chips.length, 1)];
      if (chip) chip.click();
      await p.sleep(700);
    }
    return document.querySelectorAll(".row").length > 0;
  };

  /** 按标题找会话行；找不到返回 null——**不拿 rows[0] 顶上**，那会点错对象。 */
  const findRow = (title) =>
    [...document.querySelectorAll(".row")].find(
      (item) => (item.getAttribute("title") || "").trim() === (title || "").trim()
    ) || null;

  // ── 前置：**先在界面上选中目标**，让「会话」列表真正有行 ────
  //
  // 这一步是上一版漏掉的，也是它必须补上的原因：探针此前只用 IPC 列了目标和会话，
  // **没在界面上选过目标**，于是左侧「会话」列表是空的（`rows: 0`），
  // 它只好退到「快捷」里去找行——而快捷里的行是它自己 `openAt` 留下的足迹。
  //
  // 那条路径与用户实际点的「会话列表」调的是同一个函数，所以旧结果对代码有效；
  // 但**它验证的不是用户走的那条入口**。补上这一步之后，证据才落在真实入口上。
  out.targetSelect = await safe("targetSelect", async () => {
    const select = await p.waitForTargets();
    if (!select) return { ok: false, reason: "等不到目标下拉框" };
    const local = [...select.options].find((option) => option.value.endsWith(":local"));
    if (!local) return { ok: false, reason: "下拉框里没有本机目标" };
    p.selectValue(select, local.value);
    await p.sleep(2500);
    return { ok: true, picked: local.value };
  });
  out.steps.push(`选中目标：${JSON.stringify(out.targetSelect)}`);

  // 会话页本身也要真的被选中（默认可能是别的页）。
  const chipSession = [...document.querySelectorAll(".pane-nav .chip")].find(
    (item) => p.clean(item.textContent) === "会话"
  );
  if (chipSession) chipSession.click();
  await p.sleep(2500);

  out.rowsReady = await ensureRows();
  out.sidebarBeforeFirstEntry = sidebarState();
  if (!out.rowsReady) {
    out.verdict = `FAIL：选中目标后「会话」列表仍然没有行。左侧状态：${JSON.stringify(out.sidebarBeforeFirstEntry)}`;
    return out;
  }

  const row = findRow(pick.title);
  if (!row) {
    out.verdict = `FAIL：列表里找不到标题为「${pick.title}」的会话行（行数 ${document.querySelectorAll(".row").length}）`;
    return out;
  }

  let terminalId = null;
  await window.__TAURI__.event.listen("terminal:data", (event) => {
    if (!terminalId) terminalId = event.payload.terminalId;
  });

  row.click();
  out.steps.push("第一次点击会话行");
  for (let index = 0; index < 40 && !terminalId; index += 1) await p.sleep(500);
  out.terminalId = terminalId;
  if (!terminalId) return { ...out, verdict: "FAIL：点了会话但没有终端产出，拿不到终端编号" };

  // 处理启动提示：**升级一律跳过、信任目录才按回车**。
  for (let round = 0; round < 6; round += 1) {
    await p.sleep(3000);
    const text = p.rows();
    if (/Ask Codex to do anything|for shortcuts/i.test(text)) break;
    if (/Update available/i.test(text)) {
      await safe("w", () => p.invoke("terminal:write", { terminalId, data: "\u001b" }));
      out.steps.push("跳过升级");
      continue;
    }
    if (/Trust this folder\?/i.test(text)) {
      await safe("w", () => p.invoke("terminal:write", { terminalId, data: "\r" }));
      out.steps.push("信任目录");
      continue;
    }
    if (/Use existing model/i.test(text)) {
      await safe("w", () => p.invoke("terminal:write", { terminalId, data: "\u001b" }));
      out.steps.push("用现有模型");
      continue;
    }
  }
  out.beforeExit = snapshot("退出前");

  // ── ② 敲 /exit：**打字与回车分开** ─────────────────────────
  const exitsBefore = exits.filter((item) => item.terminalId === terminalId).length;
  out.exitsBeforeTyping = exitsBefore;

  await safe("type", () => p.invoke("terminal:write", { terminalId, data: "/exit" }));
  await p.sleep(900);
  const typedScreen = p.rows();
  out.screenAfterTyping = typedScreen.slice(0, 120);

  // **自证一**：输入框里得真的有 `/exit`。
  out.inputShowsExit = /\/exit/i.test(typedScreen);
  // **自证二**：按下回车**之前**不该有这一路的退出事件。
  out.exitBeforeEnter = exits.some((item) => item.terminalId === terminalId);

  if (!out.inputShowsExit || out.exitBeforeEnter) {
    out.verdict = out.exitBeforeEnter
      ? "INVALID：还没按回车就出现了退出事件——这个退出不是探针造成的"
      : `INVALID：按回车前输入框里没出现 /exit。屏幕：${out.screenAfterTyping}`;
    return out;
  }

  const sentAt = Date.now();
  await safe("enter", () => p.invoke("terminal:write", { terminalId, data: "\r" }));
  out.steps.push("已单独发送回车");

  for (
    let index = 0;
    index < 40 && !exits.some((item) => item.terminalId === terminalId && item.at >= sentAt);
    index += 1
  ) {
    await p.sleep(500);
  }
  await p.sleep(2500);

  out.exits = exits;
  out.ourExit = exits.find((item) => item.terminalId === terminalId && item.at >= sentAt) || null;
  out.afterExit = snapshot("退出后");

  if (!out.ourExit) {
    out.verdict = `FAIL：回车之后没有等到这一路的退出事件。屏幕：${out.afterExit.screen}`;
    return out;
  }
  if ((out.ourExit.exitCode ?? 0) !== 0) {
    out.verdict = `INVALID：退出码是 ${out.ourExit.exitCode}，不是干净退出——这条路径本就不该收标签`;
    return out;
  }

  // ── ③ 再次进入 ────────────────────────────────────────────
  out.sidebarBeforeReentry = sidebarState();
  out.rowsReadyAgain = await ensureRows();
  out.sidebarAfterEnsure = sidebarState();
  const row2 = findRow(pick.title);
  if (!row2) {
    out.verdict = `FAIL：再次进入时找不到会话行。左侧状态：${JSON.stringify(out.sidebarAfterEnsure)}`;
    return out;
  }

  let newTerminalId = null;
  await window.__TAURI__.event.listen("terminal:data", (event) => {
    if (event.payload.terminalId !== terminalId && !newTerminalId) {
      newTerminalId = event.payload.terminalId;
    }
  });

  row2.click();
  out.steps.push("第二次点击会话行");
  for (let index = 0; index < 40 && !newTerminalId; index += 1) await p.sleep(500);
  await p.sleep(3000);

  out.newTerminalId = newTerminalId;
  out.reentry = snapshot("再次进入后");

  const tabsAfterExit = out.afterExit.tabs;
  const cleanExitCollectedTab = tabsAfterExit < out.beforeExit.tabs;

  if (newTerminalId && cleanExitCollectedTab) {
    out.verdict =
      "PASS：干净退出收掉了标签（未留死终端），再次进入起了**新的**终端，没有被锁住";
  } else if (newTerminalId) {
    out.verdict = "PASS（标签未收，但再次进入确实起了新终端）";
  } else {
    out.verdict = "FAIL：再次进入没有起新终端——就是「被锁住」";
  }
  return out;
})();
