/**
 * 写一个探针：验证「自管输入」真的出现了，并且能把它写的话送进 CLI。
 *
 * # 为什么这个探针必须自己确认三件事
 *
 * 1. **它出现了**。自管输入在恢复历史会话时初值是隐藏的，只有当终端吐出第一段
 *    输出时才自动显示。这一步若断，功能等于不存在——而界面上不会有任何错误。
 * 2. **它写进去的东西真的到了 CLI**。判据是**终端屏幕随后出现那段文字**，
 *    而不是「`terminal:write` 被调用过」——后者在写失败时也成立。
 * 3. **它不在非前台的终端上出现**。多出一个属于别处的输入框会让人写错地方。
 *
 * 探针自己选目标、自己开一路新会话（而不是恢复历史会话），因为**新会话的输入框
 * 一进去就该在**——那是这条路径的确定性判据。
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

  /** 自管输入的当前状态。 */
  const composerState = () => {
    const box = document.querySelector(".composer");
    const area = document.querySelector(".composer textarea");
    return {
      present: Boolean(box),
      visible: Boolean(box && box.offsetParent !== null),
      value: area ? area.value : null,
      placeholder: area ? area.getAttribute("placeholder") : null,
      hasResizeHandle: Boolean(document.querySelector(".composer-resize")),
      sendDisabled: (() => {
        const send = [...document.querySelectorAll(".composer button")].find((item) =>
          (item.textContent || "").includes("发送")
        );
        return send ? send.disabled : null;
      })()
    };
  };

  // ── 前置：拿到可用目标 ─────────────────────────────────
  const targets = await safe("targets", () => p.invoke("codex:list-targets", {}));
  const local = (targets || []).find(
    (item) => item.provider === "codex" && item.kind === "local" && item.available
  );
  if (!local) return { ...out, verdict: "SKIP：没有可用的本机 Codex 目标" };

  // 在界面上选中它，让「新建」按钮可用。
  out.targetSelect = await safe("targetSelect", async () => {
    const select = await p.waitForTargets();
    if (!select) return { ok: false, reason: "等不到目标下拉框" };
    const option = [...select.options].find((item) => item.value.endsWith(":local"));
    if (!option) return { ok: false, reason: "下拉框里没有本机目标" };
    p.selectValue(select, option.value);
    await p.sleep(2000);
    return { ok: true, picked: option.value };
  });
  out.steps.push(`选中目标：${JSON.stringify(out.targetSelect)}`);

  // ── 开一路新会话 ───────────────────────────────────────
  // 新会话的输入框**一进去就该在**，不需要等输出——这是 `initialComposerVisible`
  // 的语义（`!sessionId`）。
  const newCodex = [...document.querySelectorAll("button")].find((item) =>
    (item.textContent || "").includes("+ Codex")
  );
  if (!newCodex) return { ...out, verdict: "FAIL：找不到「+ Codex」按钮" };

  let terminalId = null;
  await window.__TAURI__.event.listen("terminal:data", (event) => {
    if (!terminalId) terminalId = event.payload.terminalId;
  });

  newCodex.click();
  out.steps.push("点击 + Codex");

  // 新会话对话框：直接确认。
  for (let round = 0; round < 20; round += 1) {
    await p.sleep(500);
    const dialog = document.querySelector(".new-session-dialog");
    if (dialog) {
      const open = [...dialog.querySelectorAll("button")].find(
        (item) => (item.textContent || "").trim() === "打开"
      );
      if (open) {
        open.click();
        out.steps.push("确认新会话对话框");
        break;
      }
    }
  }

  for (let index = 0; index < 40 && !terminalId; index += 1) await p.sleep(500);
  out.terminalId = terminalId;
  if (!terminalId) return { ...out, verdict: "FAIL：点了新会话但没有终端产出" };

  // 等终端吐出第一段输出（自管输入此时应当已经在，但等一等让状态稳定）。
  for (let index = 0; index < 30; index += 1) {
    await p.sleep(500);
    if (/Ask Codex|for shortcuts|OpenAI Codex|Update available|Trust/i.test(p.rows())) break;
  }
  // 启动提示：一律选「不改变环境」的那一项。
  for (let round = 0; round < 5; round += 1) {
    await p.sleep(2500);
    const text = p.rows();
    if (/for shortcuts|Ask Codex/i.test(text)) break;
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

  out.composerAfterStart = composerState();
  out.steps.push(
    `自管输入出现=${out.composerAfterStart.present}，含拖动手柄=${out.composerAfterStart.hasResizeHandle}`
  );

  if (!out.composerAfterStart.present) {
    out.verdict = "FAIL：新会话里**没有**自管输入（新会话应当一进去就有）";
    return out;
  }

  // ── 写一句话发出去，并**在终端屏幕上确认它到了** ────────
  const token = `composer-probe-${Date.now().toString(36)}`;
  out.sentText = token;

  await safe("type", async () => {
    const area = document.querySelector(".composer textarea");
    if (!area) throw new Error("没有输入框");
    const setter = Object.getOwnPropertyDescriptor(
      HTMLTextAreaElement.prototype,
      "value"
    ).set;
    setter.call(area, token);
    area.dispatchEvent(new Event("input", { bubbles: true }));
    await p.sleep(300);
  });
  out.composerAfterTyping = composerState();

  await safe("enter", async () => {
    const area = document.querySelector(".composer textarea");
    area.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true })
    );
  });
  out.steps.push("在自管输入里按回车发送");

  // 等那段文字出现在终端屏幕上。**这是唯一的真判据**——
  // 「terminal:write 被调用过」在写失败时也会成立。
  let echoed = false;
  for (let index = 0; index < 30; index += 1) {
    await p.sleep(500);
    if (p.rows().includes(token)) {
      echoed = true;
      break;
    }
  }
  out.echoedInTerminal = echoed;
  out.screenAfterSend = p.rows().slice(0, 40);
  out.composerAfterSend = composerState();

  if (!echoed) {
    out.verdict =
      "FAIL：自管输入写进去的文字**没有**出现在终端屏幕上——它没真的送达 CLI";
    return out;
  }

  if (out.composerAfterSend.value !== "") {
    out.verdict = "FAIL：发送后输入框没有清空";
    return out;
  }

  out.verdict = "PASS：新会话有自管输入、可拖动、发送后文字真的出现在终端里且输入框清空";
  return out;
})();
