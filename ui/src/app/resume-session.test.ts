import { describe, expect, it, vi } from "vitest";

import { planResume } from "./resume-session";

/**
 * 「恢复历史会话」的四条分支。
 *
 * 这个决策最要紧的一条是**第三、四条**：判断不出来时不能宣布目录丢了 ——
 * 否则 WSL 目标上的用户会看到「原工作目录不存在」，而那个目录其实好好的。
 */
describe("planResume", () => {
  it("没有记录工作目录时直接进终端", async () => {
    const probe = vi.fn(async () => false);
    expect(await planResume(undefined, probe)).toEqual({ kind: "terminal" });
    // 没什么可检查的，就不该去问。
    expect(probe).not.toHaveBeenCalled();
  });

  it("原目录还在时直接进终端，且**不弹对话框**", async () => {
    // 旧平台只在目录丢了的时候才弹 `设置工作目录`；正常恢复是直接进的。
    const plan = await planResume("D:\\work", async () => true);
    expect(plan).toEqual({ kind: "terminal", cwd: "D:\\work" });
  });

  it("原目录不在时先弹对话框", async () => {
    const plan = await planResume("D:\\gone", async () => false);
    expect(plan).toEqual({ kind: "dialog", cwd: "D:\\gone", missingCwd: "D:\\gone" });
  });

  it("**判断不出来时按「还在」处理**，不宣布目录丢了", async () => {
    // WSL 目标上 `shell:path-exists` 的支线尚未接线，后端**显式拒绝**
    // （`crates/aivc-ipc/src/shell.rs` 的 `require_local`，错误原文如下）。
    // 把它当成「不存在」会让用户在 WSL 上永远看到「原工作目录不存在」。
    const failing = async () => {
      throw new Error("暂不支持 WSL 目标的路径操作。");
    };
    expect(await planResume("\\\\wsl$\\Ubuntu\\home\\me", failing)).toEqual({
      kind: "terminal",
      cwd: "\\\\wsl$\\Ubuntu\\home\\me"
    });
  });

  it("目录要原样带着：Qoder 靠项目目录定位 --resume 的历史文件", async () => {
    const plan = await planResume("/home/me/proj", async () => true);
    expect(plan.kind === "terminal" && plan.cwd).toBe("/home/me/proj");
  });
});
