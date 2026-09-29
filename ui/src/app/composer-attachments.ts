/**
 * 自管输入的附件模型与纯逻辑。
 *
 * # 为什么要单独一个文件
 *
 * 附件此前只是一个 `string[]`（路径）。加了图片之后它有了**种类**，
 * 而种类决定了三件事：列表里怎么画（缩略图还是一条路径）、预览给不给、以及
 * 「复制」按钮复制的是图片还是路径。
 *
 * 这些判断都是纯函数，放在组件里会让「图片能不能预览」这类规则散在 JSX 中间。
 */

/** 一张附件。 */
export type ComposerAttachment = {
  /** 列表的 key。同一路径可以被加两次（用户可能就是想提两遍）。 */
  id: string;
  kind: "image" | "file";
  /** 真实路径。CLI 收到的是 `@<这个>`。 */
  path: string;
  /** 显示名（没有路径时——例如粘贴失败——也不至于空着）。 */
  name: string;
};

/**
 * 按后缀判断是不是图片。
 *
 * # 为什么不看 MIME
 *
 * 拖进来的文件只有一个路径，`File.type` 是浏览器按后缀猜的，和这里一样不可靠；
 * 而粘贴进来的是 `Blob`（带 MIME，但那条路走的是另一个分支）。
 * 后缀判断**必须与后端 `IMAGE_EXTENSIONS` 一致**——不一致时界面会给一张
 * 预览不了的图配一个预览按钮，点了报「不支持的类型」。
 */
const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"];

export function isImagePath(path: string): boolean {
  const name = baseName(path);
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return false;
  return IMAGE_EXTENSIONS.includes(name.slice(dot + 1).toLowerCase());
}

/** 取路径最后一段。两种分隔符都要认：WSL 目标给的是 `/`，本机是 `\`。 */
export function baseName(path: string): string {
  const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return cut >= 0 ? path.slice(cut + 1) : path;
}

/** 由路径造一条附件。 */
export function attachmentFromPath(path: string, id: string): ComposerAttachment {
  return {
    id,
    kind: isImagePath(path) ? "image" : "file",
    path,
    name: baseName(path) || path
  };
}

/**
 * 去重后追加。
 *
 * 同一条路径只留一份：重复的 `@同一个文件` 对 CLI 没有意义，
 * 而用户拖动时很容易重复命中同一个文件。
 */
export function appendAttachments(
  current: ComposerAttachment[],
  incoming: ComposerAttachment[]
): ComposerAttachment[] {
  const seen = new Set(current.map((item) => item.path));
  const added = incoming.filter((item) => {
    if (seen.has(item.path)) return false;
    seen.add(item.path);
    return true;
  });
  return added.length === 0 ? current : [...current, ...added];
}

/**
 * 缩放后的倍数。
 *
 * 夹在 0.25×–4×：再小看不清、再大只是把同一张图放大成马赛克。
 */
export function clampZoom(zoom: number): number {
  if (!Number.isFinite(zoom)) return 1;
  return Math.min(Math.max(zoom, 0.25), 4);
}

/** 缩放步进。用乘法而不是加法：从 1× 加 0.25 到 1.25× 与减回去对称。 */
export const ZOOM_STEP = 1.25;

/** 下一次缩放的倍数。 */
export function stepZoom(zoom: number, direction: "in" | "out"): number {
  const next = direction === "in" ? zoom * ZOOM_STEP : zoom / ZOOM_STEP;
  // 先取整到两位小数再夹：浮点连乘会得到 1.9531250000000002 这种值，
  // 直接显示出来很难看。
  return clampZoom(Math.round(next * 100) / 100);
}

/** 缩放的显示文案。 */
export function zoomLabel(zoom: number): string {
  return `${Math.round(zoom * 100)}%`;
}
