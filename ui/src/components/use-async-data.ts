import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { errorText } from "./error-text";

/**
 * 一次异步操作的**加载 / 错误 / 重试**骨架。
 *
 * # 为什么值得单独抽
 *
 * 配置面里每一个面板都重复着同一套动作：
 *
 * ```text
 * 把 loading 置 true → 调 IPC → 成功就 setData → 失败就报错 → loading 置 false
 * ```
 *
 * 它写在五个面板里就有五种走样方式：忘了清 loading（转圈永远不停）、
 * 失败后没清旧数据（界面显示一份**看起来成功**的旧结果）、
 * 并发两次请求后**先发的后回**把新结果盖掉。
 *
 * 这里把这三件事一次做对：
 *
 * 1. **失败时清空数据**——留着旧数据会让人以为这次成功；这是最误导的一种失败；
 * 2. **只在最新一次请求落地**（自增序号），与 `use-console` 里搜索那条竞态同一手法；
 * 3. **卸载后不再 setState**，避免「组件已卸载还在写状态」。
 *
 * `run` 返回一个 `reload`，供「重新检测」这类按钮直接复用。
 *
 * # 契约：`load` 的身份必须跟着**查询输入**走
 *
 * 「输入换了要重新取数」靠的是 effect 依赖里的 `load`。因此调用方必须用
 * `useCallback` 包住它，且**依赖数组里恰好列出这次查询的全部输入**
 * （时间区间、筛选条件、页码……）。这是修法与死循环的分界线：
 *
 * - **少列一个输入**：那个输入变了不会重查——界面摆着旧区间的数字，
 *   而新选中的区间就在旁边，看起来像后端返回了错的数据；
 * - **多列一个每次渲染都换身份的值**（渲染期新建的对象 / 函数、`new Date()`
 *   算出来的字符串）：effect 每渲染重查一次，请求 → setState → 重渲染 → 再请求，
 *   直接转成死循环。
 *
 * 推论：**「此刻」这类要到调用时才该定下来的值放在 `load` 内部现算**，
 * 不要算在渲染期再进依赖（「近 7 天」的右端就是这种值）。
 */
export function useAsyncData<T>(
  load: () => Promise<T>,
  options: { errorMessage?: string; auto?: boolean } = {}
): {
  data: T | null;
  loading: boolean;
  error: string;
  reload: () => Promise<void>;
} {
  const { errorMessage = "读取失败", auto = true } = options;
  const [data, setData] = useState<T | null>(null);
  const [loading, setLoading] = useState(auto);
  const [error, setError] = useState("");

  /**
   * 自增请求号。
   *
   * 用 ref 而不是 state：它只用于**判断哪一次该落地**，不参与渲染。
   * 放进 state 会让每次请求都触发一次无意义的重渲染。
   */
  const seqRef = useRef(0);
  const mountedRef = useRef(true);
  /** `load` 每次渲染都是新的函数，用 ref 固定住身份，避免它把 effect 拖成死循环。 */
  const loadRef = useRef(load);
  loadRef.current = load;

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const reload = useCallback(async () => {
    const seq = ++seqRef.current;
    setLoading(true);
    try {
      const fresh = await loadRef.current();
      if (!mountedRef.current || seq !== seqRef.current) return;
      setData(fresh);
      setError("");
    } catch (problem) {
      if (!mountedRef.current || seq !== seqRef.current) return;
      setData(null);
      setError(errorText(problem));
    } finally {
      if (mountedRef.current && seq === seqRef.current) setLoading(false);
    }
  }, []);

  /**
   * `load` 进依赖**不是**为了在 effect 里读它（取数统一走 `reload`，那里面读的是
   * `loadRef`），而是把「查询输入换了」变成一个可比较的依赖：换了身份就重查一次。
   * 调用方因此必须保证它是稳定的（见文件头的契约）。
   */
  useEffect(() => {
    if (!auto) return;
    void reload();
  }, [auto, reload, load]);

  /** 把「有错就说这句话」与调用方的兜底文案合起来，省得每个面板各写一次。 */
  const message = useMemo(
    () => (error ? `${errorMessage}：${error}` : ""),
    [error, errorMessage]
  );

  return { data, loading, error: message, reload };
}

/**
 * 一句话的操作结果提示（toast）。
 *
 * # 与 `Notice` 的分工
 *
 * `Notice` 是**常驻**的告警条：它说明一个问题，而且带出口（「重新探测」）。
 * 这个 hook 管的是**一次性**结果：「已清理 12 条日志」——说完就该自己消失。
 *
 * 旧平台用 `notice-toast.tsx` 做同一件事。区别是这里**必须带计时器清理**：
 * 卸载后还在跑 `setTimeout` 会在下一次挂载时冒出一条旧提示。
 */
export function useToast(durationMs = 4000): {
  message: string;
  show: (message: string) => void;
  dismiss: () => void;
} {
  const [message, setMessage] = useState("");
  const timerRef = useRef<number | undefined>(undefined);

  const clear = useCallback(() => {
    if (timerRef.current !== undefined) {
      window.clearTimeout(timerRef.current);
      timerRef.current = undefined;
    }
  }, []);

  const show = useCallback(
    (next: string) => {
      clear();
      setMessage(next);
      if (next) {
        timerRef.current = window.setTimeout(() => {
          setMessage("");
          timerRef.current = undefined;
        }, durationMs);
      }
    },
    [clear, durationMs]
  );

  const dismiss = useCallback(() => {
    clear();
    setMessage("");
  }, [clear]);

  useEffect(() => clear, [clear]);

  return { message, show, dismiss };
}
