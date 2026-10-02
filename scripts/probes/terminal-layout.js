/**
 * 诊断：进入终端后，自管输入框与终端区的几何关系。
 *
 * 用户报告「底部部分被自管输入框遮挡」。这个探针不改任何东西，只量：
 * - `.terminal-slot` / `.terminal-host` / `.xterm` / `.xterm-screen` / `.composer` 的包围盒；
 * - xterm 的行数、首行/末行位置；
 * - 关键差值：xterm 底 vs host 底（>0 = xterm 超出宿主区被裁）、末行底 vs host 底、
 *   host 底 vs composer 顶（>0 = 两块重叠）。
 *
 * 与 session-click-enter.js 同一条进入路径：选平台 → 选目标 → 单击会话卡片。
 */
(async () => {
  const p = window.__p;
  const out = { steps: [] };
  const rect = (el) => {
    if (!el) return null;
    const b = el.getBoundingClientRect();
    return { top: Math.round(b.top), bottom: Math.round(b.bottom), h: Math.round(b.height) };
  };

  // 进入终端（单击 = 进入终端，2026-10-02 口径）。
  const targets = await p.invoke("codex:list-targets", {});
  const local = (targets || []).find(
    (item) => item.provider === "codex" && item.kind === "local" && item.available
  );
  if (!local) return { ...out, verdict: "SKIP：没有可用的本机 Codex 目标" };
  const sessions = await p.invoke("codex:list-sessions", { targetId: local.id });
  const pick = (sessions || []).find((item) => item.cwd && item.messageCount >= 2);
  if (!pick) return { ...out, verdict: "SKIP：没有可点击的历史会话" };

  const first = [...document.querySelectorAll(".sidebar .pickers select")];
  const providerOption = [...(first[0]?.options ?? [])].find((item) => item.value === "codex");
  if (first[0] && providerOption) {
    p.selectValue(first[0], "codex");
    await p.sleep(2500);
  }
  const selects = [...document.querySelectorAll(".sidebar .pickers select")];
  const option = [...(selects[1]?.options ?? [])].find((item) => item.value !== "");
  if (!option) return { ...out, verdict: "SKIP：目标下拉里没有可选项" };
  p.selectValue(selects[1], option.value);
  await p.sleep(2500);
  for (let index = 0; index < 20 && document.querySelectorAll(".session-card").length === 0; index += 1) {
    await p.sleep(500);
  }
  const rows = [...document.querySelectorAll(".session-card")];
  const row =
    rows.find((item) => (item.getAttribute("title") || "").trim() === (pick.title || "").trim()) ||
    rows[0];
  if (!row) return { ...out, verdict: "FAIL：列表里没有会话行" };
  row.click();
  out.steps.push("已单击会话卡片");

  const slot = () => document.querySelector(".terminal-slot:not(.inactive)");
  for (let index = 0; index < 40 && !slot()?.querySelector(".xterm"); index += 1) {
    await p.sleep(500);
  }
  // 自管输入框：恢复会话时它等第一段输出才出现。
  for (let index = 0; index < 20 && !slot()?.querySelector(".composer"); index += 1) {
    await p.sleep(500);
  }
  await p.sleep(2000);

  const host = slot()?.querySelector(".terminal-host");
  const composer = slot()?.querySelector(".composer");
  const xterm = slot()?.querySelector(".xterm");
  const screen = slot()?.querySelector(".xterm-screen");
  const rowEls = [...(slot()?.querySelectorAll(".xterm-rows > div") ?? [])];

  out.geometry = {
    viewportH: window.innerHeight,
    slot: rect(slot()),
    host: rect(host),
    composer: rect(composer),
    xterm: rect(xterm),
    screen: rect(screen),
    rowCount: rowEls.length,
    firstRow: rect(rowEls[0]),
    lastRow: rect(rowEls[rowEls.length - 1])
  };
  const hb = host?.getBoundingClientRect();
  const xb = xterm?.getBoundingClientRect();
  const lb = rowEls[rowEls.length - 1]?.getBoundingClientRect();
  const cb = composer?.getBoundingClientRect();
  out.deltas = {
    // >0：xterm 画布比宿主还高（被 overflow 裁掉）
    xtermOverHost: hb && xb ? Math.round(xb.bottom - hb.bottom) : null,
    // >0：最后一行被裁的高度
    lastRowOverHost: hb && lb ? Math.round(lb.bottom - hb.bottom) : null,
    // >0：宿主底伸进了输入框区域（两块重叠）
    hostIntoComposer: hb && cb ? Math.round(hb.bottom - cb.top) : null
  };
  out.verdict =
    out.deltas.xtermOverHost > 2 || out.deltas.lastRowOverHost > 2 || out.deltas.hostIntoComposer > 2
      ? "REPRO：几何上存在裁切/重叠，见 deltas"
      : "OK：无裁切无重叠";
  return JSON.stringify(out, null, 2);
})()
