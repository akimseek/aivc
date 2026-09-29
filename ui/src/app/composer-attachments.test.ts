import { describe, expect, it } from "vitest";

import {
  appendAttachments,
  attachmentFromPath,
  baseName,
  isImagePath,
  stepZoom,
  zoomLabel,
  type ComposerAttachment
} from "./composer-attachments";

/**
 * 自管输入的附件模型。
 *
 * # 为什么要单测
 *
 * 「是不是图片」这个判断**必须与后端的白名单一致**。不一致的时候界面会给一张
 * 预览不了的图配一个预览按钮，点下去报「不支持的类型」——用户看不出哪里错了。
 */

describe("isImagePath", () => {
  it("认得常见的图片后缀", () => {
    for (const path of [
      "C:\\a\\b.png",
      "C:\\a\\b.JPG",
      "/home/u/截图.jpeg",
      "x.gif",
      "x.webp",
      "x.bmp",
      "x.svg"
    ]) {
      expect(isImagePath(path), path).toBe(true);
    }
  });

  it("非图片一律为假", () => {
    for (const path of ["a.txt", "a.md", "a.jsonl", "a", "a.pngx", "a.png.bak"]) {
      expect(isImagePath(path), path).toBe(false);
    }
  });

  it("**点开头的隐藏文件不算**——`.png` 是文件名不是后缀", () => {
    expect(isImagePath("/home/u/.png")).toBe(false);
  });

  it("大写后缀也认（Windows 上很常见）", () => {
    expect(isImagePath("C:\\x\\A.PNG")).toBe(true);
    expect(isImagePath("C:\\x\\A.JpEg")).toBe(true);
  });
});

describe("baseName", () => {
  it("两种分隔符都认——WSL 目标给的是 `/`，本机是 `\\`", () => {
    expect(baseName("C:\\a\\b.png")).toBe("b.png");
    expect(baseName("/home/u/b.png")).toBe("b.png");
  });

  it("没有分隔符时原样返回", () => {
    expect(baseName("b.png")).toBe("b.png");
  });
});

describe("attachmentFromPath", () => {
  it("按后缀定种类并取显示名", () => {
    const image = attachmentFromPath("C:\\a\\shot.png", "a1");
    expect(image).toEqual({ id: "a1", kind: "image", path: "C:\\a\\shot.png", name: "shot.png" });
    const file = attachmentFromPath("/home/u/notes.md", "a2");
    expect(file.kind).toBe("file");
    expect(file.name).toBe("notes.md");
  });
});

describe("appendAttachments", () => {
  const item = (path: string, id: string): ComposerAttachment =>
    attachmentFromPath(path, id);

  it("追加新的", () => {
    const result = appendAttachments([item("/a", "1")], [item("/b", "2")]);
    expect(result.map((entry) => entry.path)).toEqual(["/a", "/b"]);
  });

  it("**同一条路径只留一份**——拖动时很容易重复命中同一个文件", () => {
    const result = appendAttachments([item("/a", "1")], [item("/a", "2"), item("/b", "3")]);
    expect(result.map((entry) => entry.path)).toEqual(["/a", "/b"]);
  });

  it("一条都没加上时**返回原来的数组**（不触发多余的重渲染）", () => {
    const current = [item("/a", "1")];
    expect(appendAttachments(current, [item("/a", "2")])).toBe(current);
    expect(appendAttachments(current, [])).toBe(current);
  });
});

describe("缩放", () => {
  it("放大缩小各自乘除同一个步进，能回到原值", () => {
    expect(stepZoom(1, "in")).toBe(1.25);
    expect(stepZoom(1.25, "out")).toBe(1);
  });

  it("夹在 0.25×–4×：再小看不清、再大只是把同一张图放大成马赛克", () => {
    expect(stepZoom(0.3, "out")).toBe(0.25);
    expect(stepZoom(4, "in")).toBe(4);
  });

  it("**浮点连乘不会留下 1.9531250000000002 这种值**", () => {
    let zoom = 1;
    for (let index = 0; index < 6; index += 1) zoom = stepZoom(zoom, "in");
    expect(Number.isInteger(zoom * 100)).toBe(true);
    expect(String(zoom)).not.toMatch(/\d{6,}/);
  });

  it("NaN 不会让它返回 NaN（会让 `scale(NaN)` 失效、图直接不见）", () => {
    // 断言的是**契约**（有限、在范围内），不是某个具体数字：
    // 兜底值取 1（中性倍数）还是别的都不该让这条测试失败。
    for (const input of [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY]) {
      const value = stepZoom(input, "in");
      expect(Number.isFinite(value), String(input)).toBe(true);
      expect(value).toBeGreaterThanOrEqual(0.25);
      expect(value).toBeLessThanOrEqual(4);
    }
  });

  it("文案是百分比整数", () => {
    expect(zoomLabel(1)).toBe("100%");
    expect(zoomLabel(1.25)).toBe("125%");
    expect(zoomLabel(0.25)).toBe("25%");
  });
});
