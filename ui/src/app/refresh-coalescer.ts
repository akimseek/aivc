/**
 * 把短时间内的多次「该刷新了」并成一次。
 *
 * # 为什么需要它
 *
 * 后端每转发一个请求就推一条 `gateway:request-recorded`。一个终端在重试循环里
 * 一秒能发好几条，而**每次刷新要拉三个接口**（健康、用量、日志）。
 * 不合并的话，一次终端重试就能把数据库打满——而面板上的数字并不会因此更准。
 *
 * 因此攒一个很短的窗口：窗口内的第一条安排好刷新，其余的直接丢掉。
 *
 * # 为什么单独成一个模块
 *
 * 它是这段接线里**唯一有逻辑**的部分，抽出来就能脱离 React 穷举——
 * 「连续三次只刷一次」「窗口过去之后还能再刷」「清理之后不再刷」这三条
 * 都不该靠肉眼看界面来确认。
 */
export type RefreshCoalescer = {
  /** 请求一次刷新；窗口内重复调用只会有一次真正执行。 */
  schedule: () => void;
  /** 取消尚未执行的那一次（组件卸载时用）。 */
  cancel: () => void;
};

/**
 * 构造一个合并器。
 *
 * `delayMs` 是合并窗口；`run` 是真正要做的刷新。
 */
export function createRefreshCoalescer(delayMs: number, run: () => void): RefreshCoalescer {
  let timer: ReturnType<typeof setTimeout> | undefined;

  return {
    schedule: () => {
      // 窗口内已经有一次待执行——丢掉这一条，而不是把它排到更后面。
      // 排到后面会让连续的请求把刷新**无限推迟**，面板反而永远不更新。
      if (timer !== undefined) return;
      timer = setTimeout(() => {
        timer = undefined;
        run();
      }, delayMs);
    },
    cancel: () => {
      if (timer === undefined) return;
      clearTimeout(timer);
      timer = undefined;
    }
  };
}
