import { useCallback, useEffect, useRef, useState } from "react";

import * as api from "../../../ipc/commands";
import { resolveModelSource } from "./models";

/**
 * 输入区里的**模型选择**。
 *
 * # 它补的是一段缺失的能力
 *
 * 旧平台能从界面上切模型（选中即生效），Rust 版此前没有——用户只能自己在 CLI 里敲
 * `/model`，而模型名得先猜。这里把清单列出来，选一下就写 `/model <id>`：
 * **与旧平台一致，立即生效，不需要重启终端**。
 *
 * # 为什么用原生 `<select>`
 *
 * 它在输入区里只占很小一块，而且是「从一份清单里挑一个」这件事的标准控件：
 * 键盘可达、系统级的展开行为、不用自己写浮层的定位与失焦。清单里是模型编号，
 * 等宽字体就够了。
 *
 * # 为什么首次展开才取数
 *
 * 每开一路终端就发一次请求是不可接受的（Qoder 那条还要起一次 CLI 进程、最长 20 秒）。
 * 用户**真的去碰这个下拉**才算一次需要——原生 `<select>` 没有可靠的「展开」事件，
 * 因此 `pointerdown` / `focus` 是能拿到的最早信号（只认主键：右键与中键不算「要去选」）。
 *
 * # 失败要能再试
 *
 * 取数失败就把「无法获取模型」写在下拉里，**重新展开即重试**。失败一次就永久坏掉的
 * 控件比没有这个控件更糟：用户不知道是自己的操作不对还是它本来就不行。
 * 写入失败则把选中项复位——不复位的话它停在刚选的那一项上，同值再选不触发
 * `onChange`，用户就再也切不过去了。
 *
 * # 来源一变，清单作废
 *
 * `source`（哪一家供应商 / 哪个目标）变化时状态复位为 `idle`：供应商可能被**自动**
 * 故障转移换掉（用户什么都没做），列着上一家的模型只会让人选到一个不再服务这一路的
 * 供应商上去。
 *
 * # 没有来源就什么都不渲染
 *
 * 判据在 `models.ts`（`resolveModelSource`）：这一路终端既没有走供应商、又不是
 * Qoder 时，模型清单无从谈起——那就不出现这一项，而不是留一个永远失败的下拉。
 */

/** 一条可选项。`label` 只用于显示：供应商可能给了名字，CLI 只给编号。 */
type ModelOption = { id: string; label: string };

/** 清单的加载状态。`idle` 表示还没取过（首次展开才取）。 */
type LoadState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "ready"; models: ModelOption[] }
  | { status: "failed" };

export function ModelSelect({
  terminalId,
  providerId,
  vendorId,
  targetId,
  onSent,
  onError
}: {
  /** 选中后往这一路终端写 `/model <id>`。 */
  terminalId: string;
  providerId: string;
  vendorId?: string;
  targetId?: string;
  /** 已经写进终端（参数就是那一条命令）：父组件用它把焦点还给终端。 */
  onSent?: (command: string) => void;
  onError: (message: string) => void;
}) {
  const source = resolveModelSource({ providerId, vendorId, targetId });
  /**
   * 来源的**身份**。用它而不是 `source` 本身：`resolveModelSource` 每次渲染都返回
   * 新对象，按引用比较等于「每渲染一次就复位一次」。
   */
  const sourceKey = source
    ? `${source.kind}:${source.kind === "vendor" ? source.vendorId : source.targetId}`
    : "";
  const [state, setState] = useState<LoadState>({ status: "idle" });
  /** 选中的模型编号。它也是下拉里显示的那一项。 */
  const [selected, setSelected] = useState("");
  const appliedSourceRef = useRef(sourceKey);

  /**
   * 来源变了就把状态清掉重来。
   *
   * 最要紧的一种：**供应商被自动故障转移换掉了**（`gateway:vendor-switched`，
   * 全程没有用户操作）。不复位的话，下拉里列的还是上一家的模型，用户选一个——
   * 那一条 `/model` 会发给一家已经不再服务这一路的供应商。
   */
  useEffect(() => {
    if (appliedSourceRef.current === sourceKey) return;
    appliedSourceRef.current = sourceKey;
    setState({ status: "idle" });
    setSelected("");
  }, [sourceKey]);

  const load = useCallback(async () => {
    if (!source) return;
    setState({ status: "loading" });
    try {
      const models: ModelOption[] =
        source.kind === "vendor"
          ? (await api.listVendorModels(source.vendorId)).map((model) => ({
              id: model.id,
              label: model.name ?? model.id
            }))
          : (await api.listModels(source.targetId)).map((model) => ({
              id: model.id,
              label: model.id
            }));
      setState({ status: "ready", models });
    } catch {
      /*
        失败**不往上抛**：这只是输入区里的一个小控件，弹一条全局错误会盖住用户正在
        看的东西；而他能做的补救动作（再展开一次）就写在下拉里。
      */
      setState({ status: "failed" });
    }
  }, [source]);

  const expand = useCallback(() => {
    // 已经取到就不再取：清单在一次会话里不会变，重复取只让用户多等一次。
    // 失败则重来一次——「重新展开即重试」。
    if (state.status === "idle" || state.status === "failed") void load();
  }, [state.status, load]);

  const pick = useCallback(
    async (id: string) => {
      setSelected(id);
      try {
        await api.writeTerminal(terminalId, `/model ${id}\r`);
        onSent?.(`/model ${id}`);
      } catch (problem) {
        /*
          写失败就把选中项**复位**。

          不复位的话下拉会停在用户刚选的那一项上（看起来像切成功了），而且它与
          `selected` 同值——再选一次不会触发 `onChange`，用户**再也切不过去**，
          除非挑一个别的模型再挑回来。
        */
        setSelected("");
        onError(problem instanceof Error ? problem.message : String(problem));
      }
    },
    [terminalId, onSent, onError]
  );

  if (!source) return null;
  // 取回来是空的与「没有来源」是一回事：列不出模型就没有可选的东西。
  if (state.status === "ready" && state.models.length === 0) return null;

  const placeholder =
    state.status === "loading"
      ? "加载中…"
      : state.status === "failed"
        ? "无法获取模型"
        : "选择模型";

  return (
    <select
      className="model-select"
      aria-label="选择模型"
      title="选择模型：选中后立即生效（向 CLI 写 /model <id>）"
      value={selected}
      /*
        `pointerdown` / `focus` 就是「用户正要打开它」的最早信号，见文件头注释。
        只认**主键**（`button === 0`）：右键是「看看有什么选项」的前置动作、中键是
        粘贴，两者都不该触发取数——Qoder 那一路的取数会真起一次 CLI 进程（最长 20 秒）。
      */
      onPointerDown={(event) => {
        if (event.button === 0) expand();
      }}
      onFocus={expand}
      onChange={(event) => {
        const id = event.target.value;
        if (id) void pick(id);
      }}
    >
      <option value="">{placeholder}</option>
      {state.status === "ready" &&
        state.models.map((model) => (
          <option key={model.id} value={model.id}>
            {model.label}
          </option>
        ))}
    </select>
  );
}
