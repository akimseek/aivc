import { Card } from "../../components";
import { formatWhen } from "../../app/session-display";
import type { ApiVendor } from "../../types";
import { describeSwitchReason, vendorName, type RouteChange } from "./workbench-cards";

/**
 * 最近路由变更：谁把这一路会话接到了哪个供应商上。
 *
 * # 「无变更」为什么不摆一张空卡
 *
 * 一张写着标题、底下什么都没有的卡片会被读成「功能坏了」——旧组件的断言写的
 * 正是这件事（没有变更时那一块不渲染）。这里保留那个判断，但把「没有变更」
 * 本身**说出来**：它是有信息的（这一路从头到尾没换过供应商），不是缺数据。
 *
 * # 为什么只有「➔ B」
 *
 * 变更事件只带**目标**供应商编号（`lastSwitch.vendorId`）：原供应商在终端的
 * 状态里已经被覆盖，前端没有它。这里不编一个「A」出来——箭头指向的那个名字
 * 就是事实。
 */
export function RouteChangeCard({
  change,
  vendors
}: {
  change?: RouteChange;
  vendors: ApiVendor[];
}) {
  if (!change) {
    return <p className="workbench-hint">这一路会话还没有发生过路由变更。</p>;
  }
  return (
    <Card title="最近路由变更" description={describeSwitchReason(change.reason)}>
      <div className="route-change">
        <span className="route-change-label">供应商</span>
        <span className="route-change-arrow" aria-hidden="true">
          ➔
        </span>
        <strong className="route-change-vendor">{vendorName(vendors, change.vendorId)}</strong>
        <span className="route-change-time ui-num">{formatWhen(change.at) || "-"}</span>
      </div>
    </Card>
  );
}
