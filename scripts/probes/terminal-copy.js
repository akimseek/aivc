/**
 * 诊断 + 回归：终端右键菜单的「复制」。
 *
 * 用户报告：「选中部分文字后右键想复制，菜单弹出后选中变成未选中，复制不了。」
 *
 * # 探针做什么
 *
 * 在真实应用里开一路终端，拿 Terminal 实例（React fiber 走查，不走应用后门），
 * 程序化制造选区，然后**合成真实形态的右键事件序列**（mousedown → mouseup →
 * contextmenu），在两个变体下量同一组事实：
 *
 * - V1 鼠标上报关闭（普通 shell 形态）；
 * - V2 鼠标上报开启（`\x1b[?1000h`，agent 类 TUI 的常见形态）。
 *
 * 每个变体量：mousedown 后 / mouseup 后 / contextmenu 后选区是否还在；菜单的
 * 「复制」是否可用；点「复制」后**读回剪贴板**（先放哨兵值）断言端到端结果。
 *
 * # 判读
 *
 * - 修复前：V1 正常（正对照：证明探针本身能走通复制）；V2 应在 mousedown 处就
 *   被清选区（xterm 把右键作为输入上报 → `onUserInput` → `clearSelection`）、
 *   菜单「复制」禁用、剪贴板仍是哨兵 → REPRO。
 * - 修复后：V2 与 V1 都应「选区存活 → 复制可用 → 剪贴板 = 选中文本」。
 *
 * # 边界
 *
 * - 会动系统剪贴板：先存原文，跑完尽量还原（原文为空或为图片时无法还原，接受）。
 * - 合成事件的坐标取终端屏中央；不依赖真实鼠标。
 */
