/**
 * 操作结果反馈：做完一件事，界面上真的出现那句话吗？
 *
 * # 为什么必须实测
 *
 * 单测只能证明「`onNotify` 被调了」。而从 `notify` 到**用户看得见的那条提示**
 * 隔着三层：`useConsole` / `ConfigOverlay` 的属性传递、`useToast` 的计时器、
 * 以及 `AppOverlays` 里那个条件渲染。任何一层接错，单测照样全绿而界面上什么都没有。
 *
 * 因此这里的判据是：**触发一个真实动作，然后在 DOM 里找那句话**。
 *
 * # 自证
 *
 * 每条都先确认提示**原本不在**，再触发动作，再确认它出现了。
 */
(async () => {
  const p = window.__p;
  const out = { checks: [] };
  const check = (name, ok, detail) => out.checks.push({ name, ok: Boolean(ok), detail: detail ?? "" });

  /** 当前那条自动消失的提示（`.export-hint` 就是它的容器）。 */
  const hintText = () => {
    const element = document.querySelector(".export-hint");
    return element ? p.clean(element.textContent) : "";
  };
  const clean = (element) => p.clean(element.textContent);

  /**
   * 等到提示里出现 `keyword`。
   *
   * 提示 4 秒后自动消失，因此必须在它还在的时候读——这也顺带证明了「会自动消失」
   * 这条性质没有被破坏（下面单独验）。
   */
  const waitForHint = async (keyword, tries = 16) => {
    for (let index = 0; index < tries; index += 1) {
      if (hintText().includes(keyword)) return true;
      await p.sleep(250);
    }
    return false;
  };

  try {
    await p.waitForTargets();
    await p.sleep(1000);

    /*
      **先自己选平台与目标。**
      
      本应用现在**启动后什么都不选**（这是刻意的：此前三处「还原不上就挑第一个」，
      于是界面替你选好了平台/目标/会话）。因此列表一开始是空的——
      这个探针原来假设「侧栏本来就有会话行」，那正是被去掉的旧行为。
      
      这里走用户真实路径：在下拉里选平台，再选目标，然后才有会话可操作。
    */
    const pickers = () => [...document.querySelectorAll(".sidebar .pickers select")];
    const setSelect = async (element, value) => {
      Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value").set.call(element, value);
      element.dispatchEvent(new Event("change", { bubbles: true }));
      await p.sleep(1300);
    };
    const [providerSelect, targetSelect] = pickers();
    const codex = [...(providerSelect?.options ?? [])].find((item) => item.value === "codex");
    if (codex) await setSelect(providerSelect, "codex");
    const target = [...(targetSelect?.options ?? [])].find((item) => item.value !== "");
    if (target) await setSelect(targetSelect, target.value);
    await p.sleep(2000);

    // ── 找一个真实会话来操作 ─────────────────────────────
    //
    // 收藏是**最轻**的动作：不改磁盘上的东西（只是元数据），失败了也能复原，
    // 而且列表里本来就有会话。用它来验「动作 → 提示」这条链。
    //
    // 类名是 `.rows > .row`（`components/list.tsx`），不是 `.list-row`——
    // 第一版按后者找，一行都没有，看起来像「没有会话」。
    let row = null;
    for (let index = 0; index < 20 && !row; index += 1) {
      row = document.querySelector(".sidebar .rows .row");
      if (!row) await p.sleep(500);
    }
    out.hasRow = Boolean(row);
    check("侧栏里有会话行（没有的话下面没法验）", Boolean(row), String(Boolean(row)));
    if (!row) {
      out.verdict = "FAIL：没有可操作的会话行，无法验证";
      return out;
    }
    // 先选中它，让「当前会话」有值。
    row.click();
    await p.sleep(1200);

    const star = row.querySelector(".star");
    out.hasStar = Boolean(star);

    // 记下点之前提示里是什么（应当是空的）。
    out.hintBefore = hintText();
    check("动作之前没有残留提示（否则下面的「出现了」不算数）", out.hintBefore === "", out.hintBefore);

    if (star) {
      star.click();
      const starred = await waitForHint("收藏");
      out.hintAfterStar = hintText();
      check(
        "点收藏后出现一句话（**从 notify 到界面上可见**这条链接通了）",
        starred,
        out.hintAfterStar
      );
      check(
        "那句话说的是**收藏**这件事",
        out.hintAfterStar.includes("收藏"),
        out.hintAfterStar
      );
    }

    // ── 提示会自动消失 ───────────────────────────────────
    //
    // 这一条是 `useToast` 的性质，但它决定了「常驻条幅会不会一直压着正文」。
    // 上面那条刚把提示打出来，这里等它自己走掉。
    let gone = false;
    for (let index = 0; index < 40; index += 1) {
      await p.sleep(400);
      if (hintText() === "") {
        gone = true;
        break;
      }
    }
    check("提示几秒后自动消失（不会一直压着正文）", gone, `残留：${hintText()}`);

    // ── 供应商面板：保存 / 删除也要有回音 ─────────────────
    //
    // 这一条走的是**另一条通道**（`ConfigOverlay` 的 onNotify → 面板），
    // 与上面 useConsole 那条不是同一条代码路径。
    const toolbox = [...document.querySelectorAll("button")].find(
      (item) => clean(item) === "工具箱"
    );
    if (toolbox) {
      toolbox.click();
      await p.sleep(500);
      const item = [...document.querySelectorAll("button")].find(
        (sub) => clean(sub) === "供应商管理"
      );
      if (item) item.click();
      for (let index = 0; index < 10 && !document.querySelector(".overlay-body"); index += 1) {
        await p.sleep(400);
      }
    }
    await p.sleep(1200);

    const overlayOpened = Boolean(document.querySelector(".overlay-body"));
    check("能打开供应商管理页", overlayOpened, String(overlayOpened));

    // 造一条供应商并删掉它——两个动作各应有一句话。
    const PROBE = "AIVC-PROBE-反馈";
    const saved = await p.invoke("vendor:save", {
      providerId: "codex",
      name: PROBE,
      apiKey: "sk-probe",
      apiBaseUrl: "https://probe.example.com/v1",
      sort: 996,
      enabled: false
    });
    // 让面板重新列一次，才能点到「删除」。
    const reload = [...document.querySelectorAll(".overlay-body button")].find(
      (item) => clean(item) === "刷新"
    );
    if (reload) reload.click();
    await p.sleep(900);
    // 供应商列表没有「刷新」，靠切页签重取。
    const gatewayTab = [...document.querySelectorAll(".overlay-tabs .chip")].find(
      (item) => clean(item) === "网关"
    );
    const vendorTab = [...document.querySelectorAll(".overlay-tabs .chip")].find(
      (item) => clean(item) === "供应商"
    );
    if (gatewayTab && vendorTab) {
      gatewayTab.click();
      await p.sleep(600);
      vendorTab.click();
      await p.sleep(1400);
    }

    const probeRow = [...document.querySelectorAll(".overlay-body tr")].find((item) =>
      clean(item).includes(PROBE)
    );
    out.probeRowFound = Boolean(probeRow);
    check("探针造的供应商出现在列表里", Boolean(probeRow), String(Boolean(probeRow)));

    const deleteButton = probeRow
      ? [...probeRow.querySelectorAll("button")].find((item) => clean(item) === "删除")
      : undefined;
    if (deleteButton) {
      deleteButton.click();
      const deleted = await waitForHint("已删除");
      out.hintAfterDelete = hintText();
      check(
        "点删除后出现一句话（**面板那条通道**也通了）",
        deleted,
        out.hintAfterDelete
      );
      check("那句话点名了是哪一家", out.hintAfterDelete.includes(PROBE), out.hintAfterDelete);
    } else {
      check("探针那行有「删除」按钮", false, "没找到删除按钮");
    }

    // 清理（上面那条删除可能没成功）。
    for (const leftover of await p.invoke("vendor:list", {}) ?? []) {
      if (leftover.name === PROBE) await p.invoke("vendor:delete", { id: leftover.id });
    }
  } catch (error) {
    out.threw = String((error && error.stack) || error).slice(0, 600);
    check("探针自身没有抛异常", false, out.threw);
  }

  const failed = out.checks.filter((item) => !item.ok);
  out.verdict =
    failed.length === 0
      ? `PASS：${out.checks.length} 项操作反馈全部通过`
      : `FAIL：${failed.length}/${out.checks.length} 未通过 —— ${failed.map((f) => f.name).join("、")}`;
  return out;
})();
