/**
 * 恢复历史会话前先决定：**直接进终端**，还是**先让用户重设工作目录**。
 *
 * # 与旧平台一致的三条分支
 *
 * | 情况 | 行为 |
 * | --- | --- |
 * | 没有记录工作目录 | 直接进终端（没什么可检查的） |
 * | 原目录还在 | 直接进终端，带 `--resume`，**不弹对话框** |
 * | 原目录已不在 | 弹 `设置工作目录` 形态，按钮是 `继续打开` |
 * | **判断不出来**（例如 WSL 目标上那条检查未接线） | 按「还在」处理 |
 *
 * # 最后一条为什么这样定
 *
 * 后端在 WSL 目标上**显式报错**而不是悄悄按本机路径判断
 * （`crates/aivc-ipc/src/shell.rs` 的模块注释），因为那会把「WSL 里有」误判成「没有」。
 * 调用方如果把手误当成「不存在」，用户会看到「原工作目录不存在」而那个目录其实好好的——
 * **不知道就不宣布它丢了**。
 *
 * # 为什么抽出来
 *
 * 它是这段流程里**唯一有逻辑**的部分。留在 `app.tsx` 里就只能靠渲染整个应用来验证，
 * 而这里有四条分支，值得逐条钉住。
 */
export type ResumePlan =
  /** 直接起终端。`cwd` 有值时要带上：Qoder 靠**项目目录**定位 `--resume` 的历史文件。 */
  | { kind: "terminal"; cwd?: string }
  /** 先弹对话框让用户重设目录。 */
  | { kind: "dialog"; cwd: string; missingCwd: string };

export async function planResume(
  cwd: string | undefined,
  pathExists: (path: string) => Promise<boolean>
): Promise<ResumePlan> {
  if (!cwd) return { kind: "terminal" };
  try {
    if (await pathExists(cwd)) return { kind: "terminal", cwd };
    return { kind: "dialog", cwd, missingCwd: cwd };
  } catch {
    return { kind: "terminal", cwd };
  }
}
