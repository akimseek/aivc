import type { AiProviderId, FailoverRule, FailoverRuleScope } from "../../types";

/**
 * 故障规则的草稿模型、校验与提交形状（卡 3 的「纯逻辑」那一半）。
 *
 * # 为什么要从表单里拆出来
 *
 * 规则卡片里同时有一张表、一张表单和一组校验。三者挤在一起之后，
 * 「到底提交了什么」会被渲染结构埋掉，而这一页最容易出的缺陷恰恰在那里
 * （自定义返回配了一半、作用域没带目标）。这里只有数据与判断，没有 JSX。
 *
 * # 保存与试规则必须共用同一份载荷
 *
 * `gateway:test-failover-rule` 走的是与转发路径**同一份**规则引擎（core 的
 * `match_rule`），因此「试出会命中」的前提是**试的东西与存下去的东西一个字都不差**。
 * 两边各拼一份载荷，就会出现「测试说会命中、保存后不切」——最难查的一类偏差。
 * 所以两者都走 `rulePayload`。
 */

/**
 * 规则编辑草稿。
 *
 * 状态码用字符串存——「空」与「空数组」在后端不是同一件事（空数组 = 不按状态码判定）。
 */
export type RuleDraft = {
  id: string;
  scope: FailoverRuleScope;
  /**
   * 作用域为 provider / vendor 时的目标平台。
   *
   * 后端要求它**必填**（`FailoverRuleProviderRequired`：Provider 级和供应商级规则
   * 必须指定 Provider）；此前表单里没有这一项，于是按平台/按供应商的规则
   * 从界面上根本存不下去。
   */
  providerId: AiProviderId | "";
  /** 作用域为 vendor 时的目标供应商；全局与按平台作用域不传。 */
  vendorId: string;
  pattern: string;
  statusCodes: string;
  enabled: boolean;
  /**
   * 命中时网关自己返回给客户端的**状态码与正文**（而不是把上游的错原样透传）。
   *
   * 两项**必须同时配置**（后端校验：「自定义返回状态码和内容必须同时配置。」），
   * 因此这里存成字符串、提交时一起判断。
   */
  customStatus: string;
  customBody: string;
  /** 试规则用的模拟输入。**不是规则的一部分**，因此不参与提交。 */
  testStatus: string;
  testMessage: string;
};

/** 新规则的初值。 */
export function emptyRuleDraft(): RuleDraft {
  return {
    id: "",
    scope: "global",
    providerId: "",
    vendorId: "",
    pattern: "",
    statusCodes: "",
    enabled: true,
    customStatus: "",
    customBody: "",
    // 试规则的输入给一个**能立刻命中**的默认值：用户点「试一下」就想看到反馈，
    // 默认空着会一律得到「没命中」，看起来像功能坏了。
    testStatus: "429",
    testMessage: ""
  };
}

/** 从一条已存在的规则回填草稿。**每一项都要回填**，漏掉的那项会在「保存修改」时被清掉。 */
export function draftFromRule(rule: FailoverRule): RuleDraft {
  return {
    id: rule.id,
    scope: rule.scope,
    providerId: rule.providerId ?? "",
    vendorId: rule.vendorId ?? "",
    pattern: rule.pattern,
    statusCodes: rule.statusCodes?.join(",") ?? "",
    enabled: rule.enabled,
    // 不回填的话，点一次「保存修改」就会把用户配好的自定义返回当成「没填」而清掉
    // （供应商那三项踩过同一个坑）。
    customStatus:
      rule.customResponseStatus === undefined ? "" : String(rule.customResponseStatus),
    customBody: rule.customResponseBody ?? "",
    testStatus: (rule.statusCodes?.[0] ?? 429).toString(),
    testMessage: ""
  };
}

/**
 * 解析「自定义返回」两项。
 *
 * 四条校验照旧平台：
 * - 状态码必须是 100–599 的整数；
 * - 正文必须是合法 JSON；
 * - 正文不超过 32 KB；
 * - **状态码与正文必须同时配置**。
 *
 * 放在提交之前统一判，返回第一处错误——一次只报一条比堆一串更难读。
 * 后端也有同样的校验（这里是提前告知，不是替代）。
 */
export function parseCustomResponse(
  status: string,
  body: string
): { ok: true; status?: number; body?: string } | { ok: false; message: string } {
  const statusText = status.trim();
  const bodyText = body.trim();
  if (statusText === "" && bodyText === "") return { ok: true };
  if (statusText === "" || bodyText === "") {
    return { ok: false, message: "自定义返回状态码和内容必须同时配置。" };
  }
  const code = Number(statusText);
  if (!Number.isInteger(code) || code < 100 || code > 599) {
    return { ok: false, message: "自定义返回状态码需为 100 到 599 之间的整数。" };
  }
  // 32 KB 按**字节**算（UTF-8）：中文字符一个占三字节，按字符数算会放过三倍的量。
  const size = new TextEncoder().encode(bodyText).length;
  if (size > 32 * 1024) {
    return { ok: false, message: "自定义返回内容不能超过 32 KB。" };
  }
  try {
    JSON.parse(bodyText);
  } catch {
    return { ok: false, message: "自定义返回内容必须是合法 JSON。" };
  }
  return { ok: true, status: code, body: bodyText };
}

/** 提交给后端的规则本体（`gateway:save-failover-rule` 与试规则里的 `rule` 同形状）。 */
export type RulePayload = {
  id?: string;
  scope: FailoverRuleScope;
  providerId?: AiProviderId;
  vendorId?: string;
  pattern: string;
  statusCodes?: number[];
  customResponseStatus?: number;
  customResponseBody?: string;
  enabled: boolean;
};

/**
 * 草稿 → 提交形状。**保存与试规则共用**（见文件头）。
 *
 * 返回第一处错误：整体不提交。提交一半再报错，用户不知道哪一项存进去了。
 */
export function rulePayload(
  draft: RuleDraft
): { ok: true; value: RulePayload } | { ok: false; message: string } {
  const custom = parseCustomResponse(draft.customStatus, draft.customBody);
  if (!custom.ok) return custom;
  // 作用域目标：后端也校验这两条（`aivc-core/src/gateway.rs:270-291`），
  // 这里是提前告知，文案照后端原话。
  if (draft.scope !== "global" && draft.providerId === "") {
    return { ok: false, message: "Provider 级和供应商级规则必须指定 Provider。" };
  }
  if (draft.scope === "vendor" && draft.vendorId.trim() === "") {
    return { ok: false, message: "供应商级规则必须指定供应商。" };
  }
  return {
    ok: true,
    value: {
      ...(draft.id ? { id: draft.id } : {}),
      scope: draft.scope,
      // 全局作用域**不传**目标：后端会判「全局规则不能指定 Provider 或供应商」。
      ...(draft.scope === "global" ? {} : { providerId: draft.providerId as AiProviderId }),
      ...(draft.scope === "vendor" ? { vendorId: draft.vendorId.trim() } : {}),
      pattern: draft.pattern.trim(),
      // 状态码用逗号分隔；空表示「不按状态码判定」，因此**不传**这个字段，
      // 而不是传空数组——两者在后端不是同一件事。
      ...(draft.statusCodes.trim()
        ? {
            statusCodes: draft.statusCodes
              .split(",")
              .map((part) => Number.parseInt(part.trim(), 10))
              .filter((value) => Number.isInteger(value) && value > 0)
          }
        : {}),
      ...(custom.status === undefined ? {} : { customResponseStatus: custom.status }),
      ...(custom.body === undefined ? {} : { customResponseBody: custom.body }),
      enabled: draft.enabled
    }
  };
}
