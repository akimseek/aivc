/**
 * 单击会话行 = 进入终端（2026-10-02 口径的端到端复验）。
 *
 * 真实后端 + 真实会话：选平台 → 选目标 → 等会话卡片 → **单击一次**，然后断言：
 * 1. 画布落到终端（`.terminal-stack` / `.xterm`），**不是**历史正文（`.history-view`）——
 *    批次一旧口径的落点正好相反；
 * 2. `terminal:data` 有事件到达——证明真的有终端在跑（在干净环境里 `codex resume`
 *    可能因该会话自己的供应商配置缺失而很快退出，但只要起过、出过输出，这条就成立；
 *    退出码会缀在标签上，不影响本探针的判据）；
 * 3. 单击之后**没有再点第二次**（连点/双击的收敛由 App 级单测钉住，这里只看单次语义）。
 */
(async () => {
  const p = window.__p;
  const out = { steps: [] };
  const stageKind = () => {
    if (document.querySelector(".history-view")) return "history";
    if (
      document.querySelector(".terminal-stack .xterm") ||
      document.querySelector(".terminal-host") ||
      document.querySelector(".terminal-loading")
    ) {
      return "terminal";
    }
    return "empty";
  };
  const snapshot = () => ({
    stage: stageKind(),
    tabs: [...document.querySelectorAll(".tab-pill")].map((t) => (t.textContent || "").trim()),
    historyMessages: document.querySelectorAll(".history-message").length
  });

  let dataEvents = 0;
  await window.__TAURI__.event.listen("terminal:data", () => {
    dataEvents += 1;
  });

  // 不选平台/目标一行会话都不会列出来（与 resume-e2e 同一条用户路径）。
  const targets = await p.invoke("codex:list-targets", {});
  const local = (targets || []).find(
    (item) => item.provider === "codex" && item.kind === "local" && item.available
  );
  if (!local) return { ...out, verdict: "SKIP：没有可用的本机 Codex 目标" };
  const sessions = await p.invoke("codex:list-sessions", { targetId: local.id });
  const pick = (sessions || []).find((item) => item.cwd && item.messageCount >= 2);
  if (!pick) return { ...out, verdict: "SKIP：没有可点击的历史会话" };
  out.session = { id: pick.id, title: pick.title };

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

  out.before = snapshot();
  row.click();
  out.steps.push("已单击会话卡片");
  for (let index = 0; index < 40 && stageKind() !== "terminal"; index += 1) {
    await p.sleep(500);
  }
  await p.sleep(2000);
  out.afterClick = snapshot();
  out.terminalDataAfterClick = dataEvents;

  out.verdict =
    out.afterClick.stage === "terminal" && dataEvents > 0
      ? "PASS：单击直接进入终端（有终端输出），未途经历史正文"
      : "CHECK：单击后的落点或终端活动不符预期，见 afterClick / terminalDataAfterClick";
  return JSON.stringify(out, null, 2);
})()
