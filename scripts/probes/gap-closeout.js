/**
 * 补齐的 3 条：6.1 CLI 安装页 / 4.6 全局检索按平台筛选 / 5.5 诊断页。
 *
 * # 为什么必须实测
 *
 * 这三条的共同点都是**界面读的字段必须与后端发的一致**。6.1 上一轮就是这么错的：
 * 后端发 20 个字段、前端类型只声明了 10 个，把 `nvmVersion` / `minimumNodeMajor`
 * 裁掉了——单元测试用自己编的假数据全绿，真机上那几行根本不存在。
 *
 * 因此这里的判据是**拿后端真实返回的字段名与值去对界面上的字**。
 */
(async () => {
  const p = window.__p;
  const out = { checks: [] };
  const check = (name, ok, detail) => out.checks.push({ name, ok: Boolean(ok), detail: detail ?? "" });

  const clean = (selector) => p.clean((document.querySelector(selector) || {}).textContent);
  const openTab = async (label) => {
    if (!document.querySelector(".settings-panel")) {
      // 入口是活动栏底部的设置按钮（批次二移除了菜单栏）。
      const settings = document.querySelector(".activity-bottom button");
      if (settings) settings.click();
      await p.sleep(2200);
    }
    const chip = [...document.querySelectorAll(".settings-nav-item")].find(
      (item) => clean0(item) === label
    );
    if (!chip) return false;
    chip.click();
    await p.sleep(2500);
    return true;
  };
  const clean0 = (element) => p.clean(element.textContent);

  await p.waitForTargets();

  /*
    **先自己选平台与目标。**
    
    本应用现在**启动后什么都不选**（刻意如此：此前三处「还原不上就挑第一个」）。
    环境页、诊断页的能力矩阵与路径检查都依赖「当前平台 / 当前目标」，
    没有它们这些块渲染不出来——这个探针原来吃的是旧行为的默认选中。
    
    这里走用户真实路径：下拉里选平台，再选目标。
  */
  {
    const pickers = [...document.querySelectorAll(".sidebar .pickers select")];
    const setSelect = async (element, value) => {
      Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value").set.call(element, value);
      element.dispatchEvent(new Event("change", { bubbles: true }));
      await p.sleep(1300);
    };
    const codex = [...(pickers[0]?.options ?? [])].find((item) => item.value === "codex");
    if (codex) await setSelect(pickers[0], "codex");
    const target = [...(pickers[1]?.options ?? [])].find((item) => item.value !== "");
    if (target) await setSelect(pickers[1], target.value);
    await p.sleep(2000);
  }

  // ── 6.1 CLI 环境页 ──────────────────────────────────────
  out.openedEnvironment = await openTab("运行时环境");
  const envText = clean(".settings-content");
  out.envText = envText.slice(0, 400);

  // 拿后端真实返回，用它来断言界面上的字。
  const providers = await p.invoke("ai:list-providers", {});
  const providerId = providers?.[0]?.id ?? "codex";
  const targets = await p.invoke("codex:list-targets", {});
  const local = (targets ?? []).find((item) => item.kind === "local") ?? targets?.[0];
  const env = await p.invoke("cli:check-environment", {
    providerId,
    ...(local ? { targetId: local.id } : {})
  });
  out.env = env;

  check(
    "环境页显示五个检查项（官方 Node / Node / npm / nvm / CLI）",
    ["官方 Node", "Node", "npm", "nvm", "CLI"].every((label) => envText.includes(label)),
    envText.slice(0, 200)
  );
  // 上一轮被裁掉的字段：逐条拿后端的值去界面上找。
  check(
    "官方 Node 最低版本显示出来了（后端 minimumNodeMajor）",
    envText.includes(`${env.minimumNodeMajor}+`),
    `minimumNodeMajor=${env.minimumNodeMajor}`
  );
  if (env.nvmVersion) {
    check(
      "nvm 版本显示出来了（后端 nvmVersion，此前界面上没有这一行）",
      envText.includes(env.nvmVersion),
      `nvmVersion=${env.nvmVersion}`
    );
  }
  if (env.npmVersion) {
    check("npm 版本显示出来了", envText.includes(env.npmVersion), `npmVersion=${env.npmVersion}`);
  }
  check(
    "「目标环境」写出来了（旧平台的叫法）",
    envText.includes("目标环境"),
    envText.slice(0, 120)
  );

  // Node 大版本选择：两个选项来自后端的推荐与最低版本。
  const nodeSelect = [...document.querySelectorAll(".settings-content select")].find((item) =>
    [...item.options].some((option) => /LTS/.test(option.textContent ?? ""))
  );
  out.nodeSelectOptions = nodeSelect
    ? [...nodeSelect.options].map((item) => item.textContent)
    : null;
  check(
    "有 Node 大版本选择，两个选项是「推荐 / 兼容」两个 LTS",
    nodeSelect &&
      out.nodeSelectOptions.length >= 1 &&
      out.nodeSelectOptions.some((label) => label.includes("推荐")) &&
      out.nodeSelectOptions.some((label) => label.includes("兼容")),
    JSON.stringify(out.nodeSelectOptions)
  );
  check(
    "默认选中后端给的**推荐**版本（不是界面写死的 24）",
    nodeSelect && nodeSelect.value === String(env.recommendedNodeMajor),
    `value=${nodeSelect?.value} recommended=${env.recommendedNodeMajor}`
  );

  // ── 5.5 诊断页 ─────────────────────────────────────────
  out.openedDiagnostics = await openTab("系统诊断");
  const diagText = clean(".settings-content");
  out.diagText = diagText.slice(0, 500);

  check(
    "诊断页有「能力矩阵」",
    diagText.includes("能力矩阵"),
    diagText.slice(0, 200)
  );
  // 能力矩阵的行必须反映真实能力，而不是恒真/恒假。
  const qoder = (providers ?? []).find((item) => item.id === "qoder");
  out.qoderCaps = qoder?.capabilities;
  check(
    "能力矩阵按当前平台渲染（十项都在）",
    ["Skill", "分支", "Usage", "回收站", "批量", "工作目录", "导出", "会话设置", "复制", "供应商管理"].every(
      (label) => diagText.includes(label)
    ),
    diagText.slice(0, 300)
  );
  check(
    "命令状态用的是旧平台的措辞（可用 / 未找到命令）",
    diagText.includes("可用") || diagText.includes("未找到命令"),
    diagText.slice(0, 200)
  );
  check("诊断页有「路径」那一块", diagText.includes("路径"), diagText.slice(0, 200));
  check(
    "路径表说明了为什么不列历史目录与回收站（而不是默默少一块）",
    diagText.includes("provider") || diagText.includes("历史目录"),
    diagText.slice(0, 260)
  );

  // 配置目录那一行的存在性探测要真的跑过。
  //
  // **不能用 `includes("存在")`**：表头写的是「存在性」，那三个字里就有「存在」，
  // 于是探针即使一次都没探过也会通过——第一版就是这么骗过自己的。
  // 真正的判据是「**没有**停在「未检查」上」：那是没探过的初始值。
  check(
    "配置目录的存在性探测真的跑过（没有停在初始的「未检查」）",
    !diagText.includes("未检查"),
    `未检查出现=${diagText.includes("未检查")}`
  );

  // ── 4.6 全局检索按平台筛选 ──────────────────────────────
  // 先关掉覆盖层，再切到会话侧栏。
  const closeButton = [...document.querySelectorAll(".settings-head button")].find(
    (item) => (item.getAttribute("aria-label") || "").trim() === "关闭"
  );
  if (closeButton) {
    closeButton.click();
    await p.sleep(600);
  }
  // 一级去处现在在**活动栏**（`shell/activity-bar.tsx`），不再是 `.pane-nav .chip`。
  // 会话本来就是默认那一项，点它只是保证侧栏不是工作台。
  const toSessions = [...document.querySelectorAll(".activity-item")].find(
    (item) => (item.getAttribute("aria-label") || "") === "AI 会话管理"
  );
  if (toSessions) {
    toSessions.click();
    await p.sleep(800);
  }
  /*
    **「全部会话」按钮已随旧搜索行移除。**

    现在的入口是用户真实那条路：在搜索框里打字（列表进入筛选态），
    再点「在所有目标中搜索」把关键词送进 `globalQuery`；那之后侧栏才渲染
    「按平台筛选 / 按目标筛选」两个下拉（`features/sessions/sessions-sidebar.tsx`）。
    照着旧按钮找会永远停在本地筛选态，两个下拉根本不存在——这正是它原来 3 条 FAIL 的根因。
  */
  const searchInput = document.querySelector(".sidebar-search input");
  if (searchInput) {
    const setInput = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
    setInput.call(searchInput, "a");
    searchInput.dispatchEvent(new Event("input", { bubbles: true }));
    await p.sleep(1200);
  }
  const searchAll = [...document.querySelectorAll(".search-actions button")].find(
    (item) => clean0(item) === "在所有目标中搜索"
  );
  out.clickedSearchAll = Boolean(searchAll);
  if (searchAll) {
    searchAll.click();
    await p.sleep(2500);
  }

  const searchText = clean(".sidebar");
  out.searchText = searchText.slice(0, 400);
  const searchSelects = [...document.querySelectorAll(".sidebar select")].map((item) => ({
    label: item.getAttribute("aria-label"),
    options: [...item.options].map((option) => option.textContent)
  }));
  out.searchSelects = searchSelects;

  check(
    "检索面板有「按平台筛选」下拉",
    searchSelects.some((item) => item.label === "按平台筛选"),
    JSON.stringify(searchSelects.map((item) => item.label))
  );
  check(
    "检索面板有「按目标筛选」下拉",
    searchSelects.some((item) => item.label === "按目标筛选"),
    JSON.stringify(searchSelects.map((item) => item.label))
  );
  const providerSelect = searchSelects.find((item) => item.label === "按平台筛选");
  check(
    "平台下拉里有「全部平台」与各平台名",
    providerSelect &&
      providerSelect.options.includes("全部平台") &&
      (providers ?? []).every((item) => providerSelect.options.includes(item.label)),
    JSON.stringify(providerSelect?.options)
  );

  const failed = out.checks.filter((item) => !item.ok);
  out.verdict =
    failed.length === 0
      ? `PASS：${out.checks.length} 项补齐验收全部通过`
      : `FAIL：${failed.length}/${out.checks.length} 未通过 —— ${failed.map((f) => f.name).join("、")}`;
  return out;
})();
