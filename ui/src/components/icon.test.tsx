// @vitest-environment jsdom
import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Icon, ICON_NAMES } from "./icon";

/**
 * 图标注册表的完整性。
 *
 * 最要紧的一条：**名字表与路径表必须同步**——漏一个名字时 `Icon` 会渲染空 svg，
 * 界面上那一处就悄悄没有图标（这类缺陷在截图里很难注意到）。
 */
describe("Icon", () => {
  it("每个注册的名字都渲染出至少一条 path", () => {
    for (const name of ICON_NAMES) {
      const view = render(<Icon name={name} />);
      const svg = view.container.querySelector("svg");
      expect(svg, `icon ${name} 没有渲染 svg`).not.toBeNull();
      expect(svg!.querySelectorAll("path,circle,rect,line").length, `icon ${name} 是空的`).toBeGreaterThan(0);
      view.unmount();
    }
  });

  it("有 title 时 svg 带 role=img 与 aria-label，没有时不带", () => {
    const withTitle = render(<Icon name="chat" title="会话" />);
    const svg = withTitle.container.querySelector("svg")!;
    expect(svg.getAttribute("role")).toBe("img");
    expect(svg.getAttribute("aria-label")).toBe("会话");

    const without = render(<Icon name="chat" />);
    expect(without.container.querySelector("svg")!.getAttribute("aria-hidden")).toBe("true");
  });

  it("尺寸落到 width/height", () => {
    const view = render(<Icon name="close" size={20} />);
    const svg = view.container.querySelector("svg")!;
    expect(svg.getAttribute("width")).toBe("20");
    expect(svg.getAttribute("height")).toBe("20");
  });
});
