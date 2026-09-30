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

  it("children 为 null 时不渲染空 `.card-body`", () => {
    // `null` / `undefined` 都要判：空 `.card-body` 会白占一段内边距。
    // （`{cond && <…/>}` 为假时传进来的是 `false`，见下一条。）
    expect(render(<Card>{null}</Card>).container.querySelector(".card-body")).toBeNull();
    expect(render(<Card />).container.querySelector(".card-body")).toBeNull();
  });

  it("**`actions={false}` 不渲染空 `.card-actions`**（`{cond && <…/>}` 为假时是 `false`）", () => {
    // 判据只写 `!== undefined` 时，这个 `false` 会换来一个空的动作区：
    // 卡片头上多出一段没有内容的右半边，看上去像加载失败。
    const alone = render(<Card actions={false}>正文</Card>);
    expect(alone.container.querySelector(".card-actions")).toBeNull();
    // 三样全空时连卡片头都不该有（`false` 也要算「没有」）。
    expect(alone.container.querySelector(".card-head")).toBeNull();

    // 有标题时卡片头要在，但动作区仍然不该出现。
    const withTitle = render(
      <Card title="标题" description={false} actions={false}>
        正文
      </Card>
    );
    expect(withTitle.container.querySelector(".card-title")?.textContent).toBe("标题");
    expect(withTitle.container.querySelector(".card-actions")).toBeNull();
    expect(withTitle.container.querySelector(".card-description")).toBeNull();
  });
});
