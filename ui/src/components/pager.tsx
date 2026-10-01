import type { ReactNode } from "react";

import { Button } from "./primitives";

/**
 * 分页控件：第一页 / 上一页 / 「第 x / y 页」/ 下一页（可选「最后一页」）。
 *
 * # 为什么共用一份
 *
 * 三处（请求日志、异常诊断、消息与分支弹框）此前各写了一遍同一组按钮。
 * 三份手写就有三处走样的机会——边界禁用写错一处、页码算法差一，翻页行为
 * 就会「这一页跟那一页不一样」。差异只有四类，全部由 props 表达：
 *
 * | 差异 | 入口 |
 * | --- | --- |
 * | 弹框翻页时要额外禁用（加载中） | `disabled` |
 * | 弹框在按钮前有「每页数量」选择器 | `extra` |
 * | 弹框多一个「最后一页」 | `withLast` |
 * | 页码文字的字号两处不同 | `labelClassName` |
 * | 弹框的页码变化要播报给读屏 | `ariaLive` |
 *
 * # 返回值是 Fragment，不是自带容器
 *
 * 两处的容器不同：卡片头里是 `.toolbar`（按钮与占位撑开排成一行），弹框里是
 * `.messages-pagination`（带 padding 与分隔线）。Pager 自带一层容器就会多一层
 * 嵌套、间距与现有布局对不上；返回 Fragment 让调用方的容器直接排布这些按钮，
 * DOM 与替换前逐个一致。
 *
 * # 不钳制页码
 *
 * `page` 的合法范围由调用方的状态决定。这里只按边界禁用按钮，不替调用方把
 * 越界值改掉——钳制会把「状态不对」这件事藏起来，而它恰恰是要被看见的。
 */
export function Pager({
  page,
  pageCount,
  onPage,
  disabled = false,
  extra,
  withLast = false,
  labelClassName = "row-meta",
  ariaLive = false
}: {
  page: number;
  pageCount: number;
  /** 翻页回调。页码从 1 起。 */
  onPage: (page: number) => void;
  /** 额外禁用（弹框的「加载中」）。与页码边界**或**起来决定禁用态。 */
  disabled?: boolean;
  /** 按钮序列**之前**的内容（弹框的「每页数量」选择器与它后面的占位）。 */
  extra?: ReactNode;
  /** 末尾多一个「最后一页」。 */
  withLast?: boolean;
  /** 页码文字的类名：卡片头里是 `row-meta`，弹框里是 `messages-range`。 */
  labelClassName?: string;
  /** 页码变化要不要播报给读屏（弹框在消息区上方，翻页时它是不错的定位信息）。 */
  ariaLive?: boolean;
}) {
  return (
    <>
      {extra}
      {/* 四个按钮共用同一套边界算法：页码从 1 起，第一页时前两个禁、最后一页时后两个禁。 */}
      <Button small disabled={disabled || page <= 1} title="第一页" onClick={() => onPage(1)}>
        第一页
      </Button>
      <Button
        small
        disabled={disabled || page <= 1}
        title="上一页"
        onClick={() => onPage(page - 1)}
      >
        上一页
      </Button>
      <span className={labelClassName} aria-live={ariaLive ? "polite" : undefined}>
        第 {page} / {pageCount} 页
      </span>
      <Button
        small
        disabled={disabled || page >= pageCount}
        title="下一页"
        onClick={() => onPage(page + 1)}
      >
        下一页
      </Button>
      {withLast && (
        <Button
          small
          disabled={disabled || page >= pageCount}
          title="最后一页"
          onClick={() => onPage(pageCount)}
        >
          最后一页
        </Button>
      )}
    </>
  );
}
