/**
 * 第六批验收：启动默认、终端区动作条、详情可达性。
 *
 * # 查的是用户报的三件事
 *
 * 1. **启动后不选任何平台 / 目标 / 历史会话**——此前三处都会「还原不上就挑第一个」。
 * 2. **右侧那一排「详情 / 重命名 / 复制 / 导出 MD / 导出 JSON / 另存到…」是我自造的**：
 *    旧平台的终端工作区头部**只有「详情」与「新会话」**，其余在右键菜单与菜单栏里。
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
  const clickText = (text, root = document) => {
    const found = [...root.querySelectorAll("button")].find((item) => clean(item) === text);
    if (found) found.click();
    return Boolean(found);
  };

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
      sessionHead: Boolean(document.querySelector(".session-head")),
      rowCount: document.querySelectorAll(".sidebar .rows .row").length,
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
    for (let index = 0; index < 20 && document.querySelectorAll(".sidebar .rows .row").length === 0; index += 1) {
      await p.sleep(500);
    }
    out.loaded = {
      rowCount: document.querySelectorAll(".sidebar .rows .row").length,
      sessionHead: Boolean(document.querySelector(".session-head")),
      rowActive: document.querySelectorAll(".sidebar .rows .row.on").length,
      detailBefore: buttons().includes("详情")
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
      "没有选中任何会话时**不显示「详情」**（没有可查看的对象）",
      !out.loaded.detailBefore,
      `detail=${out.loaded.detailBefore}`
    );

    // ── 4. 那个动作条不该再出现自造的按钮 ────────────────
    const invented = ["重命名", "复制", "导出 MD", "导出 JSON", "另存到…"];
    out.inventedPresent = invented.filter((text) => buttons().includes(text));
    check(
      "**终端/会话区不再有那五个自造按钮**（它们在右键菜单与菜单栏里）",
      out.inventedPresent.length === 0,
      JSON.stringify(out.inventedPresent)
    );

    // ── 2. 点一条历史会话 → 进入终端 ─────────────────────
    //
    // 侧栏里点会话行就是「恢复它并进入终端」（`onOpen={onResume}`），
    // 也就是用户描述的那条路径。
    const row = document.querySelector(".sidebar .rows .row");
    if (!row) {
      out.verdict = "FAIL：没有可点的会话行，无法验证第 2、3 条";
      return out;
    }
    row.click();
    // 终端要起来（xterm 挂上）。
    for (let index = 0; index < 40 && !document.querySelector(".xterm"); index += 1) {
      await p.sleep(500);
    }
    await p.sleep(2500);
    out.afterOpen = {
      xterm: Boolean(document.querySelector(".xterm")),
      sessionHead: Boolean(document.querySelector(".session-head")),
      detail: buttons().includes("详情")
    };
    check(
      "点会话行进入了终端（正文不再渲染，`session-head` 消失）",
      out.afterOpen.xterm && !out.afterOpen.sessionHead,
      JSON.stringify(out.afterOpen)
    );
    check(
      "**进入终端后仍然有「详情」按钮**（此前它随正文一起消失，于是到不了「从此处分支」）",
      out.afterOpen.detail,
      JSON.stringify(out.afterOpen)
    );

    // ── 3. 点「详情」→ 弹层打开 ──────────────────────────
    //
    // 类名是 `.session-detail-dialog`（`Dialog` 的 `panelClassName`），
    // 不是 `.session-detail-modal`——后者是**旧平台**的叫法，照它找会永远找不到。
    const dialog = () => document.querySelector(".session-detail-dialog");
    check(
      "点之前详情弹层**没有**开着（否则下面「开了」不算数）",
      !dialog(),
      String(Boolean(dialog()))
    );
    const clicked = clickText("详情");
    check("点到了「详情」按钮", clicked, String(clicked));
    let opened = false;
    for (let index = 0; index < 30; index += 1) {
      await p.sleep(400);
      if (dialog()) {
        opened = true;
        break;
      }
    }
    out.detail = { opened, text: clean(dialog() ?? document.body).slice(0, 200) };
    check("**点「详情」打开了会话详情弹层**", opened, out.detail.text);
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
