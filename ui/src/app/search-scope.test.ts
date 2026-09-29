import { describe, expect, it } from "vitest";

import type { AiProviderId, AiTarget } from "../types";
import { scopeTargets } from "./search-scope";

/**
 * 全局检索的「范围收窄」。
 *
 * # 为什么要单独一个纯函数
 *
 * 收窄范围的价值一半在于**少发请求**：全量搜完再在前端过滤掉，那些请求已经付过代价了。
 * 因此这个判断不能写在渲染里，得能单测。
 *
 * # 这一组是为什么来的
 *
 * 旧平台 `global-session-search-panel.tsx` 有**两个**下拉（按平台筛选 + 按目标筛选），
 * 我此前只有「目标」一个——于是「只看 Codex 的」在界面上做不到，
 * 而一个平台常常有多个目标（本机 + 各个 WSL 发行版），从目标列表里一个个认平台并不现实。
 *
 * 注意这里**导入的是产品代码里那一个函数**，不是在测试里再写一遍：
 * 抄一遍的话，将来 `use-console` 改了筛选规则而这里没改，测试照样全绿。
 */

const target = (id: string, provider: AiProviderId): AiTarget =>
  ({ id, provider, label: id, kind: "local", available: true }) as AiTarget;

const TARGETS = [
  target("codex:local", "codex"),
  target("codex:wsl:Ubuntu", "codex"),
  target("claude:local", "claude"),
  target("gemini:local", "gemini")
];

describe("scopeTargets", () => {
  it("两个都空 = 全部（一个请求都不省，但这是用户选的）", () => {
    expect(scopeTargets(TARGETS, "", "")).toHaveLength(4);
  });

  it("只按平台收窄", () => {
    expect(scopeTargets(TARGETS, "codex", "").map((item) => item.id)).toEqual([
      "codex:local",
      "codex:wsl:Ubuntu"
    ]);
  });

  it("只按目标收窄", () => {
    expect(scopeTargets(TARGETS, "", "claude:local").map((item) => item.id)).toEqual([
      "claude:local"
    ]);
  });

  it("两个一起收窄", () => {
    expect(scopeTargets(TARGETS, "codex", "codex:wsl:Ubuntu").map((item) => item.id)).toEqual([
      "codex:wsl:Ubuntu"
    ]);
  });

  it("**平台与目标冲突时结果是空**，而不是忽略其中一个", () => {
    // 用户选了 claude 又留着 codex 的目标：宁可搜出 0 条（他能看出筛选条件矛盾），
    // 也不要悄悄忽略平台筛选、给出别家平台的结果。
    expect(scopeTargets(TARGETS, "claude", "codex:local")).toEqual([]);
  });

  it("不看编号前缀判断平台，用的是 target.provider 字段", () => {
    // 靠解析 `codex:wsl:Ubuntu` 这种字符串来认平台，会在编号格式变化时静默失效。
    const odd = [target("weird-id-without-prefix", "codex")];
    expect(scopeTargets(odd, "codex", "")).toHaveLength(1);
    expect(scopeTargets(odd, "claude", "")).toHaveLength(0);
  });
});
