/**
 * 把任意 `unknown` 错误折成一句人话。
 *
 * # 为什么要收敛到一处
 *
 * 「`instanceof Error` 取 `message`，否则 `String()`」这个折叠此前在全仓抄了
 * 五十多遍——漏一处就会把 `[object Object]` 直接糊到界面上。统一从这里出：
 * 将来要加形态（陌生对象取 `message` 字段、`IpcError` 附带命令名）只改一处。
 *
 * 唯一的例外在 `ipc/client.ts`：它是最底层（不得反向依赖 components），
 * 而且那里是**重新包装**错误构造 `IpcError`，不是展示。
 */
export function errorText(problem: unknown): string {
  return problem instanceof Error ? problem.message : String(problem);
}
