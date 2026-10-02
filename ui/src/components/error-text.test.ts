import { describe, expect, it } from "vitest";

import { errorText } from "./error-text";

describe("errorText", () => {
  it("Error 取 message", () => {
    expect(errorText(new Error("炸了"))).toBe("炸了");
  });

  it("非 Error 走 String：字符串原样、数字成文", () => {
    expect(errorText("炸了")).toBe("炸了");
    expect(errorText(42)).toBe("42");
  });
});
