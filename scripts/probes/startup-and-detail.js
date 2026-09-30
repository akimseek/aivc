/**
 * 第六批验收：启动默认、终端区动作条、详情可达性。
 *
 * # 查的是用户报的三件事
 *
 * 1. **启动后不选任何平台 / 目标 / 历史会话**——此前三处都会「还原不上就挑第一个」。
 * 2. **右侧那一排「详情 / 重命名 / 复制 / 导出 MD / 导出 JSON / 另存到…」是我自造的**：
 *    终端/会话区的详情入口最终固定在**标签栏**的「消息与分支」上，
 *    其余在右键菜单与设置中心里。
 * 3. **点历史会话进入终端后要能打开详情**（也就到得了「从此处分支」）——
 *    详情原来放在会话正文头部，进终端后正文不渲染，入口就消失了。
 *
 * # 自证
 *
 * - 第 1 条在**空运行根**上验：那里没有任何可还原的选择，因此「没选」是可以观察到的
 *   事实，而不是「恰好等于默认值」。同时确认列表里**确实有会话**——否则
 *   「没选中」可能只是因为没东西可选。
 * - 第 3 条先确认详情弹层**原本没打开**，再点，再确认它开了。
 */
(async () => {
  const p = window.__p;
  const out = { checks: [] };
  const check = (name, ok, detail) => out.checks.push({ name, ok: Boolean(ok), detail: detail ?? "" });
  const clean = (element) => p.clean(element.textContent ?? "");
  const buttons = (root = document) =>
    [...root.querySelectorAll("button")].map((item) => clean(item));

  /**
   * 标签栏动作区的「消息与分支」按钮（`features/workspace/tab-bar.tsx`）。
   *
   * **不能按文本精确匹配**：这颗按钮里除了 `<span>消息与分支</span>` 还挂着一颗条数
   * 徽章，选中会话后 `textContent` 是「消息与分支0」这种带数字的形式，精确相等永远
   * 匹配不到（`buttons()` 那套整页文本在它身上会给出假 FAIL）。按类名 + 前缀定位。
   */
  const messagesAction = () =>
    [...document.querySelectorAll(".tab-bar-action")].find((item) =>
      clean(item).startsWith("消息与分支")
    );

  try {
    await p.waitForTargets();
    await p.sleep(2000);

    /** 侧栏头部那两个下拉：第一个是平台，第二个是目标。 */
    const pickers = () => [...document.querySelectorAll(".sidebar .pickers select")];
    const setSelect = async (element, value) => {
      Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value").set.call(element, value);
      element.dispatchEvent(new Event("change", { bubbles: true }));
      await p.sleep(1200);
    };

    // ── 1. 启动默认：什么都不选 ──────────────────────────
    //
    // **不用下拉的 `.value` 判断平台**：`value=""` 时没有匹配的 `<option>`，
    // 浏览器会把 `selectedIndex` 落到第一项，`.value` 于是读出 `"codex"`——
    // 那是 DOM 的表现，不是应用的选择，照它断言会得到一个假的失败。
    //
    // 也不用工作台那几行「平台/目标/会话」：工作台在没有打开会话时渲染的是
    // **空状态**（「没有打开的会话。」），那几行根本不出现，信号不稳定。
    //
    // 用**可证明的推论**：平台真选了的话目标列表就会被拉回来（那张表按平台查）。
    // 因此「目标列表为空」就证明平台没选；这也正是用户看到的那句话。
    const initialText = clean(document.querySelector(".sidebar") ?? document.body);
    out.initial = {
      // 批次三：正文头部是 `.history-head`（`features/workspace/history-view.tsx`），
      // 主列表的行是 `.session-card`。
      sessionHead: Boolean(document.querySelector(".history-head")),
      rowCount: document.querySelectorAll(".sidebar .rows .session-card").length,
      targetSelectDisabled: pickers()[1]?.disabled ?? null,
      text: initialText.slice(0, 200)
    };
    check(
      "**启动后平台未选择**——因此目标列表是空的（侧栏显示「暂无可用目标」）",
      /暂无可用目标/.test(out.initial.text),
      out.initial.text
    );
    check(
      "**启动后目标未选择**（目标下拉被禁用，且侧栏提示「请先在上面选择目标。」）",
      out.initial.targetSelectDisabled === true && /请先在上面选择目标/.test(out.initial.text),
      `disabled=${out.initial.targetSelectDisabled}`
    );
    check(
      "**启动后没有会话被选中**（没有任何会话行，正文头部不存在）",
      out.initial.rowCount === 0 && !out.initial.sessionHead,
      `rows=${out.initial.rowCount} sessionHead=${out.initial.sessionHead}`
    );
    // ── 2. 用户自己选平台与目标 ──────────────────────────
    const [providerSelect, targetSelect] = pickers();
    if (!providerSelect || !targetSelect) {
      out.verdict = "FAIL：找不到侧栏的平台 / 目标下拉";
      return out;
    }
    const codexOption = [...providerSelect.options].find((item) => item.value === "codex");
    check("平台下拉里有 codex", Boolean(codexOption), JSON.stringify([...providerSelect.options].map((o) => o.value)));
    if (codexOption) await setSelect(providerSelect, "codex");

    const targetOption = [...targetSelect.options].find((item) => item.value !== "");
    out.targetOption = targetOption?.value ?? "";
    check("选了平台之后目标下拉里有可选项", Boolean(targetOption), out.targetOption);
    if (targetOption) await setSelect(targetSelect, targetOption.value);
    await p.sleep(2500);

    // ── 3. 列表有内容，但**仍然没有任何会话被自动选中** ───
    //
    // 这一条才是真正要验的：列表装进来了，而旧行为会把第一项选中。
    for (let index = 0; index < 20 && document.querySelectorAll(".sidebar .rows .session-card").length === 0; index += 1) {
      await p.sleep(500);
    }
    const messagesBefore = messagesAction();
    out.loaded = {
      rowCount: document.querySelectorAll(".sidebar .rows .session-card").length,
      sessionHead: Boolean(document.querySelector(".history-head")),
      rowActive: document.querySelectorAll(".sidebar .rows .session-card.on").length,
      detailBefore: Boolean(messagesBefore),
      detailBeforeDisabled: messagesBefore?.disabled ?? null
    };
    check(
      "选了目标之后会话列表**确实装进了内容**（否则下面的断言没有意义）",
      out.loaded.rowCount > 0,
      `rows=${out.loaded.rowCount}`
    );
    check(
      "**列表装了内容但仍未自动选中第一条**（此前 `fresh[0]` 会选它）",
      !out.loaded.sessionHead && out.loaded.rowActive === 0,
      JSON.stringify(out.loaded)
    );
    check(
      "没有选中任何会话时「消息与分支」**在但被禁用**（新 UI 用禁用表达「没有可查看的对象」，按钮不再消失）",
      out.loaded.detailBefore && out.loaded.detailBeforeDisabled === true,
      `present=${out.loaded.detailBefore} disabled=${out.loaded.detailBeforeDisabled}`
    );

    // ── 4. 那个动作条不该再出现自造的按钮 ────────────────
    const invented = ["重命名", "复制", "导出 MD", "导出 JSON", "另存到…"];
    out.inventedPresent = invented.filter((text) => buttons().includes(text));
    check(
      "**终端/会话区不再有那五个自造按钮**（它们在右键菜单与菜单栏里）",
      out.inventedPresent.length === 0,
      JSON.stringify(out.inventedPresent)
    );

    // ── 2. 双击一条历史会话 → 恢复并进入终端 ──────────────
    //
    // 批次一改了手势：**单击**是打开历史正文（`onOpen`），**双击**才是恢复终端
    // （`onResume`，`features/sessions/session-card.tsx`）。单击在这里什么终端都起不来。
    const row = document.querySelector(".sidebar .rows .session-card");
    if (!row) {
      out.verdict = "FAIL：没有可点的会话行，无法验证第 2、3 条";
      return out;
    }
    row.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    // 终端要起来（xterm 挂上）。
    for (let index = 0; index < 40 && !document.querySelector(".xterm"); index += 1) {
      await p.sleep(500);
    }
    await p.sleep(2500);
    const messagesAfter = messagesAction();
    out.afterOpen = {
      xterm: Boolean(document.querySelector(".xterm")),
      sessionHead: Boolean(document.querySelector(".history-head")),
      detail: Boolean(messagesAfter),
      detailDisabled: messagesAfter?.disabled ?? null
    };
    check(
      "双击会话行进入了终端（正文不再渲染，`history-head` 消失）",
      out.afterOpen.xterm && !out.afterOpen.sessionHead,
      JSON.stringify(out.afterOpen)
    );
    check(
      "**进入终端后仍然有「消息与分支」按钮，且可用**（此前详情入口随正文一起消失，于是到不了「从此处分支」）",
      out.afterOpen.detail && out.afterOpen.detailDisabled === false,
      JSON.stringify(out.afterOpen)
    );

    // ── 3. 点「消息与分支」→ 弹层打开 ────────────────────
    //
    // 类名是 `.messages-panel`（`features/workspace/messages-modal.tsx`）——
    // 旧的 `.session-detail-dialog` / `.session-detail-modal` 都不存在了。
    const dialog = () => document.querySelector(".messages-panel");
    check(
      "点之前详情弹层**没有**开着（否则下面「开了」不算数）",
      !dialog(),
      String(Boolean(dialog()))
    );
    const action = messagesAction();
    const clicked = Boolean(action && !action.disabled);
    if (clicked) action.click();
    check(
      "点到了「消息与分支」按钮（按钮存在且可用）",
      clicked,
      `present=${Boolean(action)} disabled=${action?.disabled ?? null}`
    );
    let opened = false;
    for (let index = 0; index < 30; index += 1) {
      await p.sleep(400);
      if (dialog()) {
        opened = true;
        break;
      }
    }
    out.detail = { opened, text: clean(dialog() ?? document.body).slice(0, 200) };
    check("**点「消息与分支」打开了会话详情弹层**", opened, out.detail.text);
    // 弹层要有实际内容，不能是个空壳。
    check(
      "详情弹层里渲染出了会话内容（不是空壳）",
      opened && /会话详情|消息/.test(out.detail.text),
      out.detail.text
    );
  } catch (error) {
    out.threw = String((error && error.stack) || error).slice(0, 700);
    check("探针自身没有抛异常", false, out.threw);
  }

  const failed = out.checks.filter((item) => !item.ok);
  out.verdict =
    failed.length === 0
      ? `PASS：${out.checks.length} 项启动默认与详情可达性全部通过`
      : `FAIL：${failed.length}/${out.checks.length} 未通过 —— ${failed.map((f) => f.name).join("、")}`;
  return out;
})();
