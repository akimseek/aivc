/**
 * 通过 CDP 截图 / 求值 / 输入，供自动化验证使用。
 *
 * # 为什么用远程调试协议
 *
 * 这是一个 Tauri + WebView2 应用，内核是 Chromium，因此支持 CDP。相比「截屏」，CDP
 * 的 `Page.captureScreenshot` 是**离屏渲染**：不依赖窗口可见、不会被其它窗口遮挡。
 *
 * # 环境变量（**必须走环境变量**）
 *
 * | 变量 | 作用 |
 * | --- | --- |
 * | `CDP_PROBE` | 页面加载后执行的 JS，结果打印出来 |
 * | `CDP_PROBE_WAIT` | 执行 probe 前的等待毫秒数（默认 2500） |
 * | `CDP_TYPE` | probe 之后**逐个字符**派发键盘事件，再回车 |
 * | `CDP_TYPE_WAIT` | 输入前等待毫秒数（默认 5000） |
 * | `CDP_READ` | 输入之后再求值一次并打印 |
 *
 * **从命令行传表达式会被 PowerShell 吃掉双引号**：`querySelectorAll(".row")` 会变成
 * `querySelectorAll(.row)` 并报 `SyntaxError: Unexpected token '.'`。单引号包裹的
 * PowerShell 字符串能原样进环境变量，不经过那层转义。
 *
 * 应用必须已经用 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`
 * 启动。**这是验证专用的启动方式，不要写进发布配置。**
 */

const fs = require("node:fs");

const outPath = process.argv[2] ?? "shot.png";
const probe = process.env.CDP_PROBE || "";
const readAfter = process.env.CDP_READ || "";
const typeText = process.env.CDP_TYPE || "";
const probeWait = Number(process.env.CDP_PROBE_WAIT ?? 2500);
const typeWait = Number(process.env.CDP_TYPE_WAIT ?? 5000);

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function findPageTarget() {
  const response = await fetch("http://127.0.0.1:9222/json/list");
  const targets = await response.json();
  const page = targets.find((item) => item.type === "page");
  if (!page) throw new Error("没有找到 page 目标");
  return page.webSocketDebuggerUrl;
}

async function main() {
  const ws = new WebSocket(await findPageTarget());
  let nextId = 0;
  const pending = new Map();

  function send(method, params = {}) {
    const id = ++nextId;
    ws.send(JSON.stringify({ id, method, params }));
    return new Promise((resolve) => pending.set(id, resolve));
  }

  ws.onmessage = (event) => {
    const message = JSON.parse(event.data);
    if (message.id && pending.has(message.id)) {
      pending.get(message.id)(message);
      pending.delete(message.id);
    }
  };

  await new Promise((resolve, reject) => {
    ws.onopen = resolve;
    ws.onerror = (error) => reject(new Error(error.message ?? String(error)));
  });

  /** 求值并打印。**必须显式检查异常**：抛错时返回值也是 undefined。 */
  async function evaluate(label, expression) {
    const result = await send("Runtime.evaluate", {
      expression,
      returnByValue: true,
      awaitPromise: true
    });
    const failure = result.result?.exceptionDetails;
    if (failure) {
      console.error(`--- ${label} 抛异常 ---`);
      console.error(failure.exception?.description ?? failure.text);
      return;
    }
    const value = result.result?.result?.value;
    console.log(`--- ${label} ---`);
    console.log(typeof value === "string" ? value : JSON.stringify(value));
  }

  // 给页面时间完成首次渲染与数据加载，否则会拍到空白。
  await sleep(probeWait);
  if (probe) await evaluate("求值结果", probe);

  // 输入：验证键盘链路（前端 onData → terminal:write → PTY → 回显）。
  if (typeText) {
    await sleep(typeWait);
    // 让 xterm 的隐藏 textarea 拿到焦点，否则按键没人接。
    await send("Runtime.evaluate", {
      expression: 'document.querySelector(".xterm-helper-textarea")?.focus()'
    });
    for (const char of typeText) {
      await send("Input.dispatchKeyEvent", { type: "char", text: char });
    }
    const enter = {
      key: "Enter",
      code: "Enter",
      windowsVirtualKeyCode: 13,
      nativeVirtualKeyCode: 13
    };
    await send("Input.dispatchKeyEvent", { type: "keyDown", text: "\r", ...enter });
    await send("Input.dispatchKeyEvent", { type: "keyUp", ...enter });
    await sleep(1500);
  }

  if (readAfter) await evaluate("输入后求值", readAfter);

  const shot = await send("Page.captureScreenshot", { format: "png" });
  if (!shot.result?.data) {
    throw new Error(`截图失败：${JSON.stringify(shot).slice(0, 400)}`);
  }
  fs.writeFileSync(outPath, Buffer.from(shot.result.data, "base64"));
  console.log(`--- 截图已保存：${outPath}（${fs.statSync(outPath).size} 字节）---`);
  ws.close();
}

main().then(
  () => process.exit(0),
  (error) => {
    console.error(String(error));
    process.exit(1);
  }
);
