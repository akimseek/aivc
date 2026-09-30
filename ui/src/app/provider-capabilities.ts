import type { AiProviderSummary } from "../types";

/**
 * 平台能力查询。
 *
 * # 为什么要有这一层
 *
 * 四个平台的能力并不一样，而且**差异是后端说了算**的
 * （`ProviderId::capabilities()`）。界面若把某一项当成通用的，
 * 那个平台上的用户就会看到一个**点下去必然报错**的按钮——
 * 按钮在、点得动、回来一句「Qoder CN 不支持复制」。
 *
 * 实测的能力差异（`ai:list-providers` 的真实返回）：
 *
 * | 能力 | Codex | Gemini / Claude | Qoder |
 * | --- | --- | --- | --- |
 * | `skills` | ✅ | ❌ | ❌ |
 * | `branch` | ✅ | ✅ | ❌ |
 * | `usage` | ✅ | ✅ | ✅ |
 * | `trash` | ✅ | ✅ | ✅ |
 * | `batchActions` | ✅ | ✅ | ✅ |
 * | `customCwd` | ✅ | ✅ | ✅ |
 * | `export` | ✅ | ✅ | ✅ |
 * | `sessionSettings` | ✅ | ❌ | ❌ |
 * | `duplicate` | ✅ | ✅ | ❌ |
 * | `vendorManagement` | ✅ | ✅ | ❌ |
 *
 * # 缺字段按**不支持**处理
 *
 * `capabilities` 在网络上是 `Record<string, boolean>`。少一个键时若默认放行，
 * 就会给出一个必然失败的功能；默认禁止则最多是少一个入口——
 * 两害相权，取后者。
 */
function capability(
  providers: AiProviderSummary[],
  providerId: string,
  key: string
): boolean {
  return providers.find((item) => item.id === providerId)?.capabilities[key] === true;
}

/** 支不支持回收站（删除 / 恢复 / 彻底删除）。 */
export function supportsTrash(providers: AiProviderSummary[], providerId: string): boolean {
  return capability(providers, providerId, "trash");
}

/** 支不支持批量操作。 */
export function supportsBatchActions(
  providers: AiProviderSummary[],
  providerId: string
): boolean {
  return capability(providers, providerId, "batchActions");
}

/** 支不支持复制会话。 */
export function supportsDuplicate(
  providers: AiProviderSummary[],
  providerId: string
): boolean {
  return capability(providers, providerId, "duplicate");
}

/** 支不支持「从此处分支」。 */
export function supportsBranch(providers: AiProviderSummary[], providerId: string): boolean {
  return capability(providers, providerId, "branch");
}

/**
 * 支不支持会话设置（WSL 的 `CODEX_HOME` 覆盖）。
 *
 * 只有 Codex 为真。**环境页的「WSL 会话目录」卡与「设置会话」对话框都吃它**：
 * 对 claude/qoder 放行的话，保存写的是 Codex 的全局设置
 * （`codex:set-wsl-codex-home`），点了必然失败。
 */
export function supportsSessionSettings(
  providers: AiProviderSummary[],
  providerId: string
): boolean {
  return capability(providers, providerId, "sessionSettings");
}

/** 支不支持自定义工作目录（新会话对话框里的「设置工作目录」一行）。 */
export function supportsCustomCwd(providers: AiProviderSummary[], providerId: string): boolean {
  return capability(providers, providerId, "customCwd");
}

/** 支不支持平台技能（Skills）管理。 */
export function supportsSkills(providers: AiProviderSummary[], providerId: string): boolean {
  return capability(providers, providerId, "skills");
}

/** 支不支持导出会话。 */
export function supportsExport(providers: AiProviderSummary[], providerId: string): boolean {
  return capability(providers, providerId, "export");
}
