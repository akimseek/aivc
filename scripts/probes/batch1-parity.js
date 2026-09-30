/**
 * 第一批量验收：A3 费率 / A5 高级 JSON / A4 文案。
 *
 * # 为什么必须实测
 *
 * 这一批全是「界面读的字段必须与后端发的一致」：
 * - 费率：后端 `VendorPricing { inputPerMillionUsd, outputPerMillionUsd }`；
 * - 高级 JSON：后端要的是**结构化对象**，界面递的是解析后的对象；
 * - A4：那句文案说的是不是真的，只有对着后端的实际行为才知道。
 *
 * 因此判据是**写进去、从后端读回来、再对一遍**，而不是只看界面显示什么。
 *
 * （原先还有 A7「终端类型」一段：那时它验的是系统终端的类型选择。该功能已整体
 * 移除，那一段随之删掉——不是放宽断言。）
 */
(async () => {
  const p = window.__p;
  const out = { checks: [] };
  const check = (name, ok, detail) => out.checks.push({ name, ok: Boolean(ok), detail: detail ?? "" });
  const clean = (element) => p.clean(element.textContent);

  /**
   * 打开配置覆盖层的某一页。
   *
   * 菜单里那一项叫 **「供应商管理」**，不叫「供应商」——第一版探针按后者找，
   * 找不到就静默跳过，于是后面基于「面板文本」的断言**全部空过**。
   * 因此这里返回是否真的打开了，调用方必须断言它。
   */
  const openConfigTab = async (navLabel) => {
    if (document.querySelector(".settings-content")) return true;
    // 入口是活动栏底部的设置按钮（批次二移除了菜单栏）。
    const settings = document.querySelector(".activity-bottom button");
    if (!settings) return false;
    settings.click();
    await p.sleep(500);
    const item = [...document.querySelectorAll(".settings-nav-item")].find(
      (sub) => clean(sub) === navLabel
    );
    if (!item) return false;
    item.click();
    for (let index = 0; index < 10 && !document.querySelector(".settings-content"); index += 1) {
      await p.sleep(400);
    }
    return Boolean(document.querySelector(".settings-content"));
  };

  // 整段包一层：探针自己抛异常只会留下一句「求值结果抛异常」，
  // 分不清是探针的毛病还是产品的。兜住并把栈带回来。
  try {
    await p.waitForTargets();

  // ── 先清理上次跑剩下的同名供应商 ────────────────────────
  //
  // 后端对 `(providerId, 名称)` 有唯一约束。探针中途失败会留下一条，
  // 下一次跑就会撞「供应商名称已存在」——那是探针自己的垃圾，不是产品问题。
  const PROBE_NAME = "AIVC-PROBE-费率";
  for (const leftover of await p.invoke("vendor:list", {}) ?? []) {
    if (leftover.name === PROBE_NAME) await p.invoke("vendor:delete", { id: leftover.id });
  }

  // ── A3 + A5：写进后端再读回来 ───────────────────────────
  const payload = {
    providerId: "codex",
    name: PROBE_NAME,
    apiKey: "sk-probe",
    apiBaseUrl: "https://probe.example.com/v1",
    sort: 997,
    enabled: false,
    pricing: { inputPerMillionUsd: 0.55, outputPerMillionUsd: 2.2 },
    balanceQuery: { endpoint: "/user/balance", method: "GET" },
    modelQuery: { endpoint: "/models" }
  };
  const saved = await p.invoke("vendor:save", payload);
  out.saved = saved;
  check("带费率与高级 JSON 的供应商能存进去", Boolean(saved?.id), JSON.stringify(saved ?? {}).slice(0, 160));
  check(
    "**费率按结构存住了**（inputPerMillionUsd / outputPerMillionUsd）",
    saved?.pricing?.inputPerMillionUsd === 0.55 && saved?.pricing?.outputPerMillionUsd === 2.2,
    JSON.stringify(saved?.pricing)
  );
  check(
    "**余额查询配置存住了**",
    saved?.balanceQuery?.endpoint === "/user/balance",
    JSON.stringify(saved?.balanceQuery)
  );
  check(
    "**模型查询配置存住了**",
    saved?.modelQuery?.endpoint === "/models",
    JSON.stringify(saved?.modelQuery)
  );

  // 从列表里再读一次：确认不是「返回值拼出来的、其实没落库」。
  const listed = await p.invoke("vendor:list", {});
  const found = (listed ?? []).find((item) => item.id === saved?.id);
  out.listed = found;
  check(
    "**从列表读回来费率还在**（不是返回值拼出来的）",
    found?.pricing?.inputPerMillionUsd === 0.55,
    JSON.stringify(found?.pricing)
  );
  check(
    "从列表读回来高级 JSON 还在",
    found?.balanceQuery?.endpoint === "/user/balance",
    JSON.stringify(found?.balanceQuery)
  );

  // 更新的语义：**省略即清空**（PUT 语义），而不是「沿用旧值」。
  //
  // 这里与 `apiKey` **不对称**：密钥留空表示沿用旧值（后端文档里写明），
  // 而 `pricing` / 高级 JSON 省略就是清掉。实测确认。
  //
  // 我的界面之所以不受影响，是因为**编辑时会把值回填进表单**，
  // 于是提交时一定带着它们。真正要保证的是那条路径，下面单独验。
  await p.invoke("vendor:save", {
    id: saved.id,
    providerId: "codex",
    name: "AIVC-PROBE-费率",
    apiKey: "",
    apiBaseUrl: "https://probe.example.com/v1",
    sort: 997,
    enabled: false
  });
  const afterUpdate = (await p.invoke("vendor:list", {}) ?? []).find(
    (item) => item.id === saved?.id
  );
  out.afterUpdate = { pricing: afterUpdate?.pricing, balanceQuery: afterUpdate?.balanceQuery };
  check(
    "省略 pricing / 高级 JSON 时后端会清空（**PUT 语义**，与 apiKey 的「留空沿用」不对称）",
    afterUpdate?.pricing === undefined || afterUpdate?.balanceQuery === undefined,
    JSON.stringify(out.afterUpdate)
  );

  // 重新写回一份，供下面验「界面编辑路径不会清掉」。
  //
  // **必须带上 `id`**：不带就是新建，而 `(平台, 名称)` 有唯一约束，
  // 会撞「供应商名称已存在」——第一版探针就是这么把自己的流程打断的。
  await p.invoke("vendor:save", { ...payload, id: saved.id });

  // ── 界面编辑路径：值要回填进表单 ────────────────────────
  //
  // 这才是用户真正依赖的性质：点「编辑」→ 直接「保存修改」，
  // 费率与高级 JSON **不能丢**。
  const vendorPanelOpened = await openConfigTab("供应商管理");
  check("能打开供应商管理页（打不开的话下面两条会空过）", vendorPanelOpened, String(vendorPanelOpened));
  await p.sleep(1200);

  const row = [...document.querySelectorAll(".settings-content tr")].find((item) =>
    clean(item).includes(PROBE_NAME)
  );
  out.foundProbeRow = Boolean(row);
  check("探针造的那条供应商出现在列表里", Boolean(row), String(Boolean(row)));
  const editButton = row
    ? [...row.querySelectorAll("button")].find((item) => clean(item) === "编辑")
    : undefined;
  if (editButton) {
    editButton.click();
    await p.sleep(700);
  }
  const overlay = document.querySelector(".settings-content");
  const priceInputs = overlay
    ? [...overlay.querySelectorAll("input")].map((item) => item.value).filter((v) => v !== "")
    : [];
  const textareas = overlay ? [...overlay.querySelectorAll("textarea")].map((item) => item.value) : [];
  out.editForm = { priceInputs, textareas: textareas.map((v) => v.slice(0, 40)) };
  check(
    "**点「编辑」时费率回填进表单**（不回填就会在「保存修改」时被清掉）",
    priceInputs.includes("0.55") && priceInputs.includes("2.2"),
    JSON.stringify(priceInputs)
  );
  check(
    "点「编辑」时高级 JSON 回填进表单",
    textareas.some((value) => value.includes("/user/balance")),
    JSON.stringify(out.editForm.textareas)
  );

  // 关掉覆盖层，后面的终端那几条要干净的界面。
  // 设置中心的关闭是**图标按钮**（没有文字），只能按无障碍名找。
  const closeOverlay = [...document.querySelectorAll(".settings-head button")].find(
    (item) => (item.getAttribute("aria-label") || "").trim() === "关闭"
  );
  if (closeOverlay) {
    closeOverlay.click();
    await p.sleep(600);
  }

  // 清理：探针造的数据不要留在用户的库里。
  const deleted = await p.invoke("vendor:delete", { id: saved.id });
  check("探针造的那条供应商已删掉（不污染真实数据）", deleted === true, String(deleted));

  /*
    A7「终端类型」那一段删掉了。

   它验的是系统终端的类型下拉（默认 / PowerShell / CMD / Git Bash / WSL）与
   后端 `ShellKind::parse` 的九个取值——**整个系统终端功能已按用户要求移除**，
   连同那个下拉、`ShellKind`、「+ Shell」入口与 `terminal:start-system` 一起。

   因此这里不再有可验的东西：不是把断言放宽，而是功能不存在了。
  */


  // ── A4：那句文案 ────────────────────────────────────────
  const stillOpen = await openConfigTab("供应商管理");
  await p.sleep(600);
  const overlayText = clean(document.querySelector(".settings-content") ?? document.body);
  out.overlayText = overlayText.slice(0, 300);
  // **先正向确认真的在供应商页上**：不然下面那条否定断言会空过
  // （面板没打开时「找不到那句话」当然成立）。这是本轮第三次抓到自己写的空过断言。
  //
  // 判据用「新增供应商」这个**页面上常驻的按钮**，不用「模型厂商」——
  // 后者只在编辑表单打开时才存在，而这一步表单已经关了（第一版就是这么误判的）。
  check(
    "确认当前在供应商管理页上（否则下面的否定断言会空过）",
    stillOpen && overlayText.includes("新增供应商"),
    `open=${stillOpen} :: ${overlayText.slice(0, 140)}`
  );
  check(
    "**「写入 CLI 配置前会自动备份」这句做不到的承诺已经不在了**",
    !overlayText.includes("自动备份到"),
    overlayText.slice(0, 220)
  );

  } catch (error) {
    out.threw = String((error && error.stack) || error).slice(0, 700);
    check("探针自身没有抛异常", false, out.threw);
  }

  const failed = out.checks.filter((item) => !item.ok);
  out.verdict =
    failed.length === 0
      ? `PASS：${out.checks.length} 项第一批量验收全部通过`
      : `FAIL：${failed.length}/${out.checks.length} 未通过 —— ${failed.map((f) => f.name).join("、")}`;
  return out;
})();
