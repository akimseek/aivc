import { Field, Select, TextInput } from "../../../components";
import {
  CUSTOM_BOUNDS_MESSAGES,
  customBounds,
  USAGE_RANGES,
  type UsageRangeKey
} from "../../../app/gateway-range";

/**
 * 「时间范围」一栏：预设 + 自定义起止日期。
 *
 * # 为什么抽成一个组件
 *
 * 同一个东西在**三个**面板里都要有（用量报表、异常诊断、日志清理）。旧平台也是三处
 * 各写一遍，于是三处的提示语、默认值、校验都略有不同——用户会觉得「同一件事在
 * 不同页面上不一样」。这里合成一处，三处行为必然一致。
 *
 * # 为什么把校验结果**显示出来**而不是只禁用按钮
 *
 * 旧平台的两条提示是给用户看的（`请选择完整的起止日期。` /
 * `起始日期不能晚于结束日期。`）。只把按钮置灰的话，用户不知道差什么、
 * 也不知道自己填反了——他会以为是页面坏了。
 */
export function GatewayRangePicker({
  range,
  start,
  end,
  onChange
}: {
  range: UsageRangeKey;
  /** 自定义区间的起始日期（`YYYY-MM-DD`）。 */
  start: string;
  /** 自定义区间的结束日期（`YYYY-MM-DD`）。 */
  end: string;
  onChange: (next: { range: UsageRangeKey; start: string; end: string }) => void;
}) {
  const parsed = range === "custom" ? customBounds(start, end) : null;
  const message = parsed && !parsed.ok ? CUSTOM_BOUNDS_MESSAGES[parsed.reason] : "";

  return (
    <>
      <Field label="时间范围">
        <Select
          value={range}
          aria-label="时间范围"
          onChange={(event) =>
            onChange({ range: event.target.value as UsageRangeKey, start, end })
          }
        >
          {USAGE_RANGES.map((item) => (
            <option key={item.key} value={item.key}>
              {item.label}
            </option>
          ))}
        </Select>
      </Field>
      {range === "custom" && (
        <>
          <Field label="开始日期">
            <TextInput
              type="date"
              value={start}
              aria-label="开始日期"
              onChange={(event) => onChange({ range, start: event.target.value, end })}
            />
          </Field>
          <Field label="结束日期">
            <TextInput
              type="date"
              value={end}
              aria-label="结束日期"
              onChange={(event) => onChange({ range, start, end: event.target.value })}
            />
          </Field>
        </>
      )}
      {/*
        校验结果放在这里，而不是塞进某个 `Field` 的 `hint`：
        它是**整组**的问题（缺一端 / 顺序反了），不属于某一个框。
      */}
      {message && <p className="field-hint range-message">{message}</p>}
    </>
  );
}
