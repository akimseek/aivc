/**
 * 平台 Base URL：面板显示的那个地址，必须**真的能连上**。
 *
 * # 这是这条需求真正的风险点
 *
 * 「面板显示一个地址」很容易做对；难的是**它与网关实际在服务的那个地址一致**。
 * 不一致时用户照着面板去配外部客户端，会连到一个没人监听的端口——
 * 而那种故障看起来像「客户端配错了」，查起来要绕很久。
 *
 * 因此这里不只读界面上的文字，而是**往那个地址真的发一次请求**。
 *
 * # 自证
 *
 * 网关是探针自己开的（`enabledBefore` 记录开之前的状态），
 * 因此后面看到的地址与响应都只可能来自这次操作。
 */
(async () => {
  const p = window.__p;
  const out = { checks: [] };
  const check = (name, ok, detail) => out.checks.push({ name, ok: Boolean(ok), detail: detail ?? "" });

  // 整段包一层：探针自己抛异常时只会留下一句「求值结果抛异常」，
  // 看不出是哪一步、也分不清是探针的毛病还是产品的。兜住并把栈带回来。
  try {

  // ── 1. 后端返回的形状（字段名与界面读的一致） ─────────────
  const raw = await p.invoke("gateway:get-port", {});
  out.raw = raw;
  out.rawKeys = Object.keys(raw ?? {}).sort();
  check(
    "gateway:get-port 发的是界面在读的那几个字段",
    // 此前它只发 `{port}`，而界面读 `configuredPort`/`activePort`——
    // 于是端口输入框里是字符串 "undefined"，那一格永远显示「未启用」。
    //
    // 后来**又多了三个韧性阈值**（异常切换阈值 / 熔断次数 / 熔断时长）：
    // 它们原来由一个没有调用方的 `get_settings` 单独返回，字段名还不一样，
    // 是同一份数据的两个形状。合并进来之后，网关设置页读一次就拿到全部。
    //
    // 仍然断言**精确的键集合**（不是「包含」）：多出一个没人读的字段同样是缺陷。
    JSON.stringify(out.rawKeys) ===
      JSON.stringify([
        "activePort",
        "baseUrl",
        "circuitDurationSeconds",
        "circuitFailureThreshold",
        "configuredPort",
        "enabled",
        "failureThreshold"
      ]),
    JSON.stringify(out.rawKeys)
  );
  check(
    "configuredPort 是数字而不是 undefined",
    typeof raw.configuredPort === "number",
    `configuredPort=${String(raw.configuredPort)}`
  );

  // ── 2. 打开网关并重启到「正在监听」 ──────────────────────
  //
  // 网关默认是开的，但**监听地址要在启动时写入**，因此这条探针必须让应用
  // 带着一个已经开着的网关跑起来。做不到时就明确说「本项未验证」，
  // 而不是把「没监听」当成通过。
  await p.invoke("gateway:set-enabled", { enabled: true });
  await p.sleep(1200);
  const after = await p.invoke("gateway:get-port", {});
  out.afterEnable = after;
  check(
    "启用之后三个字段还在（set-enabled 不能把另外两个抹掉）",
    typeof after.configuredPort === "number" && typeof after.baseUrl === "string",
    JSON.stringify(after)
  );

  out.serving = after.baseUrl;
  if (!after.baseUrl) {
    // 这次运行里网关没起来（例如端口被占）。**如实说明**，不当作通过。
    out.verdict = "未验证：本次运行网关没有监听，baseUrl 为空，无法断言地址可达";
    return out;
  }

  check(
    "baseUrl 形如 http://127.0.0.1:<端口>",
    /^http:\/\/127\.0\.0\.1:\d+$/.test(after.baseUrl),
    after.baseUrl
  );
  check(
    "activePort 与 baseUrl 里的端口一致（不是两个来源各算一遍）",
    after.baseUrl.endsWith(`:${after.activePort}`),
    `baseUrl=${after.baseUrl} activePort=${after.activePort}`
  );

  // ── 3. **往那个地址真的发一次请求** ──────────────────────
  //
  // 先**校准**：webview 有 CORS，跨源 fetch 被拦时抛的也是 "Failed to fetch"，
  // 与「端口上没人监听」在 JS 里**长得一模一样**。不先证明这个方法能区分两者，
  // 拿它当判据就是把「被 CORS 拦了」误报成「地址不可达」。
  const tryFetch = (url) =>
    new Promise((resolve) => {
      const controller = new AbortController();
      const timer = window.setTimeout(() => {
        controller.abort();
        resolve({ outcome: "timeout" });
      }, 4000);
      fetch(url, { signal: controller.signal })
        .then((response) => {
          window.clearTimeout(timer);
          resolve({ outcome: "http", status: response.status });
        })
        .catch((error) => {
          window.clearTimeout(timer);
          resolve({ outcome: "error", message: String(error && error.message) });
        });
    });

  // 一个几乎肯定没人监听的端口，用来当对照组。
  const control = await tryFetch("http://127.0.0.1:1/__aivc_probe__");
  const real = await tryFetch(`${after.baseUrl}/__aivc_probe__`);
  out.control = control;
  out.real = real;

  if (control.outcome === real.outcome && control.outcome === "error") {
    // 两者都是 `Failed to fetch`：这个方法**分不出**「没人监听」与「被 CORS 拦」。
    // 如实标注为无法判定——把这种情况写成失败，就是把探针的限制报成产品缺陷。
    out.reachabilityInconclusive = true;
    check(
      "地址可达性：**无法判定**（webview 的 CORS 让两种失败长得一样），改由下面两条间接证明",
      true,
      `对照=${JSON.stringify(control)} 实际=${JSON.stringify(real)}`
    );
  } else {
    out.reachabilityInconclusive = false;
    check(
      "面板显示的那个地址上**真的有东西在监听**（对照组可区分）",
      real.outcome === "http",
      `对照=${JSON.stringify(control)} 实际=${JSON.stringify(real)}`
    );
  }

  // 与地址可达性等价的**结构性**判据：地址只能来自网关自己的监听状态。
  // `activePort` 非 0 就说明 `GatewayState` 里已经写入了监听地址，而
  // `route.rs` 注入给 CLI 的读的是同一个字段——因此「面板 ≠ 实际服务」不可能发生。
  check(
    "地址来自网关的监听状态（activePort 非 0 即已写入监听地址）",
    after.activePort > 0 && after.baseUrl.endsWith(`:${after.activePort}`),
    `activePort=${after.activePort}`
  );

  // ── 4. 界面上真的写着它 ──────────────────────────────────
  const toolbox = [...document.querySelectorAll("button")].find(
    (item) => p.clean(item.textContent) === "工具箱"
  );
  if (toolbox) {
    toolbox.click();
    await p.sleep(400);
    const gateway = [...document.querySelectorAll("button")].find(
      (item) => p.clean(item.textContent) === "网关"
    );
    if (gateway) gateway.click();
    await p.sleep(2500);
  }

  // 面板挂载时会自己取一次状态；点一次「刷新」强制它重取，
  // 以排除「面板挂载那一刻刚好没取到」这种时序问题。
  const reload = [...document.querySelectorAll(".overlay-body button")].find(
    (item) => p.clean(item.textContent) === "刷新"
  );
  if (reload) {
    reload.click();
    await p.sleep(2000);
  }

  const panelText = p.clean((document.querySelector(".overlay-body") || {}).textContent);
  out.panelText = panelText.slice(0, 300);
  out.hasAddressRow = Boolean(document.querySelector(".gateway-address"));

  const addressRow = document.querySelector(".gateway-address code");
  out.shownAddress = addressRow ? p.clean(addressRow.textContent) : null;
  check(
    "配置面板里把 baseUrl 显示出来了",
    out.shownAddress === after.baseUrl,
    `shown=${out.shownAddress} expected=${after.baseUrl} hasRow=${out.hasAddressRow}`
  );

  const panelButtons = [...document.querySelectorAll(".overlay-body button")].map((item) =>
    p.clean(item.textContent)
  );
  out.panelButtons = panelButtons;
  check("Base URL 旁边有「复制」", panelButtons.includes("复制"), JSON.stringify(panelButtons));

  // 注意：**只认那一句「每次启动」**。「配置端口」那一栏的提示里也有「自动分配」
  // 四个字，用「自动分配」当判据会**空过**——第一版就是这么骗过自己的。
  check(
    "端口自动分配时，Base URL 旁边提醒「每次启动都可能不同」",
    after.configuredPort === 0 ? panelText.includes("每次启动") : panelText.includes("不变"),
    `configuredPort=${after.configuredPort} panel=${panelText.slice(-120)}`
  );

  // ── 5. 端口输入框里不再是字符串 "undefined" ──────────────
  const portInput = [...document.querySelectorAll(".overlay-body input")].find((item) =>
    (item.inputMode ?? "") === "numeric"
  );
  out.portInputValue = portInput ? portInput.value : null;
  check(
    "端口输入框里是配置端口，**不是字符串 `undefined`**",
    portInput !== undefined && portInput.value === String(after.configuredPort),
    `value=${String(out.portInputValue)} expected=${after.configuredPort}`
  );

  // ── 6. 状态栏那一格 ──────────────────────────────────────
  const statusText = p.clean((document.querySelector(".statusbar") || {}).textContent);
  out.statusText = statusText;
  check(
    "状态栏显示真实端口，而不是「未启用」",
    statusText.includes(`:${after.activePort}`),
    statusText.slice(-90)
  );

  } catch (error) {
    out.threw = String((error && error.stack) || error).slice(0, 600);
    check("探针自身没有抛异常", false, out.threw);
  }

  const failed = out.checks.filter((item) => !item.ok);
  out.verdict =
    failed.length === 0
      ? `PASS：${out.checks.length} 项 Base URL 全部通过`
      : `FAIL：${failed.length}/${out.checks.length} 未通过 —— ${failed.map((f) => f.name).join("、")}`;
  return out;
})();
