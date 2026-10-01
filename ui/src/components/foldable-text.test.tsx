// @vitest-environment jsdom
import { fireEvent, render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { FOLD_CHARS, FoldableText } from "./foldable-text";

/**
 * 长文折叠（消息弹框与历史正文共用一份）。
 *
 * 钉三点：**不超过阈值一字不折**（边界正好在 4000）；折叠只是显示层——
 * 展开是**全文**（一个字都不丢）；再点能收起。
 */
describe("FoldableText", () => {
  it("不超过阈值时不折叠、也不出现按钮", () => {
    const text = "字".repeat(FOLD_CHARS);
    const view = render(<FoldableText text={text} className="t" />);
    expect(view.container.querySelector(".t")!.textContent).toBe(text);
    expect(view.container.querySelector("button")).toBeNull();
  });

  it("超过阈值先折叠到阈值处，按钮写明总字数", () => {
    const text = "字".repeat(FOLD_CHARS + 1);
    const view = render(<FoldableText text={text} className="t" />);
    const block = view.container.querySelector(".t")!;
    // 前 4000 字 + 一个省略号。
    expect(block.textContent).toBe(`${"字".repeat(FOLD_CHARS)}…`);
    expect(view.container.textContent).toContain(`展开全文（共 ${FOLD_CHARS + 1} 字）`);
  });

  it("展开→全文；再点收起→回到折叠态", () => {
    const text = "字".repeat(FOLD_CHARS + 1);
    const view = render(<FoldableText text={text} className="t" />);
    const toggle = () => view.container.querySelector("button")!;

    fireEvent.click(toggle());
    expect(view.container.querySelector(".t")!.textContent).toBe(text);
    expect(toggle().textContent).toBe("收起");

    fireEvent.click(toggle());
    expect(view.container.querySelector(".t")!.textContent).toBe(`${"字".repeat(FOLD_CHARS)}…`);
    expect(toggle().textContent).toContain("展开全文");
  });
});
