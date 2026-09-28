/**
 * 复现「`/exit` 退出会话后，再次进入被锁住」。
 *
 * # 两个候选解释（都要靠这次实测区分）
 *
 * 1. **界面留着一个死标签**：终端退出后 `use-terminals` 只把 `running` 置 false、
 *    **不关标签**，而 `activateSessionTab` 又会优先切到「已挂着这条会话的标签」——
 *    于是再次进入时，当前标签可能还是那一路**已经死掉的**终端。
 * 2. **后端没释放**：`spawn_reaper` 收到退出码后只 `release()`（丢掉 master / writer），
 *    **没有把会话从登记表里摘掉**，`terminalId` 对应的登记项会一直留着。
 *
 * # 判据
 *
 * 逐步记录：退出前 / 退出后 / 再次进入后的
 * 标签数、终端数、**当前标签指向哪一路**、以及新终端有没有产出。
 * 只看「有没有新终端」不够——旧死终端还在的话，用户看到的仍然是它。
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
      /** 左侧列表里还有多少行可点——上一次失败就是这里为 0。 */
      rows: document.querySelectorAll(".row").length,
      rowTitles: [...document.querySelectorAll(".row")].slice(0, 5).map((item) =>
        item.getAttribute("title")
      ),
      /** 左侧面板当前是哪个页（会话 / 收藏 / 搜索）。 */
      pane: [...document.querySelectorAll("button")]
        .filter((item) => item.className.includes("pane"))
        .map((item) => p.clean(item.textContent))
        .join("|"),
      screen: p.rows().slice(0, 60)
    };
  };

  // 记下所有 terminal:exit 与 terminal:start 的往返，用于区分「谁退出了」。
  const exits = [];
  await window.__TAURI__.event.listen("terminal:exit", (event) => {
    exits.push(event.payload && event.payload.terminalId);
  });

  // 挑一条历史会话（与 resume-e2e 同一套：先列目标，再用列出来的 id）。
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

  /**
   * 确保左侧停在**会话列表**上。
   *
   * 前两次跑都栽在同一个地方：点完第一次之后再找会话行，`rows` 是 0——
   * 因为左侧那时根本不在会话页上（探针没选过目标，列表自然是空的）。
   * 探针必须**自己把前置状态摆好**，而不是假定打开就是会话列表。
   */
  const ensureSessionPane = async () => {
    for (let round = 0; round < 10; round += 1) {
      if (document.querySelectorAll(".row").length > 0) return true;
      const paneButton = [...document.querySelectorAll("button")].find((item) =>
        /会话|活跃/.test(item.textContent || "")
      );
      if (paneButton) paneButton.click();
      await p.sleep(700);
    }
    return document.querySelectorAll(".row").length > 0;
  };
  out.paneReady = await ensureSessionPane();
  out.steps.push(`会话页就绪=${out.paneReady}，行数=${document.querySelectorAll(".row").length}`);

  // 抓终端编号：界面不暴露它，只能从数据事件里认。
  let terminalId = null;
  await window.__TAURI__.event.listen("terminal:data", (event) => {
    if (!terminalId) terminalId = event.payload.terminalId;
  });

  const rows = [...document.querySelectorAll(".row")];
  const row =
    rows.find((item) => (item.getAttribute("title") || "").trim() === (pick.title || "").trim()) ||
    rows[0];
  if (!row) return { ...out, verdict: "FAIL：列表里没有会话行" };

  // ① 第一次进入：点会话行。
  row.click();
  out.steps.push("第一次点击会话行");
  for (let index = 0; index < 40 && !terminalId; index += 1) await p.sleep(500);
  out.terminalId = terminalId;

  // 处理启动提示（信任目录 / 新模型 / 升级），全部选「不改变环境」的那一项。
  for (let round = 0; round < 6; round += 1) {
    await p.sleep(3000);
    const text = p.rows();
    if (/\? for shortcuts|Ask Codex/i.test(text)) break;
    if (!terminalId) break;
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
    if (/Update available/i.test(text)) {
      await safe("w", () => p.invoke("terminal:write", { terminalId, data: "\u001b" }));
      out.steps.push("跳过升级");
      continue;
    }
  }
  out.beforeExit = snapshot("退出前");

  // ② 敲 /exit。
  await safe("exit", () => p.invoke("terminal:write", { terminalId, data: "/exit\r" }));
  out.steps.push("已发送 /exit");
  for (let index = 0; index < 40 && exits.length === 0; index += 1) await p.sleep(500);
  await p.sleep(2000);
  out.exits = exits;
  out.afterExit = snapshot("退出后");

  // ③ 再次进入：再点同一条会话行。
  const rows2 = [...document.querySelectorAll(".row")];
  const row2 =
    rows2.find((item) => (item.getAttribute("title") || "").trim() === (pick.title || "").trim()) ||
    rows2[0];
  if (!row2) return { ...out, verdict: "FAIL：再进入时找不到会话行" };
  row2.click();
  out.steps.push("第二次点击会话行");

  // 等新终端产出（或超时）。
  let newTerminalId = null;
  await window.__TAURI__.event.listen("terminal:data", (event) => {
    if (event.payload.terminalId !== terminalId) newTerminalId = event.payload.terminalId;
  });
  for (let index = 0; index < 40 && !newTerminalId; index += 1) await p.sleep(500);
  await p.sleep(3000);
  out.newTerminalId = newTerminalId;
  out.reentry = snapshot("再次进入后");

  // 也直接问后端：登记表里现在有几路会话。
  //
  // `terminal:count` **不存在**（`commands.rs` 里没有这条），所以不做这一步——
  // 要有后端视角的证据，得先加一条只读命令；那属于诊断能力，不在本次修复范围内。
  // 这里只用界面可观察的状态下判断。

  const sameActive =
    out.beforeExit.activeTab !== null && out.reentry.activeTab === out.afterExit.activeTab;
  if (newTerminalId) {
    out.verdict = "再进入起了新的终端（问题只在界面标签上）";
  } else if (out.reentry.xterms >= 1) {
    out.verdict = "再次进入**没有起新终端**，界面停在那一路已退出的终端上";
  } else {
    out.verdict = "再次进入既没有新终端，画面也空了";
  }
  out.sameActive = sameActive;
  return out;
})();
