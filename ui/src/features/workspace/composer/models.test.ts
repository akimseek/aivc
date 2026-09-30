import { describe, expect, it } from "vitest";

import { resolveModelSource } from "./models";

/**
 * 模型来源的判据。
 *
 * 这条规则决定**输入区里有没有那一项**，而它错了不会报错：要么是一个永远失败的
 * 下拉（选错来源），要么是本该有的功能凭空消失（判据太严）。因此逐条钉住。
 */

describe("resolveModelSource", () => {
  it("有供应商 → 用供应商的模型清单", () => {
    expect(resolveModelSource({ providerId: "codex", vendorId: "vendor-1" })).toEqual({
      kind: "vendor",
      vendorId: "vendor-1"
    });
  });

  it("Qoder 有目标 → 用 CLI 自己列的模型", () => {
    // Qoder 没有供应商候选池能力（`vendorManagement=false`），它的模型只能来自 CLI。
    expect(resolveModelSource({ providerId: "qoder", targetId: "qoder:local" })).toEqual({
      kind: "qoder",
      targetId: "qoder:local"
    });
  });

  it("Qoder 缺目标 → 不显示（models:list 只认 targetId）", () => {
    expect(resolveModelSource({ providerId: "qoder" })).toBeNull();
  });

  it("既不是 Qoder 又没有供应商 → 不显示", () => {
    // 宁可没有这一项，也不要一个永远失败的下拉。
    expect(resolveModelSource({ providerId: "codex" })).toBeNull();
  });
});
