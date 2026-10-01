// @vitest-environment jsdom
import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Badge } from "./badge";

describe("Badge", () => {
  it("色调与 mono 落到类名上", () => {
    const view = render(
      <Badge tone="accent" mono>
        14 条消息
      </Badge>
    );
    expect(view.container.firstElementChild!.className).toBe("badge badge-accent ui-num");
  });

  it("默认 neutral、非 mono", () => {
    const view = render(<Badge>候选</Badge>);
    expect(view.container.firstElementChild!.className).toBe("badge badge-neutral");
  });

  it("small 给出紧凑变体类，供密集表格使用", () => {
    // 密集表格里的状态标记此前靠 `.data-table .tag` 上下文选择器压到 1px 6px；
    // 上下文选择器在组件化后无从可见（同样的标记移出表格就变胖），改由显式变体表达。
    const view = render(
      <Badge small tone="neutral">
        超时
      </Badge>
    );
    expect(view.container.firstElementChild!.className).toBe("badge badge-neutral badge-sm");
  });

  it("small 与 mono 可叠加", () => {
    const view = render(
      <Badge small mono>
        0
      </Badge>
    );
    expect(view.container.firstElementChild!.className).toBe("badge badge-neutral badge-sm ui-num");
  });
});
