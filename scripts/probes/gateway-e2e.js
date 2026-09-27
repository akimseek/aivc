/**
 * 「真实 CLI → 本地网关」端到端探针。
 *
 * # 它回答的问题
 *
 * 打开一个终端之后，**真实 CLI 有没有向我们创建的那条路由发出请求**。
 * 这是「供应商 + 本地网关」链条上最要紧的一环，也是整条链上最后才被验证到的一环。
 *
 * # 为什么必须用非交互模式
 *
 * 交互式 TUI 要用户敲一句提示才会发请求。`<平台> exec <提示>` 这类非交互入口
 * 一启动就发。Codex 还需要 `--skip-git-repo-check`：它拒绝在**未被信任的目录**里运行，
 * （`Not inside a trusted directory and --skip-git-repo-check was not specified.`），
 * 那与网关无关，但会让探针看起来「什么都没发生」。
 *
 * # 上游是故意打不通的
 *
 * 供应商地址写 `http://127.0.0.1:1`（不会有服务在听）。因此**预期结果就是失败**——
 * 我们要证明的是「请求到达了网关、通过了鉴权、被记录」，而不是「上游调通了」。
 * 网关会用**它自己的**文案回应（例如 `gateway vendor unavailable`），
 * 终端里能看到这句话，就是最硬的证据。
 *
 * 运行：
 *   .\scripts\app-probe.ps1 -Probe probes\gateway-e2e.js -Log '绑定网关路由|补进子进程 PATH|panic'
 */
(async () => {
  const p = window.__p;
  const args = window.__pArgs || {};
  const provider = args.provider || "codex";

  /**
   * 各平台的**非交互**入口。
   *
   * 交互式 TUI 要用户敲一句提示才会发请求，因此每个平台都得找到它的非交互形式：
   *
   * | 平台 | 参数 | 备注 |
   * | --- | --- | --- |
   * | codex | `exec --skip-git-repo-check hi` | Codex 拒绝在**未被信任的目录**里运行（`Not inside a trusted directory…`）——与网关无关，但会让探针看起来「什么都没发生」 |
   * | claude | `-p hi` | `--print`，一次性输出 |
   * | gemini | `-p hi` | 同上（本机未安装，无法验证） |
   * | qoder | —— | **不支持网关**，永远不会创建路由，因此这个探针对它没有意义 |
   */
  const NON_INTERACTIVE = {
    codex: "exec --skip-git-repo-check hi",
    claude: "-p hi",
    gemini: "-p hi"
  };
  const cliArgs = args.cliArgs || NON_INTERACTIVE[provider] || "";
  const terminalId = args.terminalId || `probe-e2e-${provider}`;
  const waitMs = args.waitMs || 40000;

  const out = { provider, cliArgs, terminalId };

  // 1. 存一个供应商。上游故意指向一个不会有人监听的端口。
  try {
    out.vendor = await p.invoke("vendor:save", {
      providerId: provider,
      name: "探针供应商",
      apiKey: "sk-probe",
      apiBaseUrl: args.apiBaseUrl || "http://127.0.0.1:1",
      enabled: true,
      sort: 1
    });
  } catch (error) {
    out.saveError = String(error.message || error);
  }
  await p.sleep(2500);

  // 2. 订阅输出并**应答 DSR**——不应答时 ConPTY 不推进，终端永远没有输出。
  const watched = await p.watch(terminalId);

  // 3. 直接走命令面启动终端：这样才能带上 `cliArgs`（界面上没有这个入口）。
  try {
    out.started = await p.invoke("terminal:start", {
      terminalId,
      providerId: provider,
      cliArgs,
      ...(args.targetId ? { targetId: args.targetId } : {})
    });
  } catch (error) {
    out.startError = String(error.message || error);
  }

  await p.sleep(waitMs);

  // 4. 终端里 CLI 自己说了什么。
  out.terminalTail = p.clean(watched.text).slice(-600);
  out.dsrAnswered = watched.dsr;
  out.outputChunks = watched.chunks;

  // 5. 网关收到了什么。
  try {
    out.usage = await p.usage();
  } catch (error) {
    out.usageError = String(error.message || error);
  }

  // 6. 判定。**只有「网关收到过请求」才算通过**——终端里有输出不等于请求发出去了。
  out.verdict =
    (out.usage?.requestCount ?? 0) > 0
      ? "PASS：CLI 的请求到达了网关"
      : "FAIL：网关没有收到任何请求";
  return JSON.stringify(out);
})();
