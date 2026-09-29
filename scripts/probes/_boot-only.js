/**
 * 空探针：只等应用起来就返回。
 *
 * 用途是让应用**自己把运行时目录建出来**（`data/aivc.db` 与迁移）。
 * 造测试数据时必须先有库，而库的表结构由应用自己的迁移保证——
 * 手写建表语句就等于把 schema 抄了第二份，抄错一处就再也对不上。
 */
(async () => {
  const p = window.__p;
  // 等到目标列表出现，说明应用已经把库读起来、迁移也跑完了。
  const select = await p.waitForTargets();
  return {
    ready: Boolean(select),
    targetOptions: select ? [...select.options].map((item) => item.value) : []
  };
})();
