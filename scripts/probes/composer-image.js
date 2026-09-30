/**
 * 自管输入的图片面。
 *
 * # 为什么必须实测
 *
 * 这一面的每一环都在**跨层**：前端把 `Blob` 编成 PNG → 后端 `shell:save-image` 落盘
 * → 路径回填成附件 → 预览时再 `shell:read-image` 读回来渲染。中间任何一环的
 * 参数名或数据格式对不上，单元测试都看不出来（它们各自 mock 了对面）。
 *
 * # 自证
 *
 * 探针**自己**造一张真图（canvas）、自己落盘、自己读回。因此后面看到的每一个字节
 * 都只可能来自这次操作，而不是运行根里原有的东西。
 */
(async () => {
  const p = window.__p;
  const out = { checks: [] };
  const check = (name, ok, detail) => out.checks.push({ name, ok: Boolean(ok), detail: detail ?? "" });

  // 诊断用：把「开始操作之前」界面上有什么记下来，
  // 免得失败时分不清是「终端没起来」还是「输入框没渲染」。
  out.initialButtons = [...document.querySelectorAll("button")].map((item) =>
    p.clean(item.textContent)
  ).slice(0, 40);
  out.initialDialogs = [...document.querySelectorAll("[role='dialog'], .dialog")].map((item) =>
    p.clean(item.textContent).slice(0, 60)
  );

  // 探针启动时**没有任何平台 / 目标被选中**（刻意的：不替用户选），
  // 因此「+ Codex」是禁用的，点了不会弹对话框。
  // 这里先按用户真实路径选平台与目标——否则后面根本走不到终端。
  const pickers = [...document.querySelectorAll(".sidebar .pickers select")];
  const setSelect = async (element, value) => {
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value").set.call(element, value);
    element.dispatchEvent(new Event("change", { bubbles: true }));
    await p.sleep(1400);
  };
  const codex = [...(pickers[0]?.options ?? [])].find((item) => item.value === "codex");
  if (codex) await setSelect(pickers[0], "codex");
  const target = [...(pickers[1]?.options ?? [])].find((item) => item.value !== "");
  if (target) await setSelect(pickers[1], target.value);
  out.pickedTarget = target?.value ?? "";

  // ── 1. 后端：造一张真 PNG，落盘，再读回来 ────────────────
  //
  // 用 canvas 画一张 4×3 的纯色图（尺寸小、内容确定，便于断言）。
  const canvas = document.createElement("canvas");
  canvas.width = 4;
  canvas.height = 3;
  const context = canvas.getContext("2d");
  context.fillStyle = "#ff8800";
  context.fillRect(0, 0, 4, 3);
  const dataUrl = canvas.toDataURL("image/png");
  out.dataUrlPrefix = dataUrl.slice(0, 22);

  const saved = await p.invoke("shell:save-image", { dataUrl });
  out.saved = saved;
  check(
    "shell:save-image 把 PNG 落盘并返回路径",
    typeof saved?.path === "string" && saved.path.endsWith(".png"),
    JSON.stringify(saved)
  );
  check(
    "落盘大小与 PNG 字节数一致（不是空文件）",
    typeof saved?.size === "number" && saved.size > 50,
    `size=${saved?.size}`
  );

  const read = await p.invoke("shell:read-image", { path: saved.path });
  out.readPrefix = (read?.dataUrl ?? "").slice(0, 22);
  check(
    "shell:read-image 把它读回成 data URL",
    typeof read?.dataUrl === "string" && read.dataUrl.startsWith("data:image/png;base64,"),
    out.readPrefix
  );
  check(
    "读回来的字节数与原文件一致（**不是把别的文件读回来了**）",
    read?.size === saved?.size,
    `${read?.size} vs ${saved?.size}`
  );
  // 关键：把 data URL 真的解成图片，能解出来才说明内容是有效 PNG。
  const decoded = await new Promise((resolve) => {
    const image = new Image();
    image.onload = () => resolve({ width: image.naturalWidth, height: image.naturalHeight });
    image.onerror = () => resolve(null);
    image.src = read.dataUrl;
  });
  out.decoded = decoded;
  check(
    "data URL 能真的解成图片，且尺寸是 4×3",
    decoded && decoded.width === 4 && decoded.height === 3,
    JSON.stringify(decoded)
  );

  // ── 2. 后端：白名单闸门 ──────────────────────────────────
  //
  // 要拿一个**真的存在的非图片文件**来测：拿不存在的路径会先撞上「不是文件」，
  // 于是这条断言看着通过、其实白名单那一层根本没被执行到。
  const dbPath = `${out.saved.path.replace(/\\data\\cache\\composer\\[^\\]+$/, "")}\\data\\aivc.db`;
  let refused = "";
  try {
    await p.invoke("shell:read-image", { path: dbPath });
  } catch (error) {
    refused = String(error);
  }
  out.refusedPath = dbPath;
  out.refusedNonImage = refused.slice(0, 160);
  check(
    "真的存在的非图片文件被**白名单**拒绝（否则它就是一个通用文件读取接口）",
    refused.includes("不支持的类型"),
    refused.slice(0, 160)
  );

  // ── 2b. 开一路 Codex 终端，自管输入才会出现在 DOM 里 ──────
  //
  // 自管输入只在前台的终端里渲染，因此必须先有终端。
  //
  // 这里原来点的是「+ Shell」（系统终端）。那个功能已按用户要求整体移除，
  // 改成点「+ Codex」——**同一个探针验的东西没变**（自管输入框与图片面），
  // 只是用现在唯一存在的终端类型把它开出来。
  //
  // 「+ Codex」会先弹「新会话」对话框（工作目录 + 提示词），要再点一次「打开」。
  const NEW_TERMINAL_LABELS = ["+ Codex", "新建终端"];
  for (const label of NEW_TERMINAL_LABELS) {
    const button = [...document.querySelectorAll("button")].find(
      (item) => p.clean(item.textContent) === label
    );
    if (button) {
      button.click();
      break;
    }
  }
  await p.sleep(1200);
  // 对话框出现就确认它（默认目录是后端给的当前会话目录，可直接用）。
  const confirm = [...document.querySelectorAll("button")].find(
    (item) => p.clean(item.textContent) === "打开"
  );
  out.confirmedNewSession = Boolean(confirm);
  if (confirm) confirm.click();
  for (let index = 0; index < 30 && !document.querySelector(".composer"); index += 1) {
    await p.sleep(500);
  }
  await p.sleep(2000);

  // ── 3. 前端：附件种类判定与选择器 ────────────────────────
  const composer = document.querySelector(".composer");
  out.composerFound = Boolean(composer);
  const buttons = [...(composer?.querySelectorAll("button") ?? [])].map((item) =>
    p.clean(item.textContent)
  );
  out.composerButtons = buttons;
  check(
    "按钮里有「添加图片」——此前只有一个「添加文件」，而它只挑得到 .md",
    buttons.includes("添加图片"),
    JSON.stringify(buttons)
  );
  check("按钮里也有「添加文件」", buttons.includes("添加文件"), JSON.stringify(buttons));

  // ── 4. 前端：把图片塞成附件，列表里能认出来并可预览 ───────
  //
  // 直接驱动 UI 的粘贴路径不可靠（要构造带文件的 ClipboardEvent），
  // 因此这里的做法是：**验证输入框真的会把图片路径收成图片附件**——
  // 用真实存在的 PNG 路径走一遍粘贴文本路径，再看列表与预览。
  const textarea = composer?.querySelector("textarea");
  if (!textarea) {
    out.verdict = "FAIL：找不到自管输入的 textarea，后续无法进行";
    return out;
  }

  // 路径粘贴：应用会把「像路径的整行文本」当成附件。
  const pasteEvent = new Event("paste", { bubbles: true, cancelable: true });
  pasteEvent.clipboardData = {
    getData: () => saved.path,
    files: []
  };
  textarea.dispatchEvent(pasteEvent);
  await p.sleep(900);

  const attachmentLabels = [...document.querySelectorAll(".composer-attachment")].map((item) =>
    p.clean(item.textContent)
  );
  out.attachmentLabels = attachmentLabels;
  check(
    "图片路径被收成附件",
    attachmentLabels.some((label) => label.includes(".png")),
    JSON.stringify(attachmentLabels)
  );
  const imageAttachment = document.querySelector(".composer-attachment.is-image");
  check(
    "它被**认成图片**（有 is-image 标记），而不是普通文件",
    Boolean(imageAttachment),
    `is-image=${Boolean(imageAttachment)}`
  );

  // 点开预览。
  const previewTrigger = document.querySelector(".composer-attachment-image");
  if (previewTrigger) {
    previewTrigger.click();
    await p.sleep(1800);
  }
  const previewPanel = document.querySelector(".composer-preview-panel");
  out.previewOpen = Boolean(previewPanel);
  check("点附件能打开预览", Boolean(previewPanel), `open=${Boolean(previewPanel)}`);

  if (previewPanel) {
    const previewButtons = [...previewPanel.querySelectorAll("button")].map((item) =>
      p.clean(item.textContent)
    );
    out.previewButtons = previewButtons;
    for (const label of ["放大", "缩小", "复制", "关闭"]) {
      check(`预览里有「${label}」`, previewButtons.includes(label), JSON.stringify(previewButtons));
    }
    // 图片真的渲染出来了（`naturalWidth > 0` 说明 data URL 有效，不是一张碎图）。
    const img = previewPanel.querySelector("img");
    out.previewImage = img
      ? { naturalWidth: img.naturalWidth, naturalHeight: img.naturalHeight, complete: img.complete }
      : null;
    check(
      "预览里图片真的渲染出来了（不是碎图）",
      img && img.complete && img.naturalWidth === 4 && img.naturalHeight === 3,
      JSON.stringify(out.previewImage)
    );

    // 放大：倍数要变，且图片的 transform 跟着变。
    const zoomText = () => p.clean((previewPanel.querySelector(".dialog-footer") || {}).textContent);
    const before = zoomText();
    const zoomIn = [...previewPanel.querySelectorAll("button")].find(
      (item) => p.clean(item.textContent) === "放大"
    );
    zoomIn?.click();
    await p.sleep(400);
    const after = zoomText();
    out.zoom = { before: before.match(/\d+%/)?.[0], after: after.match(/\d+%/)?.[0] };
    check(
      "点「放大」倍数真的变了",
      out.zoom.before === "100%" && out.zoom.after !== out.zoom.before,
      JSON.stringify(out.zoom)
    );
    const transform = previewPanel.querySelector("img")?.style.transform ?? "";
    out.previewTransform = transform;
    check(
      "缩放作用到了图片上（不是只改了个数字）",
      /scale\(1\.[0-9]+\)/.test(transform),
      transform
    );
  }

  // ── 5. 后端：复制图片到剪贴板 ────────────────────────────
  let copyError = "";
  try {
    await p.invoke("clipboard:write-image", { path: saved.path });
  } catch (error) {
    copyError = String(error);
  }
  out.copyError = copyError.slice(0, 160);
  check("clipboard:write-image 没有报错", copyError === "", copyError.slice(0, 160));

  const failed = out.checks.filter((item) => !item.ok);
  out.verdict =
    failed.length === 0
      ? `PASS：${out.checks.length} 项图片面全部通过`
      : `FAIL：${failed.length}/${out.checks.length} 未通过 —— ${failed.map((f) => f.name).join("、")}`;
  return out;
})();