(async () => {
  const p = window.__p;
  const out = { steps: [] };

  // —— 进入终端（与 terminal-layout.js 同一条路径）——
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
  const cards = [...document.querySelectorAll(".session-card")];
  const row =
    cards.find((item) => (item.getAttribute("title") || "").trim() === (pick.title || "").trim()) ||
    cards[0];
  if (!row) return { ...out, verdict: "FAIL：列表里没有会话行" };
  row.click();
  out.steps.push("已单击会话卡片");

  const slot = () => document.querySelector(".terminal-slot:not(.inactive)");
  for (let index = 0; index < 40 && !slot()?.querySelector(".xterm"); index += 1) {
    await p.sleep(500);
  }

  /**
   * 拿 xterm 的 Terminal 实例。
   *
   * 走 React fiber：`.terminal-mount` 的 fiber 沿 `return` 上溯到 TerminalPane，
   * 遍历 hooks 找 `ref.current` 里带 `getSelection` + `write` 的那个（就是 termRef）。
   * 只读，不注入任何应用侧钩子。
   */
  const findTerminal = () => {
    const el = document.querySelector(".terminal-slot:not(.inactive) .terminal-mount");
    if (!el) return null;
    const key = Object.keys(el).find((name) => name.startsWith("__reactFiber$"));
    if (!key) return null;
    let fiber = el[key];
    for (let depth = 0; fiber && depth < 60; depth += 1) {
      let hook = fiber.memoizedState;
      for (let step = 0; hook && step < 60; step += 1) {
        const value = hook.memoizedState;
        if (
          value &&
          typeof value === "object" &&
          value.current &&
          typeof value.current.getSelection === "function" &&
          typeof value.current.write === "function"
        ) {
          return value.current;
        }
        hook = hook.next;
      }
      fiber = fiber.return;
    }
    return null;
  };

  const term = findTerminal();
  out.gotTerminal = Boolean(term);
  if (!term) return { ...out, verdict: "FAIL：fiber 走查拿不到 Terminal 实例" };

  try {
    out.codexSessionMouseMode = term.modes?.mouseTrackingMode ?? "unknown";
  } catch {
    out.codexSessionMouseMode = "unreadable";
  }

  /** 在回滚缓冲末尾挑一行有内容的文本。 */
  const pickRow = () => {
    const buffer = term.buffer.active;
    for (let index = buffer.length - 1; index >= Math.max(0, buffer.length - 40); index -= 1) {
      const line = buffer.getLine(index);
      const text = line ? line.translateToString(true) : "";
      if (text.trim().length >= 8) return { row: index, text };
    }
    return null;
  };

  // 等内容出现（恢复会话的重放是异步的）。
  for (let index = 0; index < 40 && !pickRow(); index += 1) {
    await p.sleep(500);
  }

  const screen = () => document.querySelector(".terminal-slot:not(.inactive) .xterm-screen");
  const point = () => {
    const rect = screen().getBoundingClientRect();
    return { x: Math.round(rect.left + rect.width / 2), y: Math.round(rect.top + rect.height / 2) };
  };

  /** 合成右键事件序列；返回每一步之后的选区状态。 */
  const burst = (x, y) => {
    const el = screen();
    const base = { bubbles: true, cancelable: true, clientX: x, clientY: y, detail: 1, view: window };
    el.dispatchEvent(new MouseEvent("mousedown", { ...base, button: 2, buttons: 2 }));
    const afterDown = term.hasSelection();
    el.dispatchEvent(new MouseEvent("mouseup", { ...base, button: 2, buttons: 0 }));
    const afterUp = term.hasSelection();
    el.dispatchEvent(new MouseEvent("contextmenu", { ...base, button: 2, buttons: 0 }));
    return { afterDown, afterUp };
  };

  const readMenu = async () => {
    await p.sleep(250);
    const menu = document.querySelector(".context-menu");
    if (!menu) return { open: false };
    const copy = [...menu.querySelectorAll("button")].find(
      (item) => (item.textContent || "").trim() === "复制"
    );
    return { open: true, copyDisabled: copy ? copy.disabled : null };
  };

  const dismissMenu = async () => {
    const backdrop = document.querySelector(".menu-backdrop");
    if (backdrop) backdrop.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    await p.sleep(150);
  };

  const runVariant = async (name, mouseModeOn) => {
    term.write(mouseModeOn ? "\u001b[?1000h" : "\u001b[?1000l");
    await p.sleep(150);
    const target = pickRow();
    if (!target) return { skip: "没有可选的文本行" };
    term.select(0, target.row, Math.min(12, target.text.trim().length));
    await p.sleep(100);
    const selected = term.getSelection() || "";
    const sentinel = `SENTINEL-${name}`;
    await p.invoke("clipboard:copy-text", { text: sentinel });

    const hasSelectionBefore = term.hasSelection();
    const { x, y } = point();
    const marks = burst(x, y);
    const menu = await readMenu();

    let clickedCopy = false;
    const copyButton = [...document.querySelectorAll(".context-menu button")].find(
      (item) => (item.textContent || "").trim() === "复制"
    );
    if (copyButton && !copyButton.disabled) {
      copyButton.click();
      clickedCopy = true;
    }
    await p.sleep(300);
    const clipboard = await p.invoke("clipboard:read-text", {});
    await dismissMenu();

    return {
      mouseMode: term.modes?.mouseTrackingMode ?? "unknown",
      selectedText: selected.slice(0, 30),
      hasSelectionBefore,
      hasSelectionAfterMousedown: marks.afterDown,
      hasSelectionAfterMouseup: marks.afterUp,
      hasSelectionAfterContextmenu: term.hasSelection(),
      menu,
      clickedCopy,
      copiedSelectionToClipboard: Boolean(selected) && clipboard === selected,
      clipboardStillSentinel: clipboard === sentinel
    };
  };

  // 剪贴板原文，跑完尽量还原。
  let clipboardOriginal = "";
  try {
    clipboardOriginal = (await p.invoke("clipboard:read-text", {})) || "";
  } catch {
    clipboardOriginal = "";
  }

  out.v1MouseOff = await runVariant("V1", false);
  out.v2MouseOn = await runVariant("V2", true);
  term.write("\u001b[?1000l");
  try {
    if (clipboardOriginal) await p.invoke("clipboard:copy-text", { text: clipboardOriginal });
  } catch {
    /* 还原失败无所谓，剪贴板不是本探针的断言对象。 */
  }

  const v1 = out.v1MouseOff;
  const v2 = out.v2MouseOn;
  const baselineOk = v1?.clickedCopy && v1?.copiedSelectionToClipboard;

  out.verdict = !baselineOk
    ? "INSTRUMENT：V1 正对照未成立（探针自身没走通复制），先查探针"
    : v2?.menu?.copyDisabled === true && v2?.clipboardStillSentinel === true
      ? "REPRO：V2（鼠标上报开启）右键即清选区、复制不可用"
      : "OK：V2 也正常（修复已生效或本形态不复现）";
  return JSON.stringify(out, null, 2);
})();
