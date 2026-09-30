// @vitest-environment jsdom
import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Card } from "./card";

/**
 * 卡片头。
 *
 * 批次二把 `title` 改成可选（面板头已经写了这一页的标题时不再重复），
 * 于是 `title` / `description` / `actions` 三样全空的情形真的会出现。
 * 那种时候**不能渲染一个空的 `.card-head`**：它会白占一段间距，
 * 页面上多出一条说不清来历的空白。
 */
describe("Card：三样都没有时不渲染卡片头", () => {
  it("无 title / description / actions 时没有 `.card-head`，内容仍在", () => {
    const { container } = render(<Card>正文</Card>);
    expect(container.querySelector(".card-head")).toBeNull();
    expect(container.querySelector(".card-body")?.textContent).toBe("正文");
  });

  it("给了 title 时卡片头在（说明上面那条断言不是恒真的）", () => {
    const { container } = render(<Card title="标题">正文</Card>);
    expect(container.querySelector(".card-head .card-title")?.textContent).toBe("标题");
  });
});
