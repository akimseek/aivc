/**
 * 模型列表从哪来。
 *
 * - 走网关（`vendorId` 存在）→ 那份供应商的模型（`vendor:list-models`）；
 * - Qoder 没有供应商候选池能力（`vendorManagement=false`），它的模型从 CLI 自己列
 *   （`models:list`，需要 targetId）——与旧平台一致（`listModels` 只在 Qoder 走）；
 * - 其余情况**不显示模型选择**（旧平台同样用 `modelSelectionSupported` 门控）。
 *   列不到模型时宁可没有这一项，也不要一个永远失败的下拉。
 */

/** 一份模型清单从哪取。`null` 表示这一路终端**没有**可用的模型来源。 */
export type ModelSource =
  | { kind: "vendor"; vendorId: string }
  | { kind: "qoder"; targetId: string };

export function resolveModelSource(input: {
  providerId: string;
  vendorId?: string;
  targetId?: string;
}): ModelSource | null {
  if (input.providerId === "qoder") {
    return input.targetId ? { kind: "qoder", targetId: input.targetId } : null;
  }
  return input.vendorId ? { kind: "vendor", vendorId: input.vendorId } : null;
}
