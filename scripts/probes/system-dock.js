/**
 * 系统终端底部停靠面板。
 *
 * # 为什么这一条必须实测
 *
 * 「系统终端不再是工作区的一张标签，而是底部一块独立的板」是**布局与生命周期**
 * 两方面的改动：
 *
 * - 布局：它得真的落在工作区下面、状态栏上面（挂错一层就会掉到状态栏底下）；
 * - 生命周期：最小化之后终端是否**还活着**、展开后能不能继续用，
 *   只有真跑一遍才知道（卸载会丢 PTY 输出，那是不可逆的）。
 *
 * # 怎么拿到那一路终端的编号
 *
 * 应用没有 `terminal:list`，标签 DOM 上也没有编号。探针**自己拦一层 IPC**
 * （`window.__TAURI__.core.invoke`），从应用自己发出的 `terminal:resize` 里
 * 记下编号。这样不为了测试去改产品代码加 `data-*`。
 *
 * # 自证
 *
 * 面板是探针自己点开的（`dockBefore`），因此后面看到的一切都只可能来自这次操作。
 */
(async () => {
  const p = window.__p;
  const out = { checks: [] };
  const check = (name, ok, detail) => out.checks.push({ name, ok: Boolean(ok), detail: detail ?? "" });

  // ── 怎么拿到那一路终端的编号 ──────────────────────────
  //
  // 应用没有 `terminal:list`，标签 DOM 上也没有编号，而 `window.__TAURI__.core`
  // **每次访问返回一个新对象**（实测：打补丁后 `invoke === wrapper` 是 false），
  // 因此拦 IPC 这条路不通。
  //
  // 可靠的路是**订阅后端推的事件**：`terminal:data` 的载荷里就带 `terminalId`。
  // 顺便替它应答 DSR（ConPTY 的光标查询），不应答时子进程连第一步都走不完。
  //
  // **但应答必须停。** DSR 的应答是写进 PTY 的（`\x1b[1;1R`），ConPTY 通常会把
  // 它当应答吃掉；如果它恰好在 shell 已经进入「读命令行」的时候到达，
  // 那几个字节就进了输入行，把命令拼坏——实测表现是 `CommandNotFoundException`，
  // 而同一路终端上后一次写入完全正常。这是**探针污染了自己的输入**，不是产品缺陷。
  //
  // 因此：只在启动阶段应答（这段时间 ConPTY 确实在等），提示符稳定后就停。
  let answerDsr = true;
  window.setTimeout(() => {
    answerDsr = false;
  }, 12000);
  const terminalIds = [];
  await window.__TAURI__.event.listen("terminal:data", (event) => {
    const id = event.payload?.terminalId;
    if (typeof id !== "string" || !id) return;
    if (!terminalIds.includes(id)) terminalIds.push(id);
    const text = new TextDecoder().decode(Uint8Array.from(event.payload.data ?? []));
    if (answerDsr && text.includes("\u001b[6n")) {
      p.invoke("terminal:write", { terminalId: id, data: "\u001b[1;1R" });
    }
  });

  const dock = () => document.querySelector(".system-dock");
  const rectOf = (selector) => {
    const element = document.querySelector(selector);
    return element ? element.getBoundingClientRect() : null;
  };
  const dockRows = () => {
    const rows = dock()?.querySelector(".xterm-rows");
    // `p.clean(undefined)` 会抛，而探针抛异常只留下一句「求值结果抛异常」——
    // 拿不到就读成空串，让断言自己去失败。
    return rows ? p.clean(rows.textContent) : "";
  };
  const dockButtons = () =>
    [...(dock()?.querySelectorAll("button") ?? [])].map((item) => p.clean(item.textContent));
  const clickText = async (scope, label) => {
    const root = scope ? document.querySelector(scope) : document;
    const button = [...(root?.querySelectorAll("button") ?? [])].find(
      (item) => p.clean(item.textContent) === label
    );
    if (!button) return false;
    button.click();
    await p.sleep(400);
    return true;
  };

  await p.waitForTargets();
  out.dockBefore = Boolean(dock());

  // 工作区标签条右端的按钮写的是「+ Shell」（见 `WorkspaceTrailing`），
  // 不是「新建终端」——第一版探针按后者找，找不到就静默返回 false，
  // 于是整条验收报「面板没出现」，看起来像产品缺陷，其实是探针点错了按钮。
  const NEW_SHELL_LABELS = ["+ Shell", "新建终端"];
  const opened = await (async () => {
    for (const label of NEW_SHELL_LABELS) {
      if (await clickText(null, label)) return label;
    }
    return false;
  })();
  out.clickedNewShell = opened;
  for (let i = 0; i < 40 && !dock(); i += 1) await p.sleep(500);

  check("点「新建终端」后底部停靠面板出现", Boolean(dock()), `opened=${opened}`);
  if (!dock()) {
    out.verdict = "FAIL：停靠面板没有出现，后续断言无法进行";
    return out;
  }
  await p.sleep(3500);

  out.terminalIds = terminalIds;
  const terminalId = terminalIds[0];
  check(
    "从 terminal:data 事件里拿到了这一路终端的编号（后续写命令要用）",
    Boolean(terminalId),
    `ids=${JSON.stringify(terminalIds)}`
  );
  if (!terminalId) {
    out.verdict = "FAIL：拿不到终端编号，PTY 存活那两条无法验证（这是探针的问题，不是产品的）";
    return out;
  }

  // ── 位置 ─────────────────────────────────────────────
  const d = rectOf(".system-dock");
  const w = rectOf(".workspace");
  const s = rectOf(".statusbar");
  out.rects = {
    dock: d && { top: Math.round(d.top), bottom: Math.round(d.bottom), height: Math.round(d.height) },
    workspace: w && { top: Math.round(w.top), bottom: Math.round(w.bottom), height: Math.round(w.height) },
    status: s && { top: Math.round(s.top) }
  };

  check(
    "停靠面板在工作区**下方**",
    d && w && d.top >= w.bottom - 2,
    `dock.top=${out.rects.dock?.top} workspace.bottom=${out.rects.workspace?.bottom}`
  );
  check(
    "停靠面板在状态栏**上方**（挂错一层就会掉到状态栏底下）",
    !s || (d && d.bottom <= s.top + 2),
    `dock.bottom=${out.rects.dock?.bottom} status.top=${out.rects.status?.top}`
  );
  check("停靠面板有实际高度", d && d.height > 100, `height=${out.rects.dock?.height}`);

  // ── 系统终端不在工作区标签条里 ───────────────────────
  //
  // 断言的是「工作区标签条里**没有系统终端**」，不是「标签条是空的」——
  // 空的会误报：恢复一条会话本来就会在工作区留一张会话标签，那是对的。
  const workspaceTabs = [...document.querySelectorAll(".workspace .tabs .tab")].map((item) =>
    p.clean(item.textContent)
  );
  out.workspaceTabs = workspaceTabs;
  check(
    "系统终端**不在**工作区标签条里（它是底板，不是标签）",
    !workspaceTabs.some((label) => label.includes("系统终端")),
    JSON.stringify(workspaceTabs)
  );
  const dockTabs = [...(dock()?.querySelectorAll(".tabs .tab") ?? [])].map((item) =>
    p.clean(item.textContent)
  );
  out.dockTabs = dockTabs;
  check("系统终端出现在底部面板的标签条里", dockTabs.length >= 1, JSON.stringify(dockTabs));

  // ── PTY 真的活着 ─────────────────────────────────────
  /**
   * 在停靠面板里跑一条命令并等它回显。
   *
   * 带一次重试：第一次写入可能与 shell 的初始化抢时间；**重试成功就说明是探针的时序问题，
   * 不是产品缺陷**——这一点必须能从结果里看出来，否则会把探针的毛病报成产品的问题。
   */
  const runInDock = async (marker) => {
    for (let attempt = 1; attempt <= 2; attempt += 1) {
      p.invoke("terminal:write", { terminalId, data: `echo ${marker}\r` });
      for (let waited = 0; waited < 12; waited += 1) {
        await p.sleep(500);
        if (dockRows().includes(marker)) return { ok: true, attempt };
      }
    }
    return { ok: false, attempt: 0 };
  };

  out.dockTextBefore = dockRows().slice(-200);
  const firstRun = await runInDock("DOCK-MARK-7788");
  await p.sleep(500);
  const after = dockRows();
  out.dockTail = after.slice(-200);
  out.firstRunAttempt = firstRun.attempt;
  check(
    "在停靠面板里敲命令，输出真的回来了（PTY 活着）",
    firstRun.ok,
    `attempt=${firstRun.attempt} :: ${after.slice(-140)}`
  );

  // ── 最小化 ───────────────────────────────────────────
  const workspaceHeightBefore = rectOf(".workspace")?.height ?? 0;
  await clickText(".system-dock", "最小化");
  await p.sleep(700);
  const minimizedHeight = rectOf(".system-dock")?.height ?? -1;
  const bodyDisplay = (() => {
    const body = dock()?.querySelector(".system-dock-body");
    return body ? window.getComputedStyle(body).display : "missing";
  })();
  const workspaceHeightAfter = rectOf(".workspace")?.height ?? 0;
  out.minimized = {
    height: Math.round(minimizedHeight),
    bodyDisplay,
    workspaceBefore: Math.round(workspaceHeightBefore),
    workspaceAfter: Math.round(workspaceHeightAfter),
    buttons: dockButtons()
  };

  check(
    "最小化后面板只剩一条（高度明显变小）",
    minimizedHeight > 0 && minimizedHeight < 90,
    `height=${out.minimized.height}`
  );
  check("最小化后终端本体不显示", bodyDisplay === "none", `display=${bodyDisplay}`);
  check(
    "最小化后仍有「展开」这个出口（不能是死路）",
    out.minimized.buttons.includes("展开"),
    JSON.stringify(out.minimized.buttons)
  );
  check(
    "最小化让出来的高度还给了工作区",
    workspaceHeightAfter > workspaceHeightBefore,
    `${out.minimized.workspaceBefore} → ${out.minimized.workspaceAfter}`
  );

  // ── 展开 ─────────────────────────────────────────────
  await clickText(".system-dock", "展开");
  await p.sleep(800);
  const restoredHeight = rectOf(".system-dock")?.height ?? -1;
  out.restoredHeight = Math.round(restoredHeight);
  check("展开后高度回来了", restoredHeight > 100, `height=${out.restoredHeight}`);

  const restoredRun = await runInDock("BACK-9911");
  const afterRestore = dockRows();
  out.dockTailAfterRestore = afterRestore.slice(-200);
  out.restoredRunAttempt = restoredRun.attempt;
  check(
    "展开后终端仍可用（最小化没有把 PTY 弄死）",
    restoredRun.ok,
    `attempt=${restoredRun.attempt} :: ${afterRestore.slice(-140)}`
  );

  const failed = out.checks.filter((item) => !item.ok);
  out.verdict =
    failed.length === 0
      ? `PASS：${out.checks.length} 项系统终端停靠面板全部通过`
      : `FAIL：${failed.length}/${out.checks.length} 未通过 —— ${failed.map((f) => f.name).join("、")}`;
  return out;
})();
