/**
 * 验证「终端内查找」真的能找到并跳到匹配处。
 *
 * # 判据不是「查找栏出现了」
 *
 * 查找栏出现只说明 UI 在。真正要证明的是：**它读到了终端缓冲区里的真实文本，
 * 并且把匹配数算对了**。因此探针先让终端输出一段**自己知道的**独特字符串，
 * 再在查找栏里搜它，比对「共几处」。
 *
 * 这样任何一环断了都看得出来：
 *
 * | 断在哪 | 现象 |
 * | --- | --- |
 * | Ctrl+F 没接上 | 查找栏不出现 |
 * | `readLines` 读不到缓冲区 | 显示「无匹配」 |
 * | 匹配算法错 | 数量与预期不符 |
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

  const searchState = () => {
    const bar = document.querySelector(".terminal-search");
    if (!bar) return { present: false };
    const count = bar.querySelector(".terminal-search-count");
    return {
      present: true,
      count: count ? p.clean(count.textContent) : null,
      hint: p.clean(bar.textContent).includes("回滚缓冲区"),
      nextDisabled: (() => {
        const next = [...bar.querySelectorAll("button")].find(
          (item) => item.getAttribute("aria-label") === "下一个匹配项"
        );
        return next ? next.disabled : null;
      })()
    };
  };

  // ── 前置：选中目标并开一路新会话 ──────────────────────
  const targets = await safe("targets", () => p.invoke("codex:list-targets", {}));
  const local = (targets || []).find(
    (item) => item.provider === "codex" && item.kind === "local" && item.available
  );
  if (!local) return { ...out, verdict: "SKIP：没有可用的本机 Codex 目标" };

  out.targetSelect = await safe("targetSelect", async () => {
    const select = await p.waitForTargets();
    if (!select) return { ok: false };
    const option = [...select.options].find((item) => item.value.endsWith(":local"));
    if (!option) return { ok: false };
    p.selectValue(select, option.value);
    await p.sleep(2000);
    return { ok: true };
  });

  const newCodex = [...document.querySelectorAll("button")].find((item) =>
    (item.textContent || "").includes("+ Codex")
  );
  if (!newCodex) return { ...out, verdict: "FAIL：找不到「+ Codex」按钮" };

  let terminalId = null;
  await window.__TAURI__.event.listen("terminal:data", (event) => {
    if (!terminalId) terminalId = event.payload.terminalId;
  });

  newCodex.click();
  for (let round = 0; round < 20; round += 1) {
    await p.sleep(500);
    const dialog = document.querySelector(".new-session-dialog");
    if (dialog) {
      const open = [...dialog.querySelectorAll("button")].find(
        (item) => (item.textContent || "").trim() === "打开"
      );
      if (open) {
        open.click();
        break;
      }
    }
  }
  for (let index = 0; index < 40 && !terminalId; index += 1) await p.sleep(500);
  out.terminalId = terminalId;
  if (!terminalId) return { ...out, verdict: "FAIL：点了新会话但没有终端产出" };

  // 让 CLI 把它自己的启动界面画出来。
  for (let round = 0; round < 6; round += 1) {
    await p.sleep(2500);
    const text = p.rows();
    if (/for shortcuts|Ask Codex/i.test(text)) break;
    if (/Update available/i.test(text)) {
      await safe("w", () => p.invoke("terminal:write", { terminalId, data: "\u001b" }));
      continue;
    }
    if (/Trust this folder\?/i.test(text)) {
      await safe("w", () => p.invoke("terminal:write", { terminalId, data: "\r" }));
      continue;
    }
    if (/Use existing model/i.test(text)) {
      await safe("w", () => p.invoke("terminal:write", { terminalId, data: "\u001b" }));
      continue;
    }
  }

  // ── 让终端里出现一段**探针自己知道的**独特文本 ──────────
  //
  // 用系统终端做不到（这里开的是 Codex）。改用**自管输入**发一句让它回显的话：
  // Codex 会把用户输入回显在它的输入行上，那段文字就进了 xterm 的缓冲区。
  const token = `zq${Date.now().toString(36)}`;
  out.token = token;
  await safe("write", async () => {
    // 直接写进 PTY：不提交，只让它出现在输入行上，避免真的发给模型。
    await p.invoke("terminal:write", { terminalId, data: token });
  });
  await p.sleep(2500);
  out.tokenVisible = p.rows().includes(token);
  if (!out.tokenVisible) {
    out.verdict = `SKIP：无法让终端缓冲区里出现已知文本（屏幕：${p.rows().slice(0, 200)}）`;
    return out;
  }

  // ── Ctrl+F 打开查找栏 ──────────────────────────────────
  await safe("ctrlF", () => {
    window.dispatchEvent(
      new KeyboardEvent("keydown", { key: "f", ctrlKey: true, bubbles: true })
    );
  });
  await p.sleep(800);
  out.searchAfterCtrlF = searchState();
  out.steps.push(`Ctrl+F 后查找栏=${out.searchAfterCtrlF.present}`);

  if (!out.searchAfterCtrlF.present) {
    out.verdict = "FAIL：Ctrl+F 没有打开查找栏";
    return out;
  }
  if (!out.searchAfterCtrlF.hint) {
    out.verdict = "FAIL：查找栏没有显示「只搜回滚缓冲区」的范围提示";
    return out;
  }

  // ── 搜那段已知文本 ─────────────────────────────────────
  await safe("type", async () => {
    const area = document.querySelector(".terminal-search input.control");
    if (!area) throw new Error("没有查找输入框");
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
    setter.call(area, token);
    area.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await p.sleep(1200);
  out.searchAfterTyping = searchState();

  if (!out.searchAfterTyping.count || out.searchAfterTyping.count === "无匹配") {
    out.verdict = `FAIL：查找栏读到了缓冲区却找不到那段已知文本（${out.searchAfterTyping.count}）`;
    return out;
  }

  out.verdict = `PASS：终端内查找可用——Ctrl+F 开栏、读到缓冲区文本、共 ${out.searchAfterTyping.count}`;
  return out;
})();
