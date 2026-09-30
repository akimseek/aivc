import { useCallback, useEffect, useState } from "react";

import { Button, Card, Field, TextInput } from "../../components";
import * as api from "../../ipc/commands";
import type { GatewayPortStatus } from "../../types";

/**
 * 卡 1：网关端口与熔断韧性参数（demo 第 1024-1075 行）。
 *
 * # 为什么是「集中保存」而不是每项一个保存按钮
 *
 * 这五项（端口 + 三条阈值）说的是同一件事——网关怎么分配端口、怎么判定失败。
 * 逐项保存时用户要按五次，而且**按到哪一项了看不出来**；旧平台原话就是
 * 「统一保存端口分配与熔断规则；端口需重启应用生效，阈值即时生效」。
 *
 * # 保存只发改过的项
 *
 * 端口与阈值是**四个不同的命令**（后端各有一条 setter），集中保存只是界面上的
 * 一个按钮。全部重发一遍会让每次保存都写四次设置；因此这里只发真正变了的项，
 * 一个都没变时连请求都不发（并说清「没有改动」，否则用户会以为按钮点坏了）。
 *
 * # 校验先做完再发第一个请求
 *
 * 集中保存下「存了一半」比逐项保存更难解释：用户只看到一句错误，却不知道
 * 端口是不是已经改了。因此四项全部本地校验通过之后才发第一个请求；
 * 中途某条失败时把已经落库的状态回灌给界面（见 catch）。
 */

/** 三个阈值。范围与错误文案照旧平台原文；后端也有同样的校验，这里只是为了早一步提示。 */
const THRESHOLDS = [
  {
    key: "failure" as const,
    field: "failureThreshold" as const,
    label: "异常切换阈值",
    /** 标签里的取值范围。范围不写出来，用户只能靠报错去试。 */
    rangeText: "次，1–10",
    hint: "同一供应商连续请求失败达到该次数后，才切换到候选池中的下一个供应商。",
    min: 1,
    max: 10,
    message: "异常切换阈值必须是 1 到 10 之间的整数。",
    call: api.setGatewayFailureThreshold
  },
  {
    key: "circuit" as const,
    field: "circuitFailureThreshold" as const,
    label: "熔断次数",
    rangeText: "次，1–20",
    hint: "供应商在该时间窗口内累计失败达到次数后进入熔断状态。",
    min: 1,
    max: 20,
    message: "熔断次数必须是 1 到 20 之间的整数。",
    call: api.setGatewayCircuitFailureThreshold
  },
  {
    key: "duration" as const,
    field: "circuitDurationSeconds" as const,
    label: "熔断持续时间",
    rangeText: "秒，10–86400",
    hint: "熔断期间不会选择该供应商；到期后进入半开状态并允许一次试探请求。",
    min: 10,
    max: 86400,
    message: "熔断持续时间必须是 10 到 86400 秒之间的整数。",
    call: api.setGatewayCircuitDurationSeconds
  }
];

type DraftKey = (typeof THRESHOLDS)[number]["key"] | "port";

/** 输入框草稿。数字用字符串存——「空」与「0」在输入框里不是一回事。 */
type Draft = Record<DraftKey, string>;

function draftOf(status: GatewayPortStatus): Draft {
  return {
    port: String(status.configuredPort),
    failure: String(status.failureThreshold ?? ""),
    circuit: String(status.circuitFailureThreshold ?? ""),
    duration: String(status.circuitDurationSeconds ?? "")
  };
}

