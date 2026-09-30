/**
 * 验证「点历史会话 → 恢复那次会话进终端」。
 *
 * # 一路上踩到的四件事（都写进注释了）
 *
 * 1. **目标 id 不能写死**：目标表要在 `codex:list-targets` 跑过之后才有内容，
 *    否则 `codex:list-sessions` 报「未解析到目标」。
 * 2. Codex 启动可能先问**升级**（`Update available … enter continue · esc skip`）。
 * 3. 紧接着可能问**目录信任**（`Trust this folder? … enter continue · esc quit`）。
 * 4. **不能按固定顺序按键。** 这是最后一次踩的坑：第一次跑时探针误按回车，
 *    Codex 真的把升级装上了；第二次跑就没有升级提示了，而探针仍先按 esc ——
 *    那一刻屏幕上其实是**信任提示**，那里的 `esc` 是 **quit**，
 *    于是终端被自己关掉，后续只剩一句「终端已关闭」。
 *
 * **还有一条铁律（事故换来的）：绝不对可能改动用户环境的提示按「确认」。**
 *
 * 上面第 4 条里那个「误按回车」不是小事——它真的执行了
 * `npm install -g @openai/codex`，把用户本机的 Codex 从 0.156.1 升到了 0.157.1。
 * 用户随后在物理机终端里撞上 0.157 的后台守护进程报错，**根源就在这一下**。
 *
 * 因此：**升级提示一律选「跳过」，目录信任才允许按回车**（信任只写本地配置、
 * 不改变安装），拿不准就不按。
 *
 * 所以现在是**读屏幕再决定按哪个键**：看到升级提示按 esc（跳过），
 * 看到信任提示按回车（信任），出现 `session id:` 就停下。
 *
 * # 判据
 *
 * 只看「有没有请求」不够（新建会话也会有）。真正的证据是 Codex 横幅里的
 * `session id:` **等于点的那一条**——那才叫「恢复了」。
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

  await safe("vendor", () =>
    p.invoke("vendor:save", {
      providerId: "codex",
      name: "恢复探针供应商",
      apiKey: "sk-probe",
      apiBaseUrl: "http://127.0.0.1:1",
      enabled: true,
      sort: 1
    })
  );

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
  out.clickedSessionId = pick.id;

  // **在界面上选中平台与目标**：应用启动后什么都不选（批次一去掉「还原上次选择」），
  // 不选就一行会话都不会列出来。这里走用户真实的两步。
  out.targetSelect = await safe("targetSelect", async () => {
    const pickers = [...document.querySelectorAll(".sidebar .pickers select")];
    const providerOption = [...(pickers[0]?.options ?? [])].find((item) => item.value === "codex");
    if (pickers[0] && providerOption) {
      p.selectValue(pickers[0], "codex");
      await p.sleep(2500);
    }
    const selects = [...document.querySelectorAll(".sidebar .pickers select")];
    // 第一个非空选项：本机目标的编号形如 `codex:local:C:\...\.codex`，不以 `:local` 结尾。
    const option = [...(selects[1]?.options ?? [])].find((item) => item.value !== "");
    if (!option) return { ok: false, reason: "目标下拉里没有可选项" };
    p.selectValue(selects[1], option.value);
    await p.sleep(2500);
    return { ok: true, picked: option.value };
  });

  let terminalId = null;
  await window.__TAURI__.event.listen("terminal:data", (event) => {
    if (!terminalId) terminalId = event.payload.terminalId;
  });

  out.terminalsBefore = document.querySelectorAll(".terminal-stack .xterm").length;
  // 批次一的类名：标签是 `.tab-pill`，会话卡片是 `.session-card`。
  out.tabsBefore = document.querySelectorAll(".tab-pill").length;

  for (let index = 0; index < 20 && document.querySelectorAll(".session-card").length === 0; index += 1) {
    await p.sleep(500);
  }
  const rows = [...document.querySelectorAll(".session-card")];
  const row =
    rows.find((item) => (item.getAttribute("title") || "").trim() === (pick.title || "").trim()) ||
    rows[0];
  if (!row) return { ...out, verdict: "FAIL：列表里没有会话行" };
  // **双击**才是「在终端中恢复」：单击只打开历史正文（`session-card.tsx`）。
  row.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
  out.steps.push("已双击会话卡片");

  for (let index = 0; index < 40 && !terminalId; index += 1) await p.sleep(500);
  out.terminalId = terminalId;

  // 读屏幕 → 决定按键。**不假设提示的顺序**，每一轮都重新判断。
  for (let round = 0; round < 8; round += 1) {
    await p.sleep(3000);
    const text = p.rows();
    out.steps.push(`第 ${round + 1} 轮：${text.slice(0, 36)}`);

    if (/session id:/i.test(text)) {
      out.screen = text;
      break;
    }
    if (!terminalId) {
      out.steps.push("没有终端编号，停止");
      break;
    }
    if (/Update available/i.test(text)) {
      // **一律跳过升级。** 按回车会真的执行 `npm install -g @openai/codex`，
      // 而探针**没有资格改动用户安装的 CLI 版本**——上一次就是这么把 0.156.1
      // 升成 0.157.1 的，用户随后撞上了 0.157 的后台守护进程报错。
      await safe("write", () => p.invoke("terminal:write", { terminalId, data: "\u001b" }));
      out.steps.push("按 esc 跳过升级（绝不选「立即更新」）");
      continue;
    }
    if (/Trust this folder\?/i.test(text)) {
      // 信任只写本地配置、不改变安装，因此这一处按回车是安全的。
      await safe("write", () => p.invoke("terminal:write", { terminalId, data: "\r" }));
      out.steps.push("按回车信任目录");
      continue;
    }
    // 第四个交互点：**新模型推荐**
    // `Meet GPT-6 Sol … 1. Try new model · 2. Use existing model · enter/esc confirm`。
    // 按 esc = 用现有模型：探针要验的是「会话有没有被恢复」，不该顺带换模型。
    if (/Use existing model/i.test(text)) {
      await safe("write", () => p.invoke("terminal:write", { terminalId, data: "\u001b" }));
      out.steps.push("按 esc 用现有模型");
      continue;
    }
    out.screen = text;
  }

  out.terminalsAfter = document.querySelectorAll(".terminal-stack .xterm").length;
  out.tabsAfter = document.querySelectorAll(".tab-pill").length;
  const usage = await safe("usage", () => p.usage());
  out.requestCount = usage ? usage.requestCount : 0;

  const banner = out.screen || "";
  out.bannerSessionId = (banner.match(/session id:\s*([0-9a-f-]+)/i) || [])[1] || null;

  // **决定性证据：屏幕上出现了那次会话的历史。**
  //
  // 交互式 TUI **不会**打印 `session id:`（那是 `codex exec` 的横幅），所以原来那条
  // 断言不成立。换成这个：探针挑的那条会话，其记录里有一次**旧网关失败**，
  // 里面带着**上一轮探针的路由名** `probe-e2e-codex`——而本次终端叫 `term-…`。
  //
  // 屏幕上出现 `probe-e2e-codex`，只可能是**把历史记录回放出来了**：
  // 新建的会话不可能知道那个名字。
  out.historyReplayed = /probe-e2e-codex/i.test(banner);
  const resumed = out.historyReplayed || out.bannerSessionId === pick.id;

  if (out.terminalsAfter > out.terminalsBefore && out.tabsAfter === out.tabsBefore + 1 && resumed) {
    out.verdict = "PASS：点历史会话 → 恰好一个终端标签 → 屏幕上是那次会话的历史记录";
  } else if (out.terminalsAfter > out.terminalsBefore && resumed) {
    out.verdict = "PASS（标签数未断言）：会话确实被恢复";
  } else if (out.terminalsAfter > out.terminalsBefore) {
    out.verdict = `PARTIAL：终端起来了，但没有会话历史的证据。屏幕：${banner.slice(0, 120)}`;
  } else {
    out.verdict = "FAIL：点了会话但没有出现终端";
  }
  return out;
})();
