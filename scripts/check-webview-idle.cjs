// 在隔离运行根启动 AI.exe，并设置 WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=
// --remote-debugging-port=9234 后运行：node scripts/check-webview-idle.cjs
// 通过浏览器 CDP 读取各子进程 CPU 时间；百分比按单核计算，非任务管理器总 CPU 百分比。
// 对照暂停/恢复页面动画，不改供应商、会话或其他应用数据。
const port = Number(process.env.WEBVIEW_CDP_PORT || 9234);
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
async function connect(url) {
  const socket = new WebSocket(url);
  const pending = new Map(); let seq = 0;
  socket.onmessage = event => {
    const msg = JSON.parse(event.data);
    if (pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
  };
  await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject; });
  return { socket, send: (method, params = {}) => new Promise(resolve => {
    const id = ++seq; pending.set(id, resolve); socket.send(JSON.stringify({id, method, params}));
  }) };
}
(async () => {
  const version = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
  const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  const browser = await connect(version.webSocketDebuggerUrl);
  const page = await connect(targets.find(t => t.type === 'page').webSocketDebuggerUrl);
  const evaluate = async expression => {
    const result = await page.send('Runtime.evaluate', {expression, returnByValue: true, awaitPromise: true});
    if (result.result?.exceptionDetails) throw new Error(JSON.stringify(result.result.exceptionDetails));
    return result.result?.result?.value;
  };
  await page.send('Performance.enable');
  for (const phase of ['baseline', 'paused', 'restored']) {
    if (phase !== 'baseline') await evaluate(`document.getAnimations().forEach(a => a.${phase === 'paused' ? 'pause' : 'play'}())`);
    await sleep(1000);
    const animations = await evaluate(`document.getAnimations().map(a => ({name:a.animationName, state:a.playState, target:a.effect.target.className, visible:!!a.effect.target.getClientRects().length}))`);
    const before = await browser.send('SystemInfo.getProcessInfo');
    const metricsBefore = await page.send('Performance.getMetrics');
    const start = Date.now(); await sleep(8000);
    const after = await browser.send('SystemInfo.getProcessInfo');
    const metricsAfter = await page.send('Performance.getMetrics');
    const elapsed = (Date.now() - start) / 1000;
    const processes = (after.result?.processInfo || []).map(p => {
      const old = before.result?.processInfo.find(o => o.id === p.id);
      return {type:p.type, id:p.id, cpuSeconds:old ? +(p.cpuTime-old.cpuTime).toFixed(4) : null, oneCorePercent:old ? +((p.cpuTime-old.cpuTime)/elapsed*100).toFixed(2) : null};
    });
    const metrics = Object.fromEntries((metricsAfter.result?.metrics || []).filter(m => /TaskDuration|ScriptDuration|LayoutCount|RecalcStyleCount/.test(m.name)).map(m => [m.name, +(m.value - metricsBefore.result.metrics.find(o => o.name === m.name).value).toFixed(5)]));
    const result = {phase, animations, processes, metrics, processError:after.error};
    console.log(JSON.stringify(result));
  }
  browser.socket.close(); page.socket.close();
})().catch(error => {console.error(error); process.exitCode=1;});