export function GatewayPortCard({
  status,
  onStatusChange,
  onError,
  onNotify
}: {
  /** 网关的完整状态。由面板取一次后传下来——**不让两张卡各存一份**，否则总开关一按就不同步。 */
  status: GatewayPortStatus;
  onStatusChange: (next: GatewayPortStatus) => void;
  onError: (message: string) => void;
  onNotify?: (message: string) => void;
}) {
  const [draft, setDraft] = useState<Draft>(() => draftOf(status));
  const [saving, setSaving] = useState(false);
  /** 复制成功的一次性提示——复制是「做完没有任何反馈」的典型动作。 */
  const [copied, setCopied] = useState(false);

  // 只在这五项变化时重灌草稿：父级把状态换成新对象是常事（例如切了总开关），
  // 那种变化与这些输入框无关，重灌会把用户还没保存的输入抹掉。
  useEffect(() => {
    setDraft(draftOf(status));
  }, [
    status.configuredPort,
    status.failureThreshold,
    status.circuitFailureThreshold,
    status.circuitDurationSeconds
  ]);

  const copyAddress = useCallback(async () => {
    if (!status.baseUrl) return;
    try {
      await api.copyText(status.baseUrl);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    }
  }, [status.baseUrl, onError]);

  const saveAll = async () => {
    // 端口：空表示自动分配（与后端 `require_gateway_port` 一致，它接受 0）。
    const portText = draft.port.trim();
    const port = portText === "" ? 0 : Number(portText);
    if (!Number.isInteger(port) || port < 0 || port > 65535) {
      onError("网关端口必须是 0 到 65535 之间的整数。端口 0 表示自动分配。");
      return;
    }
    const values = { failure: 0, circuit: 0, duration: 0 };
    for (const item of THRESHOLDS) {
      const value = Number(draft[item.key].trim());
      if (!Number.isInteger(value) || value < item.min || value > item.max) {
        onError(item.message);
        return;
      }
      values[item.key] = value;
    }

    setSaving(true);
    // 已经落库的那几项要留着：中途失败时下面 catch 里回灌，界面才不会显示旧值。
    let next: GatewayPortStatus | null = null;
    try {
      const parts: string[] = [];
      const portChanged = port !== status.configuredPort;
      if (portChanged) {
        next = await api.setGatewayPort(port);
        parts.push(next.configuredPort === 0 ? "端口改为自动分配" : `端口 ${next.configuredPort}`);
      }
      for (const item of THRESHOLDS) {
        if (values[item.key] === status[item.field]) continue;
        // 三个 setter 返回的都是**完整状态**，因此直接接住最后一个即可。
        next = await item.call(values[item.key]);
        parts.push(`${item.label} ${values[item.key]}`);
      }
      if (next) onStatusChange(next);
      if (parts.length === 0) {
        onNotify?.("网关配置没有改动。");
      } else {
        // 端口改了但**不会立刻生效**（要重启才重绑），这句话不说出来
        // 用户会以为点了没反应。
        onNotify?.(`已保存：${parts.join("、")}。${portChanged ? "端口需重启应用后生效。" : ""}`);
      }
    } catch (error) {
      if (next) onStatusChange(next);
      onError(error instanceof Error ? error.message : String(error));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Card
      title="网关端口与熔断韧性参数"
      description="统一保存端口分配与熔断规则；端口需重启应用生效，阈值即时生效。"
      actions={
        <Button
          small
          variant="primary"
          disabled={saving}
          title="保存端口与三条阈值（只提交改动过的项）"
          onClick={() => void saveAll()}
        >
          {saving ? "保存中…" : "保存网关与熔断配置"}
        </Button>
      }
    >
      <div className="field-grid">
        <Field
          label="配置端口"
          hint="填 0 表示自动分配。改完需重启应用才生效；端口被占用时网关会自动回退到可用端口。"
        >
          <div className="gateway-port-input">
            <TextInput
              value={draft.port}
              inputMode="numeric"
              aria-label="配置端口"
              onChange={(event) => setDraft({ ...draft, port: event.target.value })}
            />
            {/* 用户改的是「下次启动用哪个端口」，这里告诉他**现在**跑在哪个端口上。 */}
            <span className="row-meta">
              {status.activePort > 0 ? `活跃端口 ${status.activePort} · 监听中` : "未监听"}
            </span>
          </div>
        </Field>

        {/*
          平台 Base URL：**别的客户端该连哪里**。值由后端给（它读的就是注入给 CLI 的那个
          地址）——界面自己拼的话，端口为 0（自动分配）时只能拼出 `http://127.0.0.1:0`。
          各平台的完整接入端点在外部门卡里，这里给的是可复制的根地址。
        */}
        <Field label="平台 Base URL">
          {status.baseUrl ? (
            <div className="gateway-address">
              <code className="mono" title={status.baseUrl}>
                {status.baseUrl}
              </code>
              <Button small onClick={() => void copyAddress()}>
                {copied ? "已复制" : "复制"}
              </Button>
            </div>
          ) : (
            /*
              没有地址时要说清**下一步做什么**，而不是只说「没有」：
              按停在哪一步给不同的指引——关着就让他开，开着没监听就让他重启。
            */
            <span className="row-meta">
              {status.enabled
                ? "已启用但还没开始监听。重启应用后这里会出现可用地址。"
                : "尚未开启本地网关。请先在上面点「启用网关」，然后重启应用。"}
            </span>
          )}
        </Field>
      </div>
      {status.baseUrl && (
        // 端口是自动分配还是固定的，决定这个地址能不能长期用。不写清楚的话，
        // 用户可能拿一个「这次有效」的地址去配外部客户端，下次重启连不上，
        // 看起来像客户端坏了。
        <p className="field-hint">
          {status.configuredPort === 0
            ? "端口由系统自动分配，每次启动都可能不同。要长期给别的客户端用，请在上面填一个固定端口。"
            : `端口已固定在 ${status.configuredPort}，重启后地址不变。`}
        </p>
      )}

      <div className="gateway-thresholds">
        <div className="gateway-thresholds-title">失败转移与熔断阈值参数</div>
        <div className="field-grid gateway-threshold-grid">
          {THRESHOLDS.map((item) => (
            <Field
              key={item.key}
              label={`${item.label}（${item.rangeText}）`}
              hint={item.hint}
            >
              <TextInput
                value={draft[item.key]}
                inputMode="numeric"
                aria-label={item.label}
                onChange={(event) => setDraft({ ...draft, [item.key]: event.target.value })}
              />
            </Field>
          ))}
        </div>
      </div>
    </Card>
  );
}
