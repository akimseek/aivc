import { describe, expect, it } from "vitest";

import type { AiProviderSummary } from "../types";
import {
  supportsBatchActions,
  supportsBranch,
  supportsCustomCwd,
  supportsDuplicate,
  supportsExport,
  supportsSessionSettings,
  supportsSkills,
  supportsTrash
} from "./provider-capabilities";

/**
 * 能力查询函数。
 *
 * # 为什么这些断言是**判别性**的
 *
 * `capabilities` 在网络上是 `Record<string, boolean>`：少一个键、键名抄错一个字母，
 * 两边都能编译，只有在运行时表现为「入口在、点得动、必然报错」。因此这里
 *
 * 1. 用与后端 `ProviderId::capabilities()` 一致的矩阵钉住四个平台的取值；
 * 2. 用**逐键不同**的探针数据钉住每个函数读的就是它自己那把键（改名/抄错必红）；
 * 3. 单独钉住两条既有约定：**缺键 = 不支持**（绝不默认放行）、平台不在列表 = 不支持。
 */

/** 与后端一致的三个样本（Codex 全真 / Claude 除 skills、sessionSettings 外真 / Qoder 只有五项真）。 */
const BACKEND: AiProviderSummary[] = [
  {
    id: "codex",
    label: "Codex",
    capabilities: {
      skills: true,
      branch: true,
      usage: true,
      trash: true,
      batchActions: true,
      customCwd: true,
      export: true,
      sessionSettings: true,
      duplicate: true,
      vendorManagement: true
    }
  },
  {
    id: "claude",
    label: "Claude Code",
    capabilities: {
      skills: false,
      branch: true,
      usage: true,
      trash: true,
      batchActions: true,
      customCwd: true,
      export: true,
      sessionSettings: false,
      duplicate: true,
      vendorManagement: true
    }
  },
  {
    id: "qoder",
    label: "Qoder CN",
    capabilities: {
      skills: false,
      branch: false,
      usage: true,
      trash: true,
      batchActions: true,
      customCwd: true,
      export: true,
      sessionSettings: false,
      duplicate: false,
      vendorManagement: false
    }
  }
];

describe("provider-capabilities：按后端矩阵判定（真 / 假两态）", () => {
  it("会话设置只有 Codex 为真——环境页 WSL 卡与「设置会话」对话框的判据", () => {
    expect(supportsSessionSettings(BACKEND, "codex")).toBe(true);
    expect(supportsSessionSettings(BACKEND, "claude")).toBe(false);
    expect(supportsSessionSettings(BACKEND, "qoder")).toBe(false);
  });

  it("技能只有 Codex 为真", () => {
    expect(supportsSkills(BACKEND, "codex")).toBe(true);
    expect(supportsSkills(BACKEND, "claude")).toBe(false);
    expect(supportsSkills(BACKEND, "qoder")).toBe(false);
  });

  it("自定义工作目录与导出当前四个平台都为真", () => {
    for (const id of ["codex", "claude", "qoder"] as const) {
      expect(supportsCustomCwd(BACKEND, id)).toBe(true);
      expect(supportsExport(BACKEND, id)).toBe(true);
    }
  });

  it("复制与分支：Qoder 为假（这两个函数的既有行为一并钉住）", () => {
    expect(supportsDuplicate(BACKEND, "codex")).toBe(true);
    expect(supportsDuplicate(BACKEND, "qoder")).toBe(false);
    expect(supportsBranch(BACKEND, "claude")).toBe(true);
    expect(supportsBranch(BACKEND, "qoder")).toBe(false);
  });
});

describe("provider-capabilities：键名与缺键约定", () => {
  it("**每个函数读的是自己那把键**——四个探针各只开一个键，读错键必红", () => {
    // 每行只有对应函数应得 true：任何「读错键」（例如 customCwd 抄成 skills）
    // 都会让矩阵里至少两格翻转。id 用真实平台，避开类型上的虚构值。
    const probes: AiProviderSummary[] = [
      { id: "codex", label: "探针", capabilities: { customCwd: true } },
      { id: "gemini", label: "探针", capabilities: { skills: true } },
      { id: "claude", label: "探针", capabilities: { sessionSettings: true } },
      { id: "qoder", label: "探针", capabilities: { export: true } }
    ];
    const matrix = probes.map((probe) => [
      supportsCustomCwd(probes, probe.id),
      supportsSkills(probes, probe.id),
      supportsSessionSettings(probes, probe.id),
      supportsExport(probes, probe.id)
    ]);
    expect(matrix).toEqual([
      [true, false, false, false],
      [false, true, false, false],
      [false, false, true, false],
      [false, false, false, true]
    ]);
  });

  it("键在但值为 false 时按 false——不是「有键就放行」", () => {
    const off: AiProviderSummary[] = [
      {
        id: "claude",
        label: "Claude Code",
        capabilities: { sessionSettings: false, customCwd: false, skills: false, export: false }
      }
    ];
    expect(supportsSessionSettings(off, "claude")).toBe(false);
    expect(supportsCustomCwd(off, "claude")).toBe(false);
    expect(supportsSkills(off, "claude")).toBe(false);
    expect(supportsExport(off, "claude")).toBe(false);
  });

  it("**缺键 = 不支持**：绝不因为少一个键就放行一个会报错的功能", () => {
    const sparse: AiProviderSummary[] = [{ id: "codex", label: "Codex", capabilities: {} }];
    expect(supportsSessionSettings(sparse, "codex")).toBe(false);
    expect(supportsCustomCwd(sparse, "codex")).toBe(false);
    expect(supportsSkills(sparse, "codex")).toBe(false);
    expect(supportsExport(sparse, "codex")).toBe(false);
  });

  it("平台不在列表（或还没选平台）时同样算不支持", () => {
    expect(supportsSessionSettings(BACKEND, "gemini")).toBe(false);
    expect(supportsCustomCwd(BACKEND, "gemini")).toBe(false);
    expect(supportsSkills(BACKEND, "gemini")).toBe(false);
    expect(supportsExport(BACKEND, "gemini")).toBe(false);
    // `app.tsx` 在没选平台时传的就是空串：那时四项能力必须全假。
    expect(supportsSessionSettings(BACKEND, "")).toBe(false);
    expect(supportsSkills(BACKEND, "")).toBe(false);
  });
});

describe("provider-capabilities：回收站与批量操作（P2-D 由 sidebar-nav.test 迁入）", () => {
  it("按后端矩阵：四个平台的回收站与批量操作都为真", () => {
    // 与 `crates/aivc-core/src/target.rs` 的 `ProviderId::capabilities()` 一致——
    // 那里也有 Rust 侧断言（qoder.trash && qoder.batch_actions）。
    for (const provider of BACKEND) {
      expect(supportsTrash(BACKEND, provider.id)).toBe(true);
      expect(supportsBatchActions(BACKEND, provider.id)).toBe(true);
    }
  });

  it("**缺键 = 不支持**——不能因为少一个键就放行一个会报错的功能", () => {
    const sparse: AiProviderSummary[] = [{ id: "codex", label: "Codex", capabilities: {} }];
    expect(supportsTrash(sparse, "codex")).toBe(false);
    expect(supportsBatchActions(sparse, "codex")).toBe(false);
  });

  it("平台不在列表（或还没选平台）时同样算不支持", () => {
    expect(supportsTrash(BACKEND, "gemini:missing")).toBe(false);
    expect(supportsBatchActions(BACKEND, "gemini:missing")).toBe(false);
    expect(supportsTrash(BACKEND, "")).toBe(false);
    expect(supportsBatchActions(BACKEND, "")).toBe(false);
  });
});
