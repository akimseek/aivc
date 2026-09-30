import { StatusDot } from "../../components";

/**
 * 网关页的「总开关胶囊」。
 *
 * demo（第 1017、1090 行）把两个开关都画成这样：状态点 + 一句话 + 胶囊底。
 * 网关页有两处用它（卡头的网关总开关、外部网关卡头的外部网关开关），
 * 各抄一遍的话两处的间距与状态色一定会走样，因此单独成件。
 *
 * 它是**按钮**而不是状态标签：点一下就要切换开关。因此 `title`/`aria-label`
 * 写的是**点击后会做什么**（读屏用户看不出底色变化），而 `aria-pressed` 承载当前状态。
 *
 * # `disabled` 是给「请求在途」用的，不能删
 *
 * 两个调用方（`gateway-panel.tsx` 的总开关、`gateway-external-card.tsx` 的外部开关）
 * 都在切换请求发出到结果回来之间传 `disabled`：状态还没回来时第二下读到的仍是
 * **切换前**的 `on`，于是刚改完的状态又被翻回去——用户以为点快了没事，实际要多写
 * 一次设置。因此这个 prop 留着并**接上**，而不是删掉（它此前确实没有调用方）。
 */
export function SwitchPill({
  on,
  label,
  action,
  disabled = false,
  onClick
}: {
  on: boolean;
  /** 当前状态的文字，例如「本地网关已启用」。 */
  label: string;
  /** 点击后会做什么，例如「关闭本地网关」。 */
  action: string;
  /** 切换请求在途时为真（见上面那段说明）。 */
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className={`switch-pill${on ? "" : " off"}`}
      aria-pressed={on}
      aria-label={action}
      title={action}
      disabled={disabled}
      onClick={onClick}
    >
      <StatusDot tone={on ? "ok" : "idle"} pulse={on} />
      <span>{label}</span>
    </button>
  );
}
