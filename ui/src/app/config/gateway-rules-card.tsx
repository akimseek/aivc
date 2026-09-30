import { useCallback, useEffect, useState } from "react";

import { Button, Card, Icon, Toolbar } from "../../components";
import * as api from "../../ipc/commands";
import type { ApiVendor, FailoverRule } from "../../types";
import {
  draftFromRule,
  emptyRuleDraft,
  rulePayload,
  type RuleDraft
} from "./gateway-rule-model";
import { RuleForm } from "./gateway-rule-form";

/**
 * 卡 3：故障识别与自定义拦截规则（demo 第 1173-1369 行）。
 *
 * # 列表与表单互斥
 *
 * demo 是「列表视图 / 表单视图」两张视图：新增或编辑时把列表整个换掉。
 * 这样表单不会被表格挤到屏幕外，也不会出现「填到一半的规则混在列表里」的错觉。
 *
 * # 一个必须写进界面的语义
 *
 * `pattern` 是**不区分大小写的包含匹配，不是正则**。用户几乎一定会先按正则去写，
 * 因此提示必须显眼——写错正则不会报错，只会静默地匹配不到。
 */
export function GatewayRulesCard({
  onError,
  onNotify
}: {
  onError: (message: string) => void;
  onNotify?: (message: string) => void;
}) {
  const [rules, setRules] = useState<FailoverRule[]>([]);
  const [draft, setDraft] = useState<RuleDraft | null>(null);
  /**
   * 全部供应商。作用域选到「按供应商」时要出选项，而后端**要求** providerId + vendorId，
   * 没有这一份列表那个作用域就存不下去。
   */
  const [vendors, setVendors] = useState<ApiVendor[]>([]);
  const [testing, setTesting] = useState(false);
  /**
   * 上一次试规则的结果。
   *
   * `null` = 还没试过（与「试了没命中」是两件事，因此不能用一个布尔量表达）。
   */
  const [testResult, setTestResult] = useState<
    { matched: boolean; matchedBy: "status" | "pattern" | "none" } | null
  >(null);

  const load = useCallback(async () => {
    try {
      setRules(await api.listFailoverRules());
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    }
  }, [onError]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    // 供应商列表只用于表单里那一格下拉：取不到就不给选项（还能选按平台/全局），
    // 因此失败**不报错**，否则一进页面就弹一条与当前操作无关的错。
    void api
      .listVendors()
      .then((list) => setVendors(list ?? []))
      .catch(() => setVendors([]));
  }, []);

  /**
   * 打开/关闭表单。换一条规则（或收起）时，上一次的判定结论必须清掉——
   * 留着会让人以为它说的是新规则。
   */
  const open = (next: RuleDraft | null) => {
    setDraft(next);
    setTestResult(null);
  };

  const save = async () => {
    if (!draft || !draft.pattern.trim()) return;
    // 载荷拼装与试规则**共用**（见 `gateway-rule-model.ts` 的文件头说明）。
    const payload = rulePayload(draft);
    if (!payload.ok) {
      onError(payload.message);
      return;
    }
    const editing = draft.id !== "";
    try {
      await api.saveFailoverRule(payload.value);
      setDraft(null);
      setTestResult(null);
      await load();
      onNotify?.(editing ? "故障规则已保存。" : "已新增故障规则。");
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    }
  };

  /** 试当前这条草稿会不会命中。**不落库**，后端也只做纯计算。 */
  const test = async () => {
    if (!draft) return;
    const payload = rulePayload(draft);
    if (!payload.ok) {
      onError(payload.message);
      return;
    }
    setTesting(true);
    try {
      const statusCode = Number(draft.testStatus.trim());
      const result = await api.testFailoverRule({
        rule: payload.value,
        ...(Number.isInteger(statusCode) ? { statusCode } : {}),
        ...(draft.testMessage.trim() ? { message: draft.testMessage.trim() } : {})
      });
      setTestResult(result);
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    } finally {
      setTesting(false);
    }
  };

  const enabledCount = rules.filter((rule) => rule.enabled).length;

  return (
    <Card
      title="故障识别与自定义拦截规则"
      description="当上游返回指定错误文本或 HTTP 状态码时，触发自动切源，或由网关直接应答友好报错。"
      actions={
        <Button
          small
          variant="primary"
          disabled={draft !== null}
          title="打开新增规则表单"
          onClick={() => open(emptyRuleDraft())}
        >
          <Icon name="plus" size={12} />
          新增规则
        </Button>
      }
    >
      {draft ? (
        <RuleForm
          draft={draft}
          vendors={vendors}
          testing={testing}
          testResult={testResult}
          onChange={setDraft}
          onSubmit={() => void save()}
          onCancel={() => open(null)}
          onTest={() => void test()}
        />
      ) : rules.length === 0 ? (
        <div className="group-empty">还没有规则。没有规则时网关不会切换供应商。</div>
      ) : (
        <>
          <Toolbar>
            <span className="row-meta">
              {rules.length} 条 · {enabledCount} 条启用
            </span>
          </Toolbar>
          <table className="data-table">
            <thead>
              <tr>
                <th>作用域</th>
                <th>匹配</th>
                <th>状态码</th>
                {/*
                  「自定义返回」单列一栏：它是**命中之后客户端实际看到什么**，
                  与「匹配什么」是两个问题。配置了却看不出来，用户会怀疑自己没保存成功。
                */}
                <th>自定义返回</th>
                <th>状态</th>
                <th>操作</th>
              </tr>
            </thead>
            <tbody>
              {rules.map((rule) => (
                <tr key={rule.id}>
                  <td>{scopeText(rule)}</td>
                  <td className="mono">{rule.pattern}</td>
                  <td>{rule.statusCodes?.join(", ") ?? "-"}</td>
                  <td className="mono" title={rule.customResponseBody}>
                    {rule.customResponseStatus === undefined
                      ? // 没配就是「透传上游」——用 `-` 表示「未设置」，
                        // 而不是留空（留空看不出是没配还是没加载出来）。
                        "透传上游"
                      : `${rule.customResponseStatus}${rule.customResponseBody ? " · JSON" : ""}`}
                  </td>
                  <td>
                    <span className={`tag ${rule.enabled ? "ok" : "off"}`}>
                      {rule.enabled ? "已启用" : "已停用"}
                    </span>
                  </td>
                  <td>
                    <div className="gateway-rule-actions">
                      <Button small onClick={() => open(draftFromRule(rule))}>
                        编辑
                      </Button>
                      <Button
                        small
                        onClick={() =>
                          void api
                            .setFailoverRuleEnabled(rule.id, !rule.enabled)
                            .then(load)
                            .catch((error: unknown) =>
                              onError(error instanceof Error ? error.message : String(error))
                            )
                        }
                      >
                        {rule.enabled ? "停用" : "启用"}
                      </Button>
                      <Button
                        small
                        variant="danger"
                        onClick={() =>
                          void api
                            .deleteFailoverRule(rule.id)
                            .then(load)
                            .catch((error: unknown) =>
                              onError(error instanceof Error ? error.message : String(error))
                            )
                        }
                      >
                        删除
                      </Button>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
    </Card>
  );
}

/** 一行里怎么读作用域：带目标的要把目标也写出来，否则「按平台」的几条规则长得一模一样。 */
function scopeText(rule: FailoverRule): string {
  if (rule.scope === "provider") return `按平台 · ${rule.providerId ?? "?"}`;
  if (rule.scope === "vendor") return `按供应商 · ${rule.vendorId ?? "?"}`;
  return "全局";
}
