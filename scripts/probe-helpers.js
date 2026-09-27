/**
 * 页面探针的公共助手。
 *
 * # 为什么要有这个文件
 *
 * 「启动应用 → 开终端 → 读它到底输出了什么 → 查网关收到了什么」这套动作，
 * 在最近几轮里我**每次都在临时拼**。它已经三次救场（定位 Windows 引号缺陷、
 * 拿到 CLI 的真实输出、证明请求到达网关），所以值得固化下来。
 *
 * # 怎么被使用
 *
 * `scripts/app-probe.ps1` 会把本文件与探针文件拼成一段表达式交给
 * `cdp-shot.cjs` 求值。因此：
 *
 * - 本文件是**语句**（定义 `window.__p`），
 * - 探针文件必须是**最后一个表达式**（通常是 `(async()=>{ … })()`），
 *   它的值就是打印出来的结果。
 *
 * # 里面每一样都是踩过坑才有的
 *
 * - [`watch`]：**必须替 xterm.js 应答 ConPTY 的光标查询（DSR）**。不应答时
 *   ConPTY 不推进会话，子进程连第一步都走不完——表现为「终端没有输出」，
 *   而进程其实好好地跑着。
 * - [`clean`]：终端输出里有光标控制序列与窗口标题（OSC），不剥掉就没法断言。
 * - [`invoke`]：后端只有一个命令 `aivc_invoke`，参数是 `{ command, args }`。
 */
window.__p = (() => {
  /** 调一条后端命令。 */
  const invoke = (command, args) =>
    window.__TAURI__.core.invoke("aivc_invoke", { command, args: args ?? null });

  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

  /** 剥掉控制序列并压平空白，便于断言。 */
  const clean = (value) =>
    (value || "")
      .replace(/\u001b\][^\u0007]*\u0007/g, "")
      .replace(/\u001b\[[0-9;?]*[a-zA-Z]/g, "")
      .replace(/\s+/g, " ")
      .trim();

  /** 当前终端面板渲染出来的文本。 */
  const rows = () => clean((document.querySelector(".xterm-rows") || {}).textContent);

  /** 状态栏上的「当前供应商」。 */
  const statusVendor = () => clean((document.querySelector(".status-vendor") || {}).textContent);

  /**
   * 订阅一路终端，**并应答 DSR**，把输出攒成一整段文本。
   *
   * 返回的对象一直有效：读 `state.text` 拿当前累计输出。
   */
  const watch = async (terminalId) => {
    const state = { text: "", dsr: 0, chunks: 0 };
    await window.__TAURI__.event.listen("terminal:data", (event) => {
      if (event.payload.terminalId !== terminalId) return;
      const text = new TextDecoder().decode(Uint8Array.from(event.payload.data));
      state.chunks += 1;
      state.text += text;
      if (text.includes("\u001b[6n")) {
        state.dsr += 1;
        invoke("terminal:write", { terminalId, data: "\u001b[1;1R" });
      }
    });
    return state;
  };

  /** 等待某个选择器出现。 */
  const waitFor = async (selector, tries = 80, step = 250) => {
    for (let index = 0; index < tries; index += 1) {
      const found = document.querySelector(selector);
      if (found) return found;
      await sleep(step);
    }
    return null;
  };

  /** 等目标下拉框里出现真实目标（探测是异步的）。 */
  const waitForTargets = async (tries = 90, step = 500) => {
    for (let index = 0; index < tries; index += 1) {
      const select = [...document.querySelectorAll(".pickers select")].find((item) =>
        [...item.options].some((option) => /:local|:wsl/.test(option.value))
      );
      if (select) return select;
      await sleep(step);
    }
    return null;
  };

  /** 在原生 setter 上赋值并派发 change——React 只认这样的事件。 */
  const selectValue = (element, value) => {
    const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value").set;
    setter.call(element, value);
    element.dispatchEvent(new Event("change", { bubbles: true }));
  };

  /** 状态栏那个「+ 平台」按钮——它按**当前选中的目标**决定开哪一路终端。 */
  const terminalButton = () =>
    [...document.querySelectorAll("button")].find((item) => /^\s*\+/.test(item.textContent || ""));

  /** 网关用量汇总（默认覆盖一个足够宽的区间）。 */
  const usage = (periodStart = "2026-01-01T00:00:00.000Z", periodEnd = "2030-12-31T23:59:59.999Z") =>
    invoke("gateway:get-usage-summary", { periodStart, periodEnd });

  return {
    invoke,
    sleep,
    clean,
    rows,
    statusVendor,
    watch,
    waitFor,
    waitForTargets,
    selectValue,
    terminalButton,
    usage
  };
})();
