# aivc-rs 界面重做 · 批次一 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 按 `docs/ui-redesign-demo.html`（第九版）重做 aivc-rs 前端的应用骨架与主界面：双主题设计系统、TitleBar/ActivityBar/两侧栏/状态栏新壳、统一会话标签模型、标签栏+画布+输入区、消息与分支弹框、设置中心外壳（现有面板挂入），并移除旧菜单栏完成入口迁移。

**Architecture:** 就地分层重构（不重写、不并存）。设计系统（token 双主题 + 新基础组件）先行；纯逻辑（标签模型、主题解析、设置中心导航）与 UI 分离并可单测；新组件按 `shell/` 与 `features/` 目录组织，现有状态钩子（`use-console` / `use-terminals`）保留并做最小必要扩展。每个任务结束时应用完整可用。

**Tech Stack:** React 19 + TypeScript 5.9 + Vite 7 + Vitest 4（jsdom/node 双环境）+ 原生 CSS（token 变量，无 Tailwind 无 CSS-in-JS）。

**Spec:** `D:\code\ai\docs\superpowers\specs\2026-09-30-aivc-rs-ui-redesign-design.md`（在旧平台仓库；本计划在 `aivc-rs` 仓库执行）

## Global Constraints

- 工程根：`D:\code\ai\aivc-rs`；前端目录 `ui/`。所有 npm 命令在 `ui/` 下执行。
- 依赖零新增：不装任何 UI/图标/字体库。图标用内联 SVG（`components/icon.tsx`）。
- 唯一强调色 `--ui-accent` 只用于「选中 / 主操作 / 焦点环」；等待确认用 `--ui-warn`，错误用 `--ui-danger`；金色收藏星 `#d9a11a` 是刻意保留的第二色相。
- 组件不写死颜色/字号/间距：一律引用 `styles/tokens.css` 的 token（现有代码中的硬编码值随改动换成 token）。
- 代码注释用中文，只解释「为什么」；不写「做了什么」。
- 每条命令必须在前 `cd` 到目标目录；提交信息用中文、`feat:`/`refactor:`/`fix:`/`test:` 前缀，与仓库现有风格一致。
- 门禁（每任务至少跑受影响的）：`npm run typecheck`；`npm test -- <文件>`；任务收尾跑全量 `npm run typecheck && npm test && npm run build`。
- UI 验证脚本（PowerShell，从 `aivc-rs/` 根执行）：`pwsh scripts/ui-check.ps1 -Name <名>`（启动应用+CDP 截图）、`pwsh scripts/ui-baseline.ps1 -Name <名>`（视图基线集）。截图落在 `shots/<名>/`。
- Rust 后端不改。若发现必须的后端缺口：停止并在报告里说明，不擅自修改 `crates/`。
- 已知缺陷 B1（`ui/src/ipc/commands.ts:150` 调 `codex:open-session-folder`，后端注册名是 `shell:open-session-folder`）在 Task 5 中修复。

## 任务总览（执行顺序即依赖顺序）

| # | 任务 | 主要产物 | 依赖 |
| :-: | --- | --- | --- |
| 1 | 双主题 token 与主题机制 | `theme.ts` / `use-theme.ts` / `tokens.css` 重写 / `index.html` 引导 | — |
| 2 | 设计系统组件扩展 | `icon.tsx` / `badge.tsx` / `status-dot.tsx` / `progress.tsx` / `toggle.tsx` / `popover.tsx` / Button·Dialog 扩展 | — |
| 3 | 统一标签模型（纯逻辑） | `workspace-tabs.ts` v2 + 测试 | — |
| 4 | 标签 hook 与 app 接线 | `use-workspace-tabs.ts` v2 / `app.tsx` / `session-workspace.tsx` 适配 | 3 |
| 5 | 设置中心外壳 | `settings-center.tsx` / `settings-nav.ts` / 关于面板 / 环境面板 WSL 卡 / 修 B1 | 2 |
| 6 | 应用新壳 + 移除菜单栏 | `shell/*` / 状态栏重做 / 输入模式上收 / 删菜单三文件 | 1,2,5 |
| 7 | 会话侧栏重做 | `features/sessions/*`（卡片/筛选/搜索/批量/摘要） | 2,6 |
| 8 | 工作台侧栏重做 | `features/workbench/*`（五卡） | 2,6 |
| 9 | 标签栏与画布重做 | `features/workspace/tab-bar.tsx` / `stage/*` | 3,4,6 |
| 10 | 输入区重做 + 模型选择 | `features/workspace/composer/*` | 2,9 |
| 11 | 消息与分支弹框 | `features/workspace/messages-modal.tsx` | 2,6 |
| 12 | 批次一收尾 | 门禁全绿 + 明暗基线截图 + 文档记录 | 全部 |

**执行顺序说明：** Task 5 必须先于 Task 6 —— 菜单栏的动作（退出 / 打开日志目录 / 设置会话 / 关于）要有新家之后才能删菜单。批次二（设置中心 10 面板重排 + 其余弹框统一）与批次三（暗色走查 / 清理 / 基线）的纲要见文末，它们各自的完整计划在批次一开始时另写。

---

### Task 1: 双主题 token 与主题机制

**Files:**
- Create: `ui/src/app/theme.ts`
- Create: `ui/src/app/theme.test.ts`
- Create: `ui/src/app/use-theme.ts`
- Modify: `ui/src/styles/tokens.css`（整体重写）
- Modify: `ui/index.html`（首帧引导脚本）
- Modify: `ui/src/main.tsx`（不改导入顺序；仅在 App 内用 hook，无需改）

**Interfaces:**
- Consumes: 无。
- Produces:
  - `theme.ts`：`type ThemePreference = "system" | "light" | "dark"`；`THEME_STORAGE_KEY = "aivc.theme"`；`parseThemePreference(raw: string | null): ThemePreference`；`resolveTheme(pref: ThemePreference, systemPrefersDark: boolean): "light" | "dark"`；`nextPreference(current: ThemePreference): ThemePreference`；`themePreferenceLabel(pref: ThemePreference): string`。
  - `use-theme.ts`：`useTheme(): { preference: ThemePreference; resolved: "light" | "dark"; setPreference: (p: ThemePreference) => void }`（副作用：写 `document.documentElement.dataset.theme` 与 localStorage）。

- [ ] **Step 1: 写失败测试（纯逻辑）**

创建 `ui/src/app/theme.test.ts`：

```ts
// @vitest-environment node
import { describe, expect, it } from "vitest";

import {
  nextPreference,
  parseThemePreference,
  resolveTheme,
  themePreferenceLabel
} from "./theme";

/**
 * 主题偏好的解析规则。
 *
 * 最要紧的一条：**非法值必须回退到 system**，而不是保留坏值——
 * localStorage 是外部输入（用户可手改、旧版本可能写过别的值），
 * 留着一个不认识的字符串会让 `dataset.theme` 变成 undefined，界面配色整块失效。
 */
describe("parseThemePreference", () => {
  it("认识三个合法值", () => {
    expect(parseThemePreference("system")).toBe("system");
    expect(parseThemePreference("light")).toBe("light");
    expect(parseThemePreference("dark")).toBe("dark");
  });

  it("null 与非法值都回退到 system", () => {
    expect(parseThemePreference(null)).toBe("system");
    expect(parseThemePreference("")).toBe("system");
    expect(parseThemePreference("Dark")).toBe("system");
    expect(parseThemePreference("ｄａｒｋ")).toBe("system");
  });
});

describe("resolveTheme", () => {
  it("明确偏好优先于系统", () => {
    expect(resolveTheme("light", true)).toBe("light");
    expect(resolveTheme("dark", false)).toBe("dark");
  });

  it("system 跟随系统", () => {
    expect(resolveTheme("system", true)).toBe("dark");
    expect(resolveTheme("system", false)).toBe("light");
  });
});

describe("nextPreference", () => {
  it("按 明亮→暗黑→跟随系统 循环", () => {
    expect(nextPreference("light")).toBe("dark");
    expect(nextPreference("dark")).toBe("system");
    expect(nextPreference("system")).toBe("light");
  });
});

describe("themePreferenceLabel", () => {
  it("每个偏好都有可读文案", () => {
    expect(themePreferenceLabel("light")).toBe("明亮");
    expect(themePreferenceLabel("dark")).toBe("暗黑");
    expect(themePreferenceLabel("system")).toBe("跟随系统");
  });
});
```

- [ ] **Step 2: 运行测试，确认失败**

```bash
cd ui && npm test -- src/app/theme.test.ts
```

预期：FAIL，报 `Cannot find module './theme'`（或等价的解析错误）。

- [ ] **Step 3: 实现 theme.ts**

创建 `ui/src/app/theme.ts`：

```ts
/**
 * 主题偏好的纯逻辑。
 *
 * 与读写的分工：**这里只做纯函数**（解析、解析结果、循环顺序、文案），
 * localStorage 与 matchMedia 的读写都在 [`useTheme`](./use-theme.ts) 与
 * `index.html` 的引导脚本里——纯函数才好逐条断言。
 */

/** 用户偏好。`system` 表示跟随操作系统。 */
export type ThemePreference = "system" | "light" | "dark";

/** 解析后的实际主题。 */
export type ResolvedTheme = "light" | "dark";

export const THEME_STORAGE_KEY = "aivc.theme";

/** 非法值回退 `system`：localStorage 是外部输入，坏值会让 dataset.theme 变成垃圾。 */
export function parseThemePreference(raw: string | null): ThemePreference {
  return raw === "light" || raw === "dark" || raw === "system" ? raw : "system";
}

export function resolveTheme(pref: ThemePreference, systemPrefersDark: boolean): ResolvedTheme {
  if (pref === "system") return systemPrefersDark ? "dark" : "light";
  return pref;
}

/** 切换顺序：明亮 → 暗黑 → 跟随系统 → …（两态切换器表达不了「跟随系统」，故三态循环）。 */
export function nextPreference(current: ThemePreference): ThemePreference {
  if (current === "light") return "dark";
  if (current === "dark") return "system";
  return "light";
}

export function themePreferenceLabel(pref: ThemePreference): string {
  if (pref === "light") return "明亮";
  if (pref === "dark") return "暗黑";
  return "跟随系统";
}
```

- [ ] **Step 4: 运行测试，确认通过**

```bash
cd ui && npm test -- src/app/theme.test.ts
```

预期：PASS（4 个 describe、8 个 it 全绿）。

- [ ] **Step 5: 实现 use-theme.ts**

创建 `ui/src/app/use-theme.ts`：

```ts
import { useCallback, useEffect, useState } from "react";

import {
  parseThemePreference,
  resolveTheme,
  THEME_STORAGE_KEY,
  type ResolvedTheme,
  type ThemePreference
} from "./theme";

const DARK_QUERY = "(prefers-color-scheme: dark)";

/**
 * 主题状态：偏好（持久化）+ 解析结果（写到 `<html data-theme>`）。
 *
 * `dataset.theme` 由这里与 `index.html` 的引导脚本共同写：引导脚本负责**首帧**
 * （避免暗色用户闪白），这里负责**之后**每次变化。两处都调 `parseThemePreference`
 * 的判据保持一致，改一处必须改另一处。
 */
export function useTheme(): {
  preference: ThemePreference;
  resolved: ResolvedTheme;
  setPreference: (preference: ThemePreference) => void;
} {
  const [preference, setPreferenceState] = useState<ThemePreference>(() => {
    // 读的是引导脚本已经写好的值：能在首帧就与画面一致，不会闪一下。
    const fromAttr = document.documentElement.dataset.themePreference;
    return parseThemePreference(fromAttr ?? window.localStorage.getItem(THEME_STORAGE_KEY));
  });
  const [systemDark, setSystemDark] = useState(() => window.matchMedia(DARK_QUERY).matches);

  useEffect(() => {
    const media = window.matchMedia(DARK_QUERY);
    const onChange = (event: MediaQueryListEvent) => setSystemDark(event.matches);
    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
  }, []);

  const resolved = resolveTheme(preference, systemDark);

  useEffect(() => {
    document.documentElement.dataset.theme = resolved;
  }, [resolved]);

  const setPreference = useCallback((next: ThemePreference) => {
    setPreferenceState(next);
    // 同时写 dataset.themePreference：引导脚本下次启动时用它，省一次 localStorage 读。
    document.documentElement.dataset.themePreference = next;
    try {
      window.localStorage.setItem(THEME_STORAGE_KEY, next);
    } catch {
      // 存储被禁用（隐私模式等）不该让切换失效——这一次会话内仍然生效。
    }
  }, []);

  return { preference, resolved, setPreference };
}
```

- [ ] **Step 6: 重写 tokens.css（双主题）**

整体替换 `ui/src/styles/tokens.css`。要点：保留原有层级（中性/强调/语义/终端/排版/间距/圆角/层级/阴影），值换成 spec §5.1 的表，加暗色覆盖块。完整内容：

```css
/*
 * 设计令牌（双主题）。
 *
 * 明亮值在 `:root`，暗黑值在 `:root[data-theme="dark"]`——**同一组名字、两套值**，
 * 组件只引用语义名，不感知主题。`data-theme` 由 `index.html` 引导脚本在首帧前写好。
 *
 * 唯一强调色只用于「当前选中 / 主操作 / 焦点环」。等待确认走 `--ui-warn`、
 * 错误走 `--ui-danger`，三者不混用。
 */

:root,
:root[data-theme="light"] {
  /* 表面 */
  --ui-bg: #f8fafc;
  --ui-bg-header: #ffffff;
  --ui-bg-activity: #f1f5f9;
  --ui-bg-sidebar: #ffffff;
  --ui-bg-card: #ffffff;
  --ui-bg-card-hover: #f1f5f9;
  --ui-bg-card-active: #e2e8f0;
  --ui-bg-input: #ffffff;
  --ui-bg-badge: #e2e8f0;
  --ui-bg-tabbar: #f1f5f9;
  --ui-bg-modal: #ffffff;
  --ui-bg-modal-nav: #f8fafc;
  --ui-bg-statusbar: #ffffff;

  /* 边框 */
  --ui-border: #e2e8f0;
  --ui-border-strong: #cbd5e1;
  --ui-divider: #e8edf3;

  /* 文字 */
  --ui-text: #0f172a;
  --ui-text-soft: #334155;
  --ui-muted: #475569;
  --ui-faint: #64748b;

  /* 强调色（唯一彩色） */
  --ui-accent: #059669;
  --ui-accent-strong: #047857;
  --ui-accent-soft: #ecfdf5;
  --ui-accent-border: #a7f3d0;
  --ui-focus: rgba(5, 150, 105, 0.25);

  /* 语义色 */
  --ui-warn: #b45309;
  --ui-warn-soft: #fffbeb;
  --ui-warn-border: #fde68a;
  --ui-danger: #dc2626;
  --ui-danger-soft: #fef2f2;
  --ui-danger-border: #fecaca;

  /* 收藏星：刻意保留的第二色相——金色在极小面积上就能表达「已收藏」，
     换成强调色会被当成「已选中」。 */
  --ui-star: #d9a11a;

  /* 终端（明亮 = 浅色终端主题） */
  --term-bg: #f8fafc;
  --term-border: #e2e8f0;
  --term-text: #1e293b;
  --term-prompt: #059669;
  --term-card-bg: #ffffff;
  --term-card-border: #cbd5e1;
  --term-cursor: #0f172a;

  /* 阴影：只给浮层用（平铺卡片靠边框与留白分层）。 */
  --ui-shadow: 0 16px 40px rgba(15, 23, 42, 0.14);
  --ui-shadow-sm: 0 2px 8px rgba(15, 23, 42, 0.08);
  --ui-shadow-xs: 0 1px 3px rgba(15, 23, 42, 0.06);

  color-scheme: light;
}

:root[data-theme="dark"] {
  --ui-bg: #0b0d13;
  --ui-bg-header: #11131c;
  --ui-bg-activity: #0d0f16;
  --ui-bg-sidebar: #131622;
  --ui-bg-card: #181b29;
  --ui-bg-card-hover: #1f2436;
  --ui-bg-card-active: #262c42;
  --ui-bg-input: #171a27;
  --ui-bg-badge: #1f2436;
  --ui-bg-tabbar: #0f121d;
  --ui-bg-modal: #151824;
  --ui-bg-modal-nav: #11131c;
  --ui-bg-statusbar: #0d0f16;

  --ui-border: #1f2537;
  --ui-border-strong: #2f3750;
  --ui-divider: #1a2032;

  --ui-text: #f8fafc;
  --ui-text-soft: #cbd5e1;
  --ui-muted: #94a3b8;
  --ui-faint: #64748b;

  --ui-accent: #10b981;
  --ui-accent-strong: #34d399;
  --ui-accent-soft: rgba(16, 185, 129, 0.15);
  --ui-accent-border: rgba(16, 185, 129, 0.35);
  --ui-focus: rgba(16, 185, 129, 0.35);

  --ui-warn: #fbbf24;
  --ui-warn-soft: rgba(251, 191, 36, 0.12);
  --ui-warn-border: rgba(251, 191, 36, 0.35);
  --ui-danger: #f87171;
  --ui-danger-soft: rgba(248, 113, 113, 0.12);
  --ui-danger-border: rgba(248, 113, 113, 0.35);
  --ui-star: #fbbf24;

  --term-bg: #0d0f17;
  --term-border: #1e2434;
  --term-text: #e2e8f0;
  --term-prompt: #10b981;
  --term-card-bg: #141824;
  --term-card-border: #232b3e;
  --term-cursor: #10b981;

  --ui-shadow: 0 16px 40px rgba(0, 0, 0, 0.5);
  --ui-shadow-sm: 0 2px 8px rgba(0, 0, 0, 0.35);
  --ui-shadow-xs: 0 1px 3px rgba(0, 0, 0, 0.3);

  color-scheme: dark;
}

:root {
  /* 排版：有意的系统栈（不内置字体、不引 CDN）。 */
  --ui-font-sans: "Segoe UI Variable Text", "Segoe UI", "Microsoft YaHei UI",
    "Microsoft YaHei", system-ui, -apple-system, sans-serif;
  --ui-font-mono: "Cascadia Mono", "Cascadia Code", Consolas, "Courier New", monospace;

  --ui-text-xs: 11px;
  --ui-text-sm: 12px;
  --ui-text-md: 13px;
  --ui-text-base: 13.5px; /* 正文与终端（demo 基准） */
  --ui-text-lg: 15px;
  --ui-text-xl: 18px;
  --ui-leading-tight: 1.35;
  --ui-leading: 1.6;

  /* 间距阶（4 的倍数） */
  --ui-space-1: 4px;
  --ui-space-2: 8px;
  --ui-space-3: 12px;
  --ui-space-4: 16px;
  --ui-space-5: 24px;
  --ui-space-6: 32px;

  /* 圆角阶：内层更紧、外层更松 */
  --ui-radius-sm: 4px;
  --ui-radius-md: 8px;
  --ui-radius-lg: 12px;
  --ui-radius-xl: 16px;

  /* 层级阶 */
  --ui-z-base: 0;
  --ui-z-sticky: 10;
  --ui-z-drawer: 40;
  --ui-z-overlay: 60;
  --ui-z-toast: 80;

  background: var(--ui-bg);
  color: var(--ui-text);
  font-family: var(--ui-font-sans);
  font-size: 14px;
  line-height: var(--ui-leading);
  -webkit-font-smoothing: antialiased;
}

/* 数字对齐：条数、token、端口、耗时成列出现，比例数字对不齐列。 */
.ui-num,
code,
kbd,
samp {
  font-variant-numeric: tabular-nums;
  font-feature-settings: "tnum";
}

.ui-mono {
  font-family: var(--ui-font-mono);
}
```

**注意（重要）：** 这次重写**删除了**以下旧名（spec B2：它们要么未被引用、要么本来就该消失）：`--ui-surface`、`--ui-surface-subtle`、`--radius`、`--radius-sm`、`--ui-bg-*` 之外旧名。执行 Step 7 前先跑一次引用扫描并按结果决定改名清单：

```bash
cd ui && grep -rno "var(--ui-[a-z-]*)" src --include=*.css | sed 's/.*var(//;s/).*//' | sort -u > /tmp/used.txt && grep -o "^  --[a-z0-9-]*" src/styles/tokens.css | sed 's/^  //' | sort -u > /tmp/defined.txt && comm -23 /tmp/used.txt /tmp/defined.txt
```

预期（改动前）会列出被引用但未定义的名字。对每个未定义名：在 `app.css` / `components.css` 里**按语义**替换为新 token（如 `--ui-surface` → `--ui-bg-card`，`--ui-surface-subtle` → `--ui-bg-tabbar` 或 `--ui-bg-activity`，`--ui-shadow-md/lg` → `--ui-shadow`，`--ui-warning` → `--ui-warn`，`--ui-surface-sunken` → `--ui-bg-activity`，`--radius` → `--ui-radius-md`，`--radius-sm` → `--ui-radius-sm`）。改完该命令必须**零输出**。这一步与 Task 2 的组件样式改动可以合并提交，但必须在本任务内做完。

- [ ] **Step 7: index.html 加首帧引导脚本**

修改 `ui/index.html`，在 `<head>` 内 `</head>` 前插入（**必须在任何样式生效前执行**）：

```html
    <script>
      // 首帧就定好主题：暗色用户不会先看到一屏白色。
      // 判据必须与 src/app/theme.ts 的 parseThemePreference 一致（改一处要改两处）。
      (function () {
        var pref = "system";
        var dark = false;
        try {
          pref = localStorage.getItem("aivc.theme") || "system";
          if (pref !== "light" && pref !== "dark" && pref !== "system") pref = "system";
          dark =
            pref === "dark" ||
            (pref === "system" &&
              window.matchMedia("(prefers-color-scheme: dark)").matches);
        } catch (error) {
          pref = "system";
          dark = false;
        }
        document.documentElement.dataset.theme = dark ? "dark" : "light";
        document.documentElement.dataset.themePreference = pref;
      })();
    </script>
```

同时把 `<title>` 改为 `AI Visual Console`（与 demo 一致）。

- [ ] **Step 8: 在 App 里接上 useTheme**

修改 `ui/src/app/app.tsx`：在 `const toast = useToast();` 之前加：

```tsx
  // 主题：把偏好写到 <html data-theme>，所有 token 随之整套切换。
  const theme = useTheme();
```

并在 `AppOverlays` 的 `state` 里传入 `themePreference: theme.preference` / `themeResolved: theme.resolved`（先把这两个字段加到 `overlay-layer.tsx` 的 `OverlaysState` 类型里，暂不渲染——Task 6 的 TitleBar 会用）。import 加 `import { useTheme } from "./use-theme";`。

- [ ] **Step 9: 门禁 + 手动看一眼两种主题**

```bash
cd ui && npm run typecheck && npm test && npm run build
pwsh scripts/ui-check.ps1 -Name t1-theme-light
```

预期：全绿；截图里界面是明亮主题（配色已换成 emerald/slate 系）。随后在浏览器 DevTools（或临时在 `index.html` 里把 `dataset.theme` 改成 `dark`）确认暗色不崩版式——本步只做粗看，逐屏走查在批次三。

- [ ] **Step 10: 提交**

```bash
cd ui && git add src/app/theme.ts src/app/theme.test.ts src/app/use-theme.ts src/styles/tokens.css src/app/app.tsx src/app/overlay-layer.tsx index.html src/app/app.css src/components/components.css
git commit -m "feat(ui): 双主题设计令牌与主题机制

明亮/暗黑两套 token（emerald+slate 调色板）、theme 纯逻辑与 useTheme、
index.html 首帧引导脚本避免闪白；删掉被引用但未定义的旧 token 引用。"
```

---

### Task 2: 设计系统组件扩展

**Files:**
- Create: `ui/src/components/icon.tsx`
- Create: `ui/src/components/icon.test.tsx`
- Create: `ui/src/components/badge.tsx`
- Create: `ui/src/components/status-dot.tsx`
- Create: `ui/src/components/progress.tsx`
- Create: `ui/src/components/toggle.tsx`
- Create: `ui/src/components/popover.tsx`
- Create: `ui/src/components/popover.test.tsx`
- Modify: `ui/src/components/primitives.tsx`（Button 变体扩展）
- Modify: `ui/src/components/dialog.tsx`（尺寸档）
- Modify: `ui/src/components/index.ts`（导出）
- Modify: `ui/src/components/components.css`（新组件样式）

**Interfaces:**
- Consumes: Task 1 的 token。
- Produces:
  - `IconName` 联合类型 + `Icon({ name, size = 16, className, title }: { name: IconName; size?: number; className?: string; title?: string })`。
  - `Badge({ tone = "neutral", mono = false, children })`，`tone: "accent" | "neutral" | "warn" | "danger"`。
  - `StatusDot({ tone = "idle", pulse = false, size = 8 })`，`tone: "ok" | "warn" | "danger" | "idle"`。
  - `ProgressBar({ value, tone = "accent", label }: { value: number; tone?: "accent" | "warn" | "danger"; label?: string })`（`value` 为 0–100，内部夹取）。
  - `Toggle({ checked, onChange, disabled, label, title })`。
  - `Popover({ open, onClose, anchorRef, children, className })`（视口夹取的锚定浮层）。
  - `Button` 的 `variant` 扩展为 `"default" | "primary" | "danger" | "ghost"`。
  - `Dialog` 增加 `size?: "sm" | "md" | "lg"`（映射到 `.dialog-panel-{size}`，默认 `md`）。

- [ ] **Step 1: 写 Icon 的失败测试**

创建 `ui/src/components/icon.test.tsx`：

```tsx
// @vitest-environment jsdom
import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Icon, ICON_NAMES } from "./icon";

/**
 * 图标注册表的完整性。
 *
 * 最要紧的一条：**名字表与路径表必须同步**——漏一个名字时 `Icon` 会渲染空 svg，
 * 界面上那一处就悄悄没有图标（这类缺陷在截图里很难注意到）。
 */
describe("Icon", () => {
  it("每个注册的名字都渲染出至少一条 path", () => {
    for (const name of ICON_NAMES) {
      const view = render(<Icon name={name} />);
      const svg = view.container.querySelector("svg");
      expect(svg, `icon ${name} 没有渲染 svg`).not.toBeNull();
      expect(svg!.querySelectorAll("path,circle,rect,line").length, `icon ${name} 是空的`).toBeGreaterThan(0);
      view.unmount();
    }
  });

  it("有 title 时 svg 带 role=img 与 aria-label，没有时不带", () => {
    const withTitle = render(<Icon name="chat" title="会话" />);
    const svg = withTitle.container.querySelector("svg")!;
    expect(svg.getAttribute("role")).toBe("img");
    expect(svg.getAttribute("aria-label")).toBe("会话");

    const without = render(<Icon name="chat" />);
    expect(without.container.querySelector("svg")!.getAttribute("aria-hidden")).toBe("true");
  });

  it("尺寸落到 width/height", () => {
    const view = render(<Icon name="close" size={20} />);
    const svg = view.container.querySelector("svg")!;
    expect(svg.getAttribute("width")).toBe("20");
    expect(svg.getAttribute("height")).toBe("20");
  });
});
```

- [ ] **Step 2: 运行，确认失败**

```bash
cd ui && npm test -- src/components/icon.test.tsx
```

预期：FAIL，`Cannot find module './icon'`。

- [ ] **Step 3: 实现 icon.tsx**

创建 `ui/src/components/icon.tsx`。采用统一 24 viewBox、`stroke=currentColor`、`fill=none`、`stroke-width=1.75`、`stroke-linecap/linejoin=round`：

```tsx
import type { ReactNode } from "react";

/**
 * 图标注册表（内联 SVG，零依赖）。
 *
 * 路径来源两类：**demo 里已经画好的**（按注释里的 demo 行号抄，形状与 demo 一致）与
 * **Heroicons v2 outline（MIT）**。两者观感统一（24 网格、1.75 线宽、圆头圆角），
 * 因此可以混用。新增图标时**只加路径、不加库**——这个应用运行时不联网，
 * 引图标库也会把每个图标都打进产物。
 */
const PATHS: Record<string, ReactNode> = {
  /** demo `docs/ui-redesign-demo.html:186` */
  chat: <path d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z" />,
  /** demo:193（柱状图） */
  chart: <path d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z" />,
  /** demo:202（齿轮） */
  settings: (
    <>
      <path d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
      <path d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
    </>
  ),
  /** demo:168（铃铛） */
  bell: <path d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9" />,
  /** demo:277（放大镜） */
  search: <path d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />,
  /** demo:481（加号） */
  plus: <path d="M12 4v16m8-8H4" />,
  /** demo:696（关闭） */
  close: <path d="M6 18L18 6M6 6l12 12" />,
  /** demo:242（下拉箭头） */
  "chevron-down": <path d="M19 9l-7 7-7-7" />,
  "chevron-right": <path d="M9 5l7 7-7 7" />,
  /** demo:290（垃圾桶） */
  trash: (
    <>
      <path d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
    </>
  ),
  /** Heroicons v2 outline · star */
  star: <path d="M11.48 3.499a.562.562 0 011.04 0l2.125 5.111a.563.563 0 00.475.345l5.518.442c.499.04.701.663.321.988l-4.204 3.602a.563.563 0 00-.182.557l1.285 5.385a.562.562 0 01-.84.61l-4.725-2.885a.563.563 0 00-.586 0L6.982 20.54a.562.562 0 01-.84-.61l1.285-5.386a.562.562 0 00-.182-.557l-4.204-3.602a.563.563 0 01.321-.988l5.518-.442a.563.563 0 00.475-.345L11.48 3.5z" />,
  /** 分支（lucide 风格自绘）：会话分支的标识 */
  branch: (
    <>
      <path d="M6 3v12" />
      <circle cx="18" cy="6" r="3" />
      <circle cx="6" cy="18" r="3" />
      <path d="M18 9a9 9 0 01-9 9" />
    </>
  ),
  /** Heroicons v2 outline · paper-clip */
  paperclip: <path d="m18.375 12.739-7.693 7.693a4.5 4.5 0 0 1-6.364-6.364l10.94-10.94A3 3 0 1 1 19.5 7.372L8.552 18.32m.009-.01-.01.01m5.699-9.941-7.81 7.81a1.5 1.5 0 0 0 2.112 2.13" />,
  /** Heroicons v2 outline · photo */
  image: (
    <>
      <path d="m2.25 15.75 5.159-5.159a2.25 2.25 0 0 1 3.182 0l5.159 5.159m-1.5-1.5 1.409-1.409a2.25 2.25 0 0 1 3.182 0l2.909 2.909m-18 3.75h16.5a1.5 1.5 0 0 0 1.5-1.5V6a1.5 1.5 0 0 0-1.5-1.5H3.75A1.5 1.5 0 0 0 2.25 6v12a1.5 1.5 0 0 0 1.5 1.5Zm10.5-11.25h.008v.008h-.008V8.25Z" />
    </>
  ),
  /** Heroicons v2 outline · document */
  file: <path d="M19.5 14.25v-2.625a3.375 3.375 0 0 0-3.375-3.375h-1.5A1.125 1.125 0 0 1 13.5 7.125v-1.5a3.375 3.375 0 0 0-3.375-3.375H8.25m2.25 0H5.625c-.621 0-1.125.504-1.125 1.125v17.25c0 .621.504 1.125 1.125 1.125h12.75c.621 0 1.125-.504 1.125-1.125V11.25a9 9 0 0 0-9-9Z" />,
  /** Heroicons v2 outline · folder-open */
  folder: <path d="M3.75 9.776c.112-.017.227-.026.344-.026h15.812c.117 0 .232.009.344.026m-16.5 0a2.25 2.25 0 0 0-1.883 2.542l.857 6a2.25 2.25 0 0 0 2.227 1.932H19.05a2.25 2.25 0 0 0 2.227-1.932l.857-6a2.25 2.25 0 0 0-1.883-2.542m-16.5 0V6A2.25 2.25 0 0 1 6 3.75h3.879a1.5 1.5 0 0 1 1.06.44l2.122 2.12a1.5 1.5 0 0 0 1.06.44H18A2.25 2.25 0 0 1 20.25 9v.776" />,
  /** Heroicons v2 outline · arrow-path */
  refresh: <path d="M16.023 9.348h4.992v-.001M2.985 19.644v-4.992m0 0h4.992m-4.993 0 3.181 3.183a8.25 8.25 0 0 0 13.803-3.7M4.031 9.865a8.25 8.25 0 0 1 13.803-3.7l3.181 3.182m0-4.991v4.99" />,
  /** Heroicons v2 outline · document-duplicate */
  copy: <path d="M15.75 17.25v3.375c0 .621-.504 1.125-1.125 1.125h-9.75a1.125 1.125 0 0 1-1.125-1.125V7.875c0-.621.504-1.125 1.125-1.125H6.75a9.06 9.06 0 0 1 1.5.124m7.5 10.376h3.375c.621 0 1.125-.504 1.125-1.125V11.25c0-4.46-3.243-8.161-7.5-8.876a9.06 9.06 0 0 0-1.5-.124H9.375c-.621 0-1.125.504-1.125 1.125v3.5m7.5 10.375H9.375a1.125 1.125 0 0 1-1.125-1.125v-9.25m12 6.625v-1.875a3.375 3.375 0 0 0-3.375-3.375h-1.5a1.125 1.125 0 0 1-1.125-1.125v-1.5a3.375 3.375 0 0 0-3.375-3.375H9.75" />,
  /** Heroicons v2 outline · exclamation-triangle */
  alert: <path d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126ZM12 15.75h.007v.008H12v-.008Z" />,
  /** Heroicons v2 outline · check */
  check: <path d="m4.5 12.75 6 6 9-13.5" />,
  /** Heroicons v2 outline · information-circle */
  info: <path d="m11.25 11.25.041-.02a.75.75 0 0 1 1.063.852l-.708 2.836a.75.75 0 0 0 1.063.853l.041-.021M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Zm-9-3.75h.008v.008H12V8.25Z" />,
  /** Heroicons v2 outline · arrow-top-right-on-square */
  external: <path d="M13.5 6H5.25A2.25 2.25 0 0 0 3 8.25v10.5A2.25 2.25 0 0 0 5.25 21h10.5A2.25 2.25 0 0 0 18 18.75V10.5m-10.5 6L21 3m0 0h-5.25M21 3v5.25" />,
  /** Heroicons v2 outline · paper-airplane */
  send: <path d="M6 12 3.269 3.125A59.769 59.769 0 0 1 21.485 12 59.768 59.768 0 0 1 3.27 20.875L5.999 12Zm0 0h7.5" />,
  /** Heroicons v2 outline · sun（demo 用 emoji，这里换 SVG） */
  sun: <path d="M12 3v2.25m6.364.386-1.591 1.591M21 12h-2.25m-.386 6.364-1.591-1.591M12 18.75V21m-4.773-4.227-1.591 1.591M5.25 12H3m4.227-4.773L5.636 5.636M15.75 12a3.75 3.75 0 1 1-7.5 0 3.75 3.75 0 0 1 7.5 0Z" />,
  /** Heroicons v2 outline · moon */
  moon: <path d="M21.752 15.002A9.72 9.72 0 0 1 18 15.75c-5.385 0-9.75-4.365-9.75-9.75 0-1.33.266-2.597.748-3.752A9.753 9.753 0 0 0 3 11.25C3 16.635 7.365 21 12.75 21a9.753 9.753 0 0 0 9.002-5.998Z" />,
  /** Heroicons v2 outline · computer-desktop（跟随系统） */
  monitor: <path d="M9 17.25v1.007a3 3 0 0 1-.879 2.122L7.5 21h9l-.621-.621A3 3 0 0 1 15 18.257V17.25m6-12V15a2.25 2.25 0 0 1-2.25 2.25H5.25A2.25 2.25 0 0 1 3 15V5.25m18 0A2.25 2.25 0 0 0 18.75 3H5.25A2.25 2.25 0 0 0 3 5.25" />,
  /** 自绘：侧栏开关（矩形 + 竖分隔线） */
  "panel-left": (
    <>
      <rect x="3.75" y="5.25" width="16.5" height="13.5" rx="1.5" />
      <path d="M9.5 5.25v13.5" />
    </>
  ),
  /** Heroicons v2 outline · globe-alt（服务与路由分组） */
  globe: <path d="M12 21a9.004 9.004 0 0 0 8.716-6.747M12 21a9.004 9.004 0 0 1-8.716-6.747M12 21c2.485 0 4.5-4.03 4.5-9S14.485 3 12 3m0 18c-2.485 0-4.5-4.03-4.5-9S9.515 3 12 3m0 0a8.997 8.997 0 0 1 7.843 4.582M12 3a8.997 8.997 0 0 0-7.843 4.582m15.686 0A11.953 11.953 0 0 1 12 10.5c-2.998 0-5.74-1.1-7.843-2.918m15.686 0A8.959 8.959 0 0 1 21 12c0 .778-.099 1.533-.284 2.253m0 0A17.919 17.919 0 0 1 12 16.5c-3.162 0-6.133-.815-8.716-2.247m0 0A9.015 9.015 0 0 1 3 12c0-1.605.42-3.113 1.157-4.418" />,
  /** 自绘：扳手（环境与扩展分组） */
  wrench: (
    <>
      <path d="M14.7 6.3a4.5 4.5 0 1 0 5.2 5.2l-11 11a2.5 2.5 0 1 1-3.5-3.5l11-11z" />
      <path d="M15.5 8.5l-3-3" />
    </>
  ),
  /** 自绘：时钟（重置倒计时） */
  clock: (
    <>
      <circle cx="12" cy="12" r="8.25" />
      <path d="M12 7.5V12l3 1.5" />
    </>
  ),
  /** 自绘：键盘（终端原生输入） */
  keyboard: (
    <>
      <rect x="2.25" y="6.75" width="19.5" height="10.5" rx="1.5" />
      <path d="M6 10h.008M9 10h.008M12 10h.008M15 10h.008M18 10h.008M8 14h8" />
    </>
  )
};

export type IconName = keyof typeof PATHS;

/** 供测试与「名字表/路径表必须同步」的断言使用。 */
export const ICON_NAMES = Object.keys(PATHS) as IconName[];

export function Icon({
  name,
  size = 16,
  className,
  title
}: {
  name: IconName;
  size?: number;
  className?: string;
  /** 有意义时给（会带 role="img"）；纯装饰不给（aria-hidden）。 */
  title?: string;
}) {
  return (
    <svg
      className={className}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={title ? undefined : "true"}
      role={title ? "img" : undefined}
      aria-label={title}
    >
      {PATHS[name]}
    </svg>
  );
}
```

- [ ] **Step 4: 运行 Icon 测试，确认通过**

```bash
cd ui && npm test -- src/components/icon.test.tsx
```

预期：PASS。

- [ ] **Step 5: 实现 badge / status-dot / progress / toggle**

创建 `ui/src/components/badge.tsx`：

```tsx
import type { ReactNode } from "react";

export type BadgeTone = "accent" | "neutral" | "warn" | "danger";

/** 徽章：消息条数、候选池状态、平台标识等小面积信息。mono 用于数字/编号。 */
export function Badge({
  tone = "neutral",
  mono = false,
  children
}: {
  tone?: BadgeTone;
  mono?: boolean;
  children: ReactNode;
}) {
  return <span className={`badge badge-${tone}${mono ? " ui-num" : ""}`}>{children}</span>;
}
```

创建 `ui/src/components/status-dot.tsx`：

```tsx
export type StatusDotTone = "ok" | "warn" | "danger" | "idle";

/** 状态点。`pulse` 表示「进行中」（运行中的终端、在线网关）。 */
export function StatusDot({
  tone = "idle",
  pulse = false,
  size = 8,
  title
}: {
  tone?: StatusDotTone;
  pulse?: boolean;
  size?: number;
  title?: string;
}) {
  return (
    <span
      className={`status-dot-pin dot-${tone}${pulse ? " pulse" : ""}`}
      style={{ width: size, height: size }}
      title={title}
      aria-hidden="true"
    />
  );
}
```

创建 `ui/src/components/progress.tsx`：

```tsx
/** 进度条。value 为 0–100，超界夹取；tone 由调用方按语义给（≥80% 用 warn）。 */
export function ProgressBar({
  value,
  tone = "accent",
  label
}: {
  value: number;
  tone?: "accent" | "warn" | "danger";
  label?: string;
}) {
  const clamped = Math.min(100, Math.max(0, value));
  return (
    <div className={`progress progress-${tone}`} role="progressbar" aria-valuenow={clamped} aria-valuemin={0} aria-valuemax={100} aria-label={label}>
      <div className="progress-fill" style={{ width: `${clamped}%` }} />
    </div>
  );
}
```

创建 `ui/src/components/toggle.tsx`：

```tsx
/** 开关。受控组件——界面不允许再出现「本地状态与后端不一致」的开关。 */
export function Toggle({
  checked,
  onChange,
  disabled = false,
  label,
  title
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  label: string;
  title?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      title={title ?? label}
      disabled={disabled}
      className={`toggle${checked ? " on" : ""}`}
      onClick={() => onChange(!checked)}
    >
      <span className="toggle-knob" />
    </button>
  );
}
```

- [ ] **Step 6: 写 Popover 的失败测试**

创建 `ui/src/components/popover.test.tsx`：

```tsx
// @vitest-environment jsdom
import { fireEvent, render } from "@testing-library/react";
import { createRef } from "react";
import { describe, expect, it, vi } from "vitest";

import { Popover } from "./popover";

/**
 * 浮层的关闭行为。
 *
 * 两条必须有的：**点外部关闭**（不关的话浮层会挡住下面的内容且找不到出口）与
 * **Escape 关闭**（键盘用户的唯一出口）。
 */
describe("Popover", () => {
  it("打开时渲染内容，关闭时不渲染", () => {
    const anchor = createRef<HTMLButtonElement>();
    const closed = render(
      <button ref={anchor}>
        <Popover open={false} onClose={() => undefined} anchorRef={anchor}>
          内容
        </Popover>
      </button>
    );
    expect(closed.queryByText("内容")).toBeNull();

    const opened = render(
      <button ref={anchor}>
        <Popover open onClose={() => undefined} anchorRef={anchor}>
          内容
        </Popover>
      </button>
    );
    expect(opened.getByText("内容")).toBeTruthy();
  });

  it("点浮层外部与按 Escape 都会调 onClose", () => {
    const anchor = createRef<HTMLButtonElement>();
    const onClose = vi.fn();
    const view = render(
      <div>
        <button ref={anchor}>锚</button>
        <Popover open onClose={onClose} anchorRef={anchor}>
          内容
        </Popover>
      </div>
    );
    fireEvent.mouseDown(document.body);
    expect(onClose).toHaveBeenCalledTimes(1);

    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(2);

    // 点内部不该关。
    fireEvent.mouseDown(view.getByText("内容"));
    expect(onClose).toHaveBeenCalledTimes(2);
  });
});
```

- [ ] **Step 7: 运行，确认失败**

```bash
cd ui && npm test -- src/components/popover.test.tsx
```

预期：FAIL，`Cannot find module './popover'`。

- [ ] **Step 8: 实现 popover.tsx**

创建 `ui/src/components/popover.tsx`：

```tsx
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react";

/**
 * 锚定浮层（供应商锁定、附件菜单）。
 *
 * 用自绘而不是 `dialog` 元素：它是**贴在触发按钮旁边**的，不是模态——
 * 模态会抢走焦点并遮住整屏。
 *
 * 定位策略：先按「锚的下方、左对齐」放，超出视口就翻到上方（或右对齐），
 * 与现有右键菜单的 `viewport-menu.ts` 同一套「不越界」原则。
 */
export function Popover({
  open,
  onClose,
  anchorRef,
  children,
  className
}: {
  open: boolean;
  onClose: () => void;
  anchorRef: RefObject<HTMLElement | null>;
  children: ReactNode;
  className?: string;
}) {
  const panelRef = useRef<HTMLDivElement | null>(null);
  const [style, setStyle] = useState<{ top: number; left: number }>({ top: 0, left: 0 });

  useLayoutEffect(() => {
    if (!open) return;
    const anchor = anchorRef.current;
    const panel = panelRef.current;
    if (!anchor || !panel) return;
    const a = anchor.getBoundingClientRect();
    const p = panel.getBoundingClientRect();
    const below = a.bottom + 6;
    const top = below + p.height > window.innerHeight ? Math.max(8, a.top - p.height - 6) : below;
    const left = Math.min(Math.max(8, a.left), Math.max(8, window.innerWidth - p.width - 8));
    setStyle({ top, left });
  }, [open, anchorRef]);

  useEffect(() => {
    if (!open) return;
    const onDown = (event: MouseEvent) => {
      const target = event.target as Node;
      if (panelRef.current?.contains(target) || anchorRef.current?.contains(target)) return;
      onClose();
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, onClose, anchorRef]);

  if (!open) return null;
  return (
    <div
      ref={panelRef}
      className={`popover${className ? ` ${className}` : ""}`}
      style={{ top: style.top, left: style.left }}
      role="dialog"
    >
      {children}
    </div>
  );
}
```

- [ ] **Step 9: 扩展 Button 变体与 Dialog 尺寸**

修改 `ui/src/components/primitives.tsx` 的 `Button`：`variant?: "default" | "primary" | "danger" | "ghost"`；类名规则 `btn btn-${variant}`（原 `default` 行为不变，新增 `btn-ghost` 样式）。修改 `ui/src/components/dialog.tsx`：新增 `size?: "sm" | "md" | "lg"`，默认 `"md"`，类名追加 `dialog-panel-${size}`（`sm`=宽度 420–560，`md`=当前默认，`lg`=min(1024px, 92vw) 高 min(660px, 90vh)）。

- [ ] **Step 10: 加样式并导出**

在 `ui/src/components/components.css` 末尾追加（全部用 token）：

```css
/* ── 徽章 / 状态点 / 进度条 / 开关 / 浮层 ─────────────────────── */
.badge { display: inline-flex; align-items: center; gap: var(--ui-space-1); padding: 2px 8px; border-radius: var(--ui-radius-sm); font-size: var(--ui-text-xs); line-height: 1.5; white-space: nowrap; }
.badge-neutral { background: var(--ui-bg-badge); color: var(--ui-muted); }
.badge-accent { background: var(--ui-accent-soft); color: var(--ui-accent); border: 1px solid var(--ui-accent-border); }
.badge-warn { background: var(--ui-warn-soft); color: var(--ui-warn); border: 1px solid var(--ui-warn-border); }
.badge-danger { background: var(--ui-danger-soft); color: var(--ui-danger); border: 1px solid var(--ui-danger-border); }

.status-dot-pin { display: inline-block; border-radius: 50%; background: var(--ui-faint); flex: 0 0 auto; }
.status-dot-pin.dot-ok { background: var(--ui-accent); }
.status-dot-pin.dot-warn { background: var(--ui-warn); }
.status-dot-pin.dot-danger { background: var(--ui-danger); }
.status-dot-pin.dot-idle { background: var(--ui-border-strong); }
.status-dot-pin.pulse { animation: dot-pulse 1.6s ease-in-out infinite; }
@keyframes dot-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.45; } }

.progress { width: 100%; height: 6px; border-radius: 999px; overflow: hidden; background: var(--ui-bg-badge); }
.progress-fill { height: 100%; border-radius: 999px; background: var(--ui-accent); transition: width 160ms ease; }
.progress-warn .progress-fill { background: var(--ui-warn); }
.progress-danger .progress-fill { background: var(--ui-danger); }

.toggle { position: relative; width: 34px; height: 20px; border-radius: 999px; border: 1px solid var(--ui-border-strong); background: var(--ui-bg-badge); cursor: pointer; padding: 0; transition: background 120ms ease, border-color 120ms ease; }
.toggle .toggle-knob { position: absolute; top: 2px; left: 2px; width: 14px; height: 14px; border-radius: 50%; background: var(--ui-surface-knob, #fff); box-shadow: var(--ui-shadow-xs); transition: transform 120ms ease; }
.toggle.on { background: var(--ui-accent); border-color: var(--ui-accent); }
.toggle.on .toggle-knob { transform: translateX(14px); }
:root[data-theme="dark"] .toggle .toggle-knob { background: var(--ui-text); }
.toggle:disabled { opacity: 0.5; cursor: not-allowed; }

.popover { position: fixed; z-index: var(--ui-z-overlay); min-width: 160px; padding: var(--ui-space-2); border: 1px solid var(--ui-border-strong); border-radius: var(--ui-radius-lg); background: var(--ui-bg-modal); box-shadow: var(--ui-shadow); }

.btn-ghost { background: transparent; border-color: transparent; color: var(--ui-muted); }
.btn-ghost:hover:not(:disabled) { background: var(--ui-bg-card-hover); color: var(--ui-text); }
```

并在 `ui/src/components/index.ts` 追加导出：`export * from "./icon"; export * from "./badge"; export * from "./status-dot"; export * from "./progress"; export * from "./toggle"; export * from "./popover";`

- [ ] **Step 11: 全量门禁**

```bash
cd ui && npm run typecheck && npm test && npm run build
```

预期：全绿（现有 40 个测试文件 + 新增 2 个）。

- [ ] **Step 12: 提交**

```bash
cd ui && git add src/components src/styles/tokens.css
git commit -m "feat(ui): 设计系统基础组件（图标/徽章/状态点/进度条/开关/浮层）

Icon 用内联 SVG 注册表（零依赖），组件全部走 token；Button 加 ghost 变体、
Dialog 加尺寸档，供后续骨架与面板复用。"
```

---

### Task 3: 统一标签模型（纯逻辑）

**背景（必须读懂再动手）：** 现在的模型是「一个标签要么是会话、要么是终端」（`kind: "session" | "terminal"`），点历史会话会**同时**产生会话标签与终端标签。新模型（spec D3 / §9.1）改成**一个会话一张标签**：标签同时挂会话与终端；运行中显示终端、未运行显示历史。

**行为变化一处（重要，需在报告里明确写出）：** 现在「点会话行 = 恢复并进入终端」；新模型里「点会话行 = 打开/激活该会话的标签（未运行则显示历史）」，**恢复终端改为显式动作**（历史视图头部的「继续会话」按钮；`beginResumeSession` 逻辑完整保留）。理由：单击就拉起一个 CLI 进程既慢又意外；浏览历史是更常见的意图。

**Files:**
- Modify: `ui/src/app/workspace-tabs.ts`（整体重写）
- Modify: `ui/src/app/workspace-tabs.test.ts`（整体重写）
- Create: `ui/src/app/stage-content.ts`
- Create: `ui/src/app/stage-content.test.ts`

**Interfaces:**
- Consumes: 无。
- Produces:
  - `WorkspaceTabLike = { key: string; sessionId?: string; terminalId?: string; title: string }`
  - `openSessionTab<T extends WorkspaceTabLike>(tabs: T[], session: { id: string; title: string }): { tabs: T[]; activeKey: string }`
  - `attachTerminal<T extends WorkspaceTabLike>(tabs: T[], input: { terminalId: string; sessionId?: string; title: string }): { tabs: T[]; activeKey: string; replacedTerminalId?: string }`
  - `closeTab<T extends WorkspaceTabLike>(tabs: T[], key: string, activeKey: string): { tabs: T[]; activeKey: string }`
  - `tabSuffix(state?: { running: boolean; exitedCleanly?: boolean; exitCode?: number }): string`
  - `hasSessionTab(tabs: WorkspaceTabLike[]): boolean`
  - `resolveStageContent(tab, hasTerminal): StageContent`（`stage-content.ts`）
  - `type StageContent = { kind: "terminal"; terminalId: string } | { kind: "history"; sessionId: string } | { kind: "empty" }`

- [ ] **Step 1: 重写纯逻辑测试（先写测试）**

整体替换 `ui/src/app/workspace-tabs.test.ts`：

```ts
import { describe, expect, it } from "vitest";

import {
  attachTerminal,
  closeTab,
  hasSessionTab,
  openSessionTab,
  tabSuffix,
  type WorkspaceTabLike
} from "./workspace-tabs";

/**
 * 统一的会话标签模型（spec D3）。
 *
 * 三条最要紧的：**同一会话不建第二张标签**（点一次历史会话曾同时冒出两张）、
 * **终端挂到已有标签上而不是新建**（恢复会话时复用）、
 * **关闭激活标签后切到剩余最后一张**（不留空工作区）。
 */
describe("openSessionTab", () => {
  const session = { id: "sess-1", title: "会话甲" };

  it("没有相关标签时建一个会话标签（key = 会话编号）", () => {
    const { tabs, activeKey } = openSessionTab<WorkspaceTabLike>([], session);
    expect(tabs).toEqual([{ key: "sess-1", sessionId: "sess-1", title: "会话甲" }]);
    expect(activeKey).toBe("sess-1");
  });

  it("已有挂着该会话的终端标签时直接切过去，不新建", () => {
    const tabs: WorkspaceTabLike[] = [
      { key: "term-9", sessionId: "sess-1", terminalId: "term-9", title: "会话甲" }
    ];
    const result = openSessionTab(tabs, session);
    expect(result.tabs).toBe(tabs);
    expect(result.activeKey).toBe("term-9");
  });

  it("已有会话标签时切过去，不新建", () => {
    const tabs: WorkspaceTabLike[] = [{ key: "sess-1", sessionId: "sess-1", title: "会话甲" }];
    const result = openSessionTab(tabs, session);
    expect(result.tabs).toBe(tabs);
    expect(result.activeKey).toBe("sess-1");
  });

  it("别的会话的标签不影响，仍然新建自己那条", () => {
    const tabs: WorkspaceTabLike[] = [{ key: "sess-0", sessionId: "sess-0", title: "会话零" }];
    const result = openSessionTab(tabs, session);
    expect(result.tabs).toHaveLength(2);
    expect(result.activeKey).toBe("sess-1");
  });
});

describe("attachTerminal", () => {
  it("会话已有标签时把终端挂上去，key 不变，并报告被替换的旧终端", () => {
    const tabs: WorkspaceTabLike[] = [
      { key: "sess-1", sessionId: "sess-1", terminalId: "term-old", title: "会话甲" }
    ];
    const result = attachTerminal(tabs, { terminalId: "term-new", sessionId: "sess-1", title: "会话甲" });
    expect(result.activeKey).toBe("sess-1");
    expect(result.tabs[0].terminalId).toBe("term-new");
    expect(result.replacedTerminalId).toBe("term-old");
  });

  it("会话还没有标签时用会话编号为 key 新建（带终端）", () => {
    const result = attachTerminal<WorkspaceTabLike>([], {
      terminalId: "term-1",
      sessionId: "sess-2",
      title: "会话乙"
    });
    expect(result.tabs).toEqual([
      { key: "sess-2", sessionId: "sess-2", terminalId: "term-1", title: "会话乙" }
    ]);
    expect(result.activeKey).toBe("sess-2");
    expect(result.replacedTerminalId).toBeUndefined();
  });

  it("全新会话（还没有会话编号）以终端编号为 key 新建", () => {
    const result = attachTerminal<WorkspaceTabLike>([], { terminalId: "term-3", title: "新会话" });
    expect(result.tabs).toEqual([{ key: "term-3", terminalId: "term-3", title: "新会话" }]);
    expect(result.activeKey).toBe("term-3");
  });

  it("同一终端重复挂载不产生第二张标签", () => {
    const tabs: WorkspaceTabLike[] = [
      { key: "term-3", terminalId: "term-3", title: "新会话" }
    ];
    const result = attachTerminal(tabs, { terminalId: "term-3", title: "新会话" });
    expect(result.tabs).toHaveLength(1);
    expect(result.activeKey).toBe("term-3");
  });
});

describe("closeTab", () => {
  const tabs: WorkspaceTabLike[] = [
    { key: "sess-1", sessionId: "sess-1", title: "甲" },
    { key: "term-2", terminalId: "term-2", title: "乙" }
  ];

  it("关掉激活标签时切到剩余最后一张", () => {
    const result = closeTab(tabs, "term-2", "term-2");
    expect(result.tabs).toHaveLength(1);
    expect(result.activeKey).toBe("sess-1");
  });

  it("关掉非激活标签时激活键不变", () => {
    const result = closeTab(tabs, "sess-1", "term-2");
    expect(result.activeKey).toBe("term-2");
  });

  it("关掉最后一张时激活键清空", () => {
    const result = closeTab([tabs[0]], "sess-1", "sess-1");
    expect(result.tabs).toEqual([]);
    expect(result.activeKey).toBe("");
  });
});

describe("tabSuffix", () => {
  it("运行中不加后缀", () => {
    expect(tabSuffix({ running: true })).toBe("");
    expect(tabSuffix(undefined)).toBe("");
  });

  it("异常退出带退出码——1 与 137 指向完全不同的原因", () => {
    expect(tabSuffix({ running: false, exitedCleanly: false, exitCode: 137 })).toBe("（退出码 137）");
    expect(tabSuffix({ running: false, exitedCleanly: false })).toBe("（退出码 ?）");
  });

  it("干净退出标「已退出」（正常路径下会被自动收掉，这是兜底）", () => {
    expect(tabSuffix({ running: false, exitedCleanly: true })).toBe("（已退出）");
  });
});

describe("hasSessionTab", () => {
  it("只认挂着会话的标签", () => {
    expect(hasSessionTab([{ key: "term-1", terminalId: "term-1", title: "x" }])).toBe(false);
    expect(hasSessionTab([{ key: "sess-1", sessionId: "sess-1", title: "x" }])).toBe(true);
  });
});
```

创建 `ui/src/app/stage-content.test.ts`：

```ts
import { describe, expect, it } from "vitest";

import { resolveStageContent } from "./stage-content";

/**
 * 画布内容判定：终端优先于历史（同一张标签里两者都存在时，用户要看的是活的那一个）。
 * 终端不在了就回落到历史——这正是「异常退出的标签保留现场」之外的另一半：
 * 用户关掉终端后不该看到一块空白。
 */
describe("resolveStageContent", () => {
  const alive = (id: string) => id === "term-1";

  it("没有标签 → 空", () => {
    expect(resolveStageContent(null, alive)).toEqual({ kind: "empty" });
  });

  it("有存活的终端 → 终端", () => {
    expect(resolveStageContent({ terminalId: "term-1", sessionId: "s1" }, alive)).toEqual({
      kind: "terminal",
      terminalId: "term-1"
    });
  });

  it("终端不在（未启动 / 已关闭）但有会话 → 历史", () => {
    expect(resolveStageContent({ terminalId: "term-x", sessionId: "s1" }, alive)).toEqual({
      kind: "history",
      sessionId: "s1"
    });
  });

  it("没有会话也没有终端 → 空", () => {
    expect(resolveStageContent({}, alive)).toEqual({ kind: "empty" });
  });
});
```

- [ ] **Step 2: 运行，确认失败**

```bash
cd ui && npm test -- src/app/workspace-tabs.test.ts src/app/stage-content.test.ts
```

预期：FAIL（`resolveStageContent` 模块不存在；旧 `workspace-tabs.ts` 没有 `openSessionTab` 等导出）。

- [ ] **Step 3: 实现 stage-content.ts**

创建 `ui/src/app/stage-content.ts`：

```ts
/** 画布当前该显示什么。 */
export type StageContent =
  | { kind: "terminal"; terminalId: string }
  | { kind: "history"; sessionId: string }
  | { kind: "empty" };

/**
 * 判定规则：**终端优先**。
 *
 * 一张标签同时挂着会话与终端（spec D3 的统一模型），两者都在时显示终端——
 * 用户刚启动的就是它。终端不在了（还没启动、或被关掉）才回落到历史。
 * 这一步必须是纯函数：它是「切换画布」唯一的判据，错了整块区域就是错的。
 */
export function resolveStageContent(
  tab: { sessionId?: string; terminalId?: string } | null,
  hasTerminal: (terminalId: string) => boolean
): StageContent {
  if (!tab) return { kind: "empty" };
  if (tab.terminalId && hasTerminal(tab.terminalId)) {
    return { kind: "terminal", terminalId: tab.terminalId };
  }
  if (tab.sessionId) return { kind: "history", sessionId: tab.sessionId };
  return { kind: "empty" };
}
```

- [ ] **Step 4: 重写 workspace-tabs.ts**

整体替换 `ui/src/app/workspace-tabs.ts`：

```ts
/**
 * 工作区标签的规则（统一模型）。
 *
 * # 一张标签同时挂会话与终端
 *
 * 旧模型里「会话标签」与「终端标签」是两种东西，点一次历史会话会**同时**建出两张
 * （选中会话的 effect 建一张、起终端又建一张）。新模型（spec D3）把两者合成一张：
 *
 * - key 优先用**会话编号**（会话已知时），否则用终端编号（全新会话还没编号）；
 * - 终端通过 `attachTerminal` **挂到**已有标签上，绝不新建第二张；
 * - 画布显示什么由 [`resolveStageContent`](./stage-content.ts) 判定（终端优先）。
 *
 * 这里全部是纯函数：标签的增删是最容易出竞态的一块，纯函数才能逐条钉住。
 */

/** 标签。链路上只关心这几个字段。 */
export type WorkspaceTabLike = {
  key: string;
  /** 这条标签对应的会话（历史正文）。新会话在拿到编号前没有它。 */
  sessionId?: string;
  /** 这条标签上活着的终端。 */
  terminalId?: string;
  title: string;
};

/** 打开（或激活）一条会话的标签。已有挂着该会话的标签就切过去，不新建。 */
export function openSessionTab<T extends WorkspaceTabLike>(
  tabs: T[],
  session: { id: string; title: string }
): { tabs: T[]; activeKey: string } {
  const existing = tabs.find((tab) => tab.sessionId === session.id || tab.key === session.id);
  if (existing) return { tabs, activeKey: existing.key };
  return {
    tabs: [...tabs, { key: session.id, sessionId: session.id, title: session.title } as T],
    activeKey: session.id
  };
}

/**
 * 把一路终端挂到它的会话标签上；没有标签就建一个。
 *
 * 返回 `replacedTerminalId`：同一张标签上原来那一路终端（若有）——调用方要把它
 * `closeTerminal` 收掉，否则后端登记项与 xterm 会泄漏（这正是「再次进入会话被锁住」
 * 那个历史缺陷的成因之一）。
 */
export function attachTerminal<T extends WorkspaceTabLike>(
  tabs: T[],
  input: { terminalId: string; sessionId?: string; title: string }
): { tabs: T[]; activeKey: string; replacedTerminalId?: string } {
  const existing = input.sessionId
    ? tabs.find((tab) => tab.sessionId === input.sessionId)
    : tabs.find((tab) => tab.key === input.terminalId || tab.terminalId === input.terminalId);

  if (existing) {
    const replacedTerminalId =
      existing.terminalId && existing.terminalId !== input.terminalId
        ? existing.terminalId
        : undefined;
    const next = tabs.map((tab) =>
      tab.key === existing.key
        ? {
            ...tab,
            terminalId: input.terminalId,
            ...(tab.sessionId ? {} : input.sessionId ? { sessionId: input.sessionId } : {})
          }
        : tab
    );
    return { tabs: next, activeKey: existing.key, ...(replacedTerminalId ? { replacedTerminalId } : {}) };
  }

  const key = input.sessionId ?? input.terminalId;
  return {
    tabs: [
      ...tabs,
      {
        key,
        terminalId: input.terminalId,
        ...(input.sessionId ? { sessionId: input.sessionId } : {}),
        title: input.title
      } as T
    ],
    activeKey: key
  };
}

/** 关掉一张标签。关掉的是激活标签时切到剩余最后一张（空工作区看起来像「什么都没了」）。 */
export function closeTab<T extends WorkspaceTabLike>(
  tabs: T[],
  key: string,
  activeKey: string
): { tabs: T[]; activeKey: string } {
  const next = tabs.filter((tab) => tab.key !== key);
  return { tabs: next, activeKey: key === activeKey ? (next.at(-1)?.key ?? "") : activeKey };
}

/** 标签标题后缀。异常退出必须带退出码：`1` 与 `137` 要查的地方完全不同。 */
export function tabSuffix(state?: {
  running: boolean;
  exitedCleanly?: boolean;
  exitCode?: number;
}): string {
  if (!state || state.running) return "";
  if (state.exitedCleanly) return "（已退出）";
  return `（退出码 ${state.exitCode ?? "?"}）`;
}

/** 有没有能导出的活动会话（导出动作的启用条件）。 */
export function hasSessionTab(tabs: WorkspaceTabLike[]): boolean {
  return tabs.some((tab) => tab.sessionId !== undefined);
}
```

删除旧导出 `activateSessionTab` / `deadTabsForSession` / `findSession` / `ResumeRequest`。**先 grep 确认无引用再删：**

```bash
cd ui && grep -rn "activateSessionTab\|deadTabsForSession\|ResumeRequest\|findSession" src --include=*.ts --include=*.tsx | grep -v "workspace-tabs"
```

预期：只应有 `use-workspace-tabs.ts`（Task 4 会重写它）。若还有别处引用，在该处一并改成新 API，并在提交信息里点名。

- [ ] **Step 5: 运行测试，确认通过**

```bash
cd ui && npm test -- src/app/workspace-tabs.test.ts src/app/stage-content.test.ts
```

预期：PASS。

- [ ] **Step 6: 提交**

```bash
cd ui && git add src/app/workspace-tabs.ts src/app/workspace-tabs.test.ts src/app/stage-content.ts src/app/stage-content.test.ts
git commit -m "refactor(ui): 统一会话标签模型（一个会话一张标签）

标签同时挂会话与终端，终端经 attachTerminal 复用已有标签并报告被替换的旧终端；
画布内容由 resolveStageContent 判定（终端优先、回落历史）。纯逻辑先行，接线在下一提交。"
```

---

### Task 4: 标签 hook 与 app 接线

**Files:**
- Modify: `ui/src/app/use-workspace-tabs.ts`（整体重写，保留文件头「为什么这段逻辑离开 app.tsx」的说明）
- Modify: `ui/src/app/app.tsx`（接线：openTerminal / beginResumeSession / 标签菜单动作）
- Modify: `ui/src/app/session-workspace.tsx`（画布按 `resolveStageContent` 切换；视觉重做在 Task 9）
- Modify: `ui/src/app/use-workspace-tabs` 无独立测试文件；行为由 Task 3 的纯逻辑测试覆盖，hook 层新增断言见 Step 1

**Interfaces:**
- Consumes: Task 3 的 `openSessionTab` / `attachTerminal` / `closeTab` / `tabSuffix` / `hasSessionTab` / `resolveStageContent`。
- Produces:
  - `useWorkspaceTabs(terminals, selectedSession)` 返回：
    `{ tabs: WorkspaceTab[]; activeKey: string; setActiveKey: (key: string) => void; tabItems: WorkspaceTabItem[]; activeTab: WorkspaceTab | null; attachTerminalTab: (terminalId: string, sessionId: string | undefined, title: string) => void; closeTab: (key: string) => void; closeOtherTabs: (keepKey: string) => void; closeAllTabs: () => void }`
  - `type WorkspaceTab = WorkspaceTabLike & { sessionId?: string; terminalId?: string }`
  - `type WorkspaceTabItem = { key: string; label: string; suffix: string; tone: "normal" | "attention"; providerId?: string; state: "running" | "idle" | "exited" }`（供 Task 9 的 TabBar；本任务先喂给现有 `Tabs` 组件时取 `{key,label,tone}`）

- [ ] **Step 1: 重写 hook**

整体替换 `ui/src/app/use-workspace-tabs.ts`。要点（保留原文件里两条注释精神：React 不保证同步调用更新函数；干净退出要收标签）：

```ts
import { useCallback, useEffect, useMemo, useState } from "react";

import type { AiSession, TerminalAttention } from "../types";
import { displayTitle } from "./session-display";
import { attachTerminal, closeTab as closeTabPure, openSessionTab, tabSuffix, type WorkspaceTabLike } from "./workspace-tabs";
import type { useTerminals } from "./use-terminals";
import type { TerminalInputMode } from "./composer-text";

export type WorkspaceTab = WorkspaceTabLike;

export type WorkspaceTabItem = {
  key: string;
  label: string;
  suffix: string;
  tone: "normal" | "attention";
  providerId?: string;
  state: "running" | "idle" | "exited";
};

export function useWorkspaceTabs(
  terminals: ReturnType<typeof useTerminals>,
  selectedSession: AiSession | null
) {
  const [tabs, setTabs] = useState<WorkspaceTab[]>([]);
  const [activeKey, setActiveKey] = useState("");

  /**
   * 把一路终端挂到标签上（等价于旧模型里的 addTerminalTab）。
   *
   * 关键差异：**复用**已有标签而不是新建；被替换掉的旧终端要 `closeTerminal`，
   * 否则后端登记项与 xterm 会泄漏。`closeTerminal` 必须从**闭包里的 tabs** 之外取
   * ——它是 `terminals` 的方法，直接调用即可，不经过 setState 更新函数。
   */
  const attachTerminalTab = useCallback(
    (terminalId: string, sessionId: string | undefined, title: string) => {
      let replaced: string | undefined;
      setTabs((current) => {
        const result = attachTerminal(current, { terminalId, sessionId, title });
        replaced = result.replacedTerminalId;
        setActiveKey(result.activeKey);
        return result.tabs;
      });
      setActiveKey((currentKey) => {
        // setTabs 的更新函数不保证同步执行，这里用当前 tabs 的闭包值来算激活键。
        const result = attachTerminal(tabs, { terminalId, sessionId, title });
        return result.activeKey || currentKey;
      });
      if (replaced) void terminals.closeTerminal(replaced);
    },
    [tabs, terminals]
  );
  ...
}
```

**注意：** 上面这段故意展示了一个坑——`replaced` 在 `setTabs` 的更新函数里被赋值，而 React 不保证同步调用它（这正是原文件头注释记录的缺陷形态）。**不要照抄它**。正确写法：先用闭包里的 `tabs` 算出结果，再 `setTabs(result.tabs)` / `setActiveKey(result.activeKey)`：

```ts
  const attachTerminalTab = useCallback(
    (terminalId: string, sessionId: string | undefined, title: string) => {
      const result = attachTerminal(tabs, { terminalId, sessionId, title });
      setTabs(result.tabs);
      setActiveKey(result.activeKey);
      if (result.replacedTerminalId) void terminals.closeTerminal(result.replacedTerminalId);
    },
    [tabs, terminals]
  );
```

其余部分（选中会话自动成标签、干净退出自动收、attention 映射、批量关闭）按同样的「闭包值先算、再 setState」原则写：

```ts
  /** 选中会话 → 打开/激活它的标签（列表点击、搜索跳转、批量后刷新都汇到这一处）。 */
  useEffect(() => {
    if (!selectedSession) return;
    const result = openSessionTab(tabsRef.current, {
      id: selectedSession.id,
      title: displayTitle(selectedSession)
    });
    setTabs(result.tabs);
    setActiveKey(result.activeKey);
  }, [selectedSession]);
```

（其中 `tabsRef` 用 `useRef<WorkspaceTab[]>([])` 与 `useEffect` 同步：这个 effect 依赖 `selectedSession` 一个键，读最新 tabs 只能用 ref。）

```ts
  /** 干净退出的终端：收掉它的标签（异常退出保留现场看报错）。 */
  useEffect(() => {
    const finished = tabs.filter(
      (tab) =>
        tab.terminalId !== undefined &&
        terminals.terminals.some((item) => item.id === tab.terminalId && item.exitedCleanly === true)
    );
    for (const tab of finished) closeTab(tab.key);
  }, [tabs, terminals.terminals, closeTab]);

  const closeTab = useCallback(
    (key: string) => {
      const tab = tabs.find((item) => item.key === key);
      const result = closeTabPure(tabs, key, activeKey);
      setTabs(result.tabs);
      setActiveKey(result.activeKey);
      if (tab?.terminalId) void terminals.closeTerminal(tab.terminalId);
    },
    [tabs, activeKey, terminals]
  );

  /** 关闭其他 / 关闭全部：所有标签都可关（统一模型里没有「跟着选中走的会话标签」了）。 */
  const closeOtherTabs = useCallback(
    (keepKey: string) => {
      const closing = tabs.filter((tab) => tab.key !== keepKey);
      setTabs(tabs.filter((tab) => tab.key === keepKey));
      for (const tab of closing) if (tab.terminalId) void terminals.closeTerminal(tab.terminalId);
    },
    [tabs, terminals]
  );

  const closeAllTabs = useCallback(() => {
    const closing = [...tabs];
    setTabs([]);
    setActiveKey("");
    for (const tab of closing) if (tab.terminalId) void terminals.closeTerminal(tab.terminalId);
  }, [tabs, terminals]);

  const activeTab = tabs.find((tab) => tab.key === activeKey) ?? null;

  const tabItems: WorkspaceTabItem[] = useMemo(
    () =>
      tabs.map((tab) => {
        const terminal = terminals.terminals.find((item) => item.id === tab.terminalId);
        return {
          key: tab.key,
          label: tab.title,
          suffix: tabSuffix(terminal),
          tone: tab.terminalId && terminals.attention[tab.terminalId] ? ("attention" as const) : ("normal" as const),
          providerId: terminal?.providerId,
          state: terminal ? (terminal.running ? "running" : "exited") : "idle"
        };
      }),
    [tabs, terminals.attention, terminals.terminals]
  );

  return { tabs, activeKey, setActiveKey, tabItems, activeTab, openSession: ..., attachTerminalTab, closeTab, closeOtherTabs, closeAllTabs };
```

`openSession` 的实现：把 `openSessionTab` 包一层（供 app 在 `console.openAt` 之后无需额外动作；选中联动 effect 已覆盖，故 `openSession` 可省略——**实现时如无调用方就不写它**，YAGNI）。

- [ ] **Step 2: 适配 session-workspace.tsx（暂时保持旧外观）**

修改 `ui/src/app/session-workspace.tsx`：
- `SessionWorkspace` 的 props 增加 `stage: StageContent`（由 app.tsx 用 `resolveStageContent` 算出）；
- `terminal-stack` 的显隐条件从 `activeTab?.kind === "terminal"` 改为 `stage.kind === "terminal"`；`TerminalPane` 的 `active` 从 `activeTab?.terminalId === item.id` 改为 `stage.kind === "terminal" && stage.terminalId === item.id`；
- 历史正文的渲染条件从 `activeTab?.kind === "terminal" ? null : <SessionBody/>` 改为 `stage.kind === "history" ? <SessionBody/> : stage.kind === "empty" ? <EmptyState …/> : null`；
- `tabItems` 暂时映射成旧 `Tabs` 需要的形状（`{ key, label: label + suffix, tone }`），由 app.tsx 完成映射（`WorkspaceTabItem` → `TabItem`），Task 9 换成新 TabBar 后删除该映射。

- [ ] **Step 3: 改 app.tsx 接线**

`ui/src/app/app.tsx` 的修改点（逐处）：
1. `openTerminal`：`workspace.addTerminalTab(...)` → `workspace.attachTerminalTab(id, options?.sessionId, options?.title ?? "新会话")`；
2. `beginResumeSession`：删除 `workspace.clearDeadTabs(session.id)` 调用（新模型里 `attachTerminal` 会替换旧终端并自动收掉它）；其余 `planResume` 逻辑不动；
3. **标签 ↔ 选中联动（spec §9.1 行为表）**：把现有「选中会话跟随当前终端」的 effect（现在判 `kind === "terminal"` 才同步）改成统一规则——**激活任一标签、且它有 `sessionId` 时**，`console.setSelectedId(activeTab.sessionId)`；标签没有 `sessionId`（全新会话）时不动选中项。这条规则是「切标签时左侧列表高亮跟着走」的唯一实现，别在点击处各写一次；
4. 加 `const stage = resolveStageContent(workspace.activeTab, (id) => terminals.terminals.some((item) => item.id === id));` 并传给 `SessionWorkspace`；
5. 标签右键菜单动作：`closeTabThis: workspace.closeTab`（不变）、`closeTabOthers: workspace.closeOtherTabs`、`closeTabAll: workspace.closeAllTabs`（不变）；`closableTabCount` 改为 `workspace.tabs.length`；
6. `hasActiveSession`（导出启用条件）继续用 `workspace.tabs.some((tab) => tab.sessionId)`；
7. **会话行点击行为改变**：`resumeById` 保留给「继续会话」动作；新增 `openSessionById(target, sessionId)`：`console.openAt(target, session)`（不再起终端），失败时给出与现在相同的提示。`Sidebar` 的 `onResume` 改为传 `openSessionById`，并把 prop 改名为 `onOpen`。

- [ ] **Step 4: 更新受影响测试并跑全量**

```bash
cd ui && npm run typecheck && npm test
```

预期：FAIL 集中在 `use-console.test.ts` / `use-terminals.test.tsx`（若有引用）与 `sidebar` 相关（`onResume` 改名）。逐个修：把「点会话行会起终端」的断言改成「点会话行只打开标签」；`onResume` → `onOpen`。全部改完后重跑，必须全绿。

- [ ] **Step 5: 应用启动冒烟**

```bash
cd ui && npm run build
cd .. && pwsh scripts/ui-check.ps1 -Name t4-tabs
```

预期：应用能启动；点会话行不再拉起终端，画布显示该会话历史；点「+ Codex / 新建会话」起终端后同一会话的标签变终端视图。**用截图与 `-Probe` 各验证一次：**

```powershell
pwsh scripts/ui-check.ps1 -Name t4-tabs-probe -Probe "document.querySelectorAll('.tabs .tab').length"
```

预期：数字与打开的标签数一致（不是每点一次多一张）。

- [ ] **Step 6: 提交**

```bash
cd ui && git add src/app/use-workspace-tabs.ts src/app/app.tsx src/app/session-workspace.tsx src/app/sidebar.tsx
git commit -m "refactor(ui): 标签 hook 与接线切到统一模型

终端挂到会话标签上并回收被替换的旧终端；画布由 resolveStageContent 切换；
会话行单击改为打开标签（历史），恢复终端改为显式动作。"
```

---

### Task 5: 设置中心外壳（现有面板挂入 + 修 B1）

**Files:**
- Create: `ui/src/features/settings/settings-nav.ts`
- Create: `ui/src/features/settings/settings-nav.test.ts`
- Create: `ui/src/features/settings/settings-center.tsx`
- Create: `ui/src/features/settings/panels/about-panel.tsx`
- Create: `ui/src/features/settings/panel-header.tsx`
- Modify: `ui/src/app/config-overlay.tsx` → 删除（被 settings-center 取代）
- Modify: `ui/src/app/config/environment-panel.tsx`（加「WSL 会话目录」卡）
- Modify: `ui/src/app/overlay-layer.tsx`（ConfigOverlay → SettingsCenter；去掉 aboutOpen）
- Modify: `ui/src/app/app.tsx`（openConfig → openSettings；去掉 about 状态）
- Modify: `ui/src/ipc/commands.ts:149`（修 B1）
- Create: `ui/src/ipc/commands.test.ts`
- Delete: `ui/src/app/about-dialog.tsx`、`ui/src/app/about-dialog.test.tsx`

**Interfaces:**
- Consumes: Task 2 的 `Icon` / `Badge` / `Toggle`。
- Produces:
  - `settings-nav.ts`：`type SettingsTab`（10 个 id，含 `"about"`）；`SETTINGS_GROUPS: SettingsGroup[]`（4 组，顺序见 spec §10）；`SETTINGS_TAB_IDS: SettingsTab[]`；`settingsTabLabel(tab: SettingsTab): string`；`type SettingsGroup = { id: string; label: string; icon: IconName; tabs: Array<{ id: SettingsTab; label: string }> }`。
  - `SettingsCenter({ providerId, providers, targetId, targets, selectedTarget, initialTab, version, sessionSettings, onSessionSettings..., onQuit, onOpenLogDirectory, onClose, onError, onNotify })`。
  - `PanelHeader({ title, description, actions }: { title: string; description?: string; actions?: ReactNode })`。
  - `AboutPanel({ version, onOpenLogDirectory, onQuit })`。

- [ ] **Step 1: 写 settings-nav 的失败测试**

创建 `ui/src/features/settings/settings-nav.test.ts`：

```ts
import { describe, expect, it } from "vitest";

import { SETTINGS_GROUPS, SETTINGS_TAB_IDS, settingsTabLabel } from "./settings-nav";

/**
 * 导航模型。两条：**每个面板恰好出现一次**（漏一个等于那个面板没有入口，
 * 重复一个会出现两个激活态）与**分组顺序固定**（spec §10 是验收依据）。
 */
describe("设置中心导航", () => {
  it("十个面板恰好各出现一次", () => {
    const ids = SETTINGS_GROUPS.flatMap((group) => group.tabs.map((tab) => tab.id));
    expect(ids).toHaveLength(10);
    expect(new Set(ids).size).toBe(10);
    expect([...ids].sort()).toEqual([...SETTINGS_TAB_IDS].sort());
  });

  it("分组顺序与服务/观测/扩展/关于一致", () => {
    expect(SETTINGS_GROUPS.map((group) => group.label)).toEqual([
      "服务与路由",
      "观测与审计",
      "环境与扩展",
      "系统与关于"
    ]);
  });

  it("每一页都有可读标题", () => {
    for (const id of SETTINGS_TAB_IDS) {
      expect(settingsTabLabel(id).length).toBeGreaterThan(0);
    }
    expect(settingsTabLabel("about")).toBe("关于控制台");
  });
});
```

- [ ] **Step 2: 运行，确认失败**

```bash
cd ui && npm test -- src/features/settings/settings-nav.test.ts
```

预期：FAIL，模块不存在。

- [ ] **Step 3: 实现 settings-nav.ts**

创建 `ui/src/features/settings/settings-nav.ts`：

```ts
import type { IconName } from "../../components";

/** 设置中心的面板编号。与 spec §10 的表一一对应。 */
export type SettingsTab =
  | "vendors"
  | "gateway"
  | "usage"
  | "failures"
  | "logs"
  | "skills"
  | "environment"
  | "presets"
  | "diagnostics"
  | "about";

export type SettingsGroup = {
  id: string;
  label: string;
  icon: IconName;
  tabs: Array<{ id: SettingsTab; label: string }>;
};

/**
 * 分组不是装饰：十个扁平标签在 224px 的导航列里读不出结构，
 * 而「服务与路由」（改行为）与「观测与审计」（只看不改）是两类动作。
 */
export const SETTINGS_GROUPS: SettingsGroup[] = [
  {
    id: "service",
    label: "服务与路由",
    icon: "globe",
    tabs: [
      { id: "vendors", label: "供应商管理" },
      { id: "gateway", label: "本地网关 & 路由" }
    ]
  },
  {
    id: "observe",
    label: "观测与审计",
    icon: "chart",
    tabs: [
      { id: "usage", label: "用量报表" },
      { id: "failures", label: "异常诊断" },
      { id: "logs", label: "请求日志" }
    ]
  },
  {
    id: "extend",
    label: "环境与扩展",
    icon: "wrench",
    tabs: [
      { id: "skills", label: "平台技能 (Skills)" },
      { id: "environment", label: "运行时环境" },
      { id: "presets", label: "工作区预设" },
      { id: "diagnostics", label: "系统诊断" }
    ]
  },
  {
    id: "about",
    label: "系统与关于",
    icon: "info",
    tabs: [{ id: "about", label: "关于控制台" }]
  }
];

export const SETTINGS_TAB_IDS: SettingsTab[] = SETTINGS_GROUPS.flatMap((group) =>
  group.tabs.map((tab) => tab.id)
);

export function settingsTabLabel(tab: SettingsTab): string {
  for (const group of SETTINGS_GROUPS) {
    const found = group.tabs.find((item) => item.id === tab);
    if (found) return found.label;
  }
  return tab;
}
```

- [ ] **Step 4: 修 B1（命令名）+ 回归测试**

修改 `ui/src/ipc/commands.ts:149-150`：

```ts
/** 在系统文件管理器里打开某个会话所在的目录。 */
export const openSessionFolder = (targetId: string, sessionId: string) =>
  invoke<unknown>("shell:open-session-folder", { targetId, sessionId });
```

创建 `ui/src/ipc/commands.test.ts`（这条测试专防「前端调的命令名后端不认」这类静默缺陷）：

```ts
// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";

import { openSessionFolder } from "./commands";

/**
 * 命令名必须与后端注册表一致。
 *
 * 此前这里踩过一次：前端调 `codex:open-session-folder`，后端注册的是
 * `shell:open-session-folder`——**两边都编译得过**，按钮点下去报「未知命令」，
 * 只有人肉发现。因此对这条（以及同类）命令补一条断言。
 */
describe("openSessionFolder", () => {
  const invoke = vi.fn(async () => null);

  beforeEach(() => {
    invoke.mockClear();
    (window as unknown as { __TAURI__: unknown }).__TAURI__ = {
      core: { invoke },
      event: { listen: vi.fn() }
    };
  });

  it("调用后端注册的 shell:open-session-folder", async () => {
    await openSessionFolder("local", "sess-1");
    expect(invoke).toHaveBeenCalledWith("aivc_invoke", {
      command: "shell:open-session-folder",
      args: { targetId: "local", sessionId: "sess-1" }
    });
  });
});
```

运行确认通过：

```bash
cd ui && npm test -- src/ipc/commands.test.ts
```

- [ ] **Step 5: 实现 PanelHeader 与 AboutPanel**

创建 `ui/src/features/settings/panel-header.tsx`：

```tsx
import type { ReactNode } from "react";

/** 面板头：标题 + 一句说明 + 右侧操作区。十个面板共用，保证「每个面板第一眼长得一样」。 */
export function PanelHeader({
  title,
  description,
  actions
}: {
  title: string;
  description?: string;
  actions?: ReactNode;
}) {
  return (
    <div className="panel-header">
      <div className="panel-header-text">
        <h3>{title}</h3>
        {description && <p>{description}</p>}
      </div>
      {actions && <div className="panel-header-actions">{actions}</div>}
    </div>
  );
}
```

创建 `ui/src/features/settings/panels/about-panel.tsx`：

```tsx
import { Badge, Button } from "../../../components";
import { PanelHeader } from "../panel-header";

/**
 * 关于控制台。
 *
 * 它同时承接了被移除的菜单栏里的两个动作：**打开日志目录**与**退出**
 * （spec §11 的入口迁移表）。「检查新版本」不实现——后端没有这个能力
 * （spec §15），做一个点不动的按钮比没有更糟。
 */
export function AboutPanel({
  version,
  onOpenLogDirectory,
  onQuit
}: {
  version: string;
  onOpenLogDirectory: () => void;
  onQuit: () => void;
}) {
  return (
    <div className="about-panel">
      <PanelHeader title="关于控制台" description="AI CLI 会话与本地智能 Gateway 管理平台" />

      <div className="about-brand">
        <div className="about-logo">AI</div>
        <div>
          <h3>AI Visual Console（AI 可视化控制台）</h3>
          <p>统一管理 Codex / Claude / Gemini / Qoder 会话，网关提供故障转移、熔断与 Token 审计。</p>
        </div>
      </div>

      <div className="about-cards">
        <div className="about-card">
          <span>应用版本</span>
          <Badge tone="accent" mono>
            v{version || "…"}
          </Badge>
        </div>
        <div className="about-card">
          <span>渲染核心</span>
          <b className="ui-mono">React 19 + @xterm/xterm</b>
        </div>
      </div>

      <div className="about-actions">
        <Button onClick={onOpenLogDirectory} title="在文件管理器里打开日志目录">
          打开日志目录
        </Button>
        <Button variant="danger" onClick={onQuit} title="退出应用（会先确认）">
          退出控制台
        </Button>
      </div>
    </div>
  );
}
```

- [ ] **Step 6: 实现 SettingsCenter**

创建 `ui/src/features/settings/settings-center.tsx`。它自己渲染模态外壳（比 `Dialog` 多两栏结构，硬套会把 Dialog 撑成上帝组件）：

```tsx
import { useState } from "react";

import { Icon } from "../../components";
import { DiagnosticsPanel } from "../../app/config/diagnostics-panel";
import { EnvironmentPanel } from "../../app/config/environment-panel";
import { GatewayPanel } from "../../app/config/gateway-panel";
import { PresetsPanel } from "../../app/config/presets-panel";
import { SkillsPanel } from "../../app/config/skills-panel";
import { VendorsPanel } from "../../app/config/vendors-panel";
import { GatewayFailurePanel } from "../../app/gateway-failure-panel";
import { GatewayLogPanel } from "../../app/gateway-log-panel";
import { GatewayUsagePanel } from "../../app/gateway-usage-panel";
import type { AiProviderId, AiProviderSummary, AiTarget } from "../../types";
import { AboutPanel } from "./panels/about-panel";
import { SETTINGS_GROUPS, type SettingsTab } from "./settings-nav";

export type { SettingsTab } from "./settings-nav";

/**
 * 设置中心（模态两栏）。
 *
 * 左列是分组导航，右列是面板。与旧 `ConfigOverlay` 的差别只在**信息架构**：
 * 九个扁平标签收进四个组、多出「关于」页承接菜单栏迁移过来的动作（spec §11）。
 * 面板本身全部原样复用——它们的业务逻辑与测试是既有资产，重排发生在批次二。
 */
export function SettingsCenter({
  providerId,
  providers,
  targetId,
  targets,
  selectedTarget,
  initialTab = "vendors",
  version,
  sessionSettings,
  onOpenSessionSettings,
  onChangeSessionSettingsPath,
  onRestoreSessionSettings,
  onSaveSessionSettings,
  onCloseSessionSettings,
  onQuit,
  onOpenLogDirectory,
  onClose,
  onError,
  onNotify
}: { /* 各 prop 类型与旧 ConfigOverlay 一致；sessionSettings 三件套与 app.tsx 现有的同名状态一致 */ }) {
  const [tab, setTab] = useState<SettingsTab>(initialTab);

  return (
    <div className="settings-overlay" role="dialog" aria-modal="true" aria-label="系统配置与服务中心">
      <div className="settings-panel">
        <header className="settings-head">
          <span className="settings-head-icon">
            <Icon name="settings" size={16} />
          </span>
          <div className="settings-head-text">
            <h2>系统配置与服务中心</h2>
            <p>统一管理供应商、网关路由、观测审计、运行时扩展及系统信息</p>
          </div>
          <button type="button" className="icon-btn" title="关闭" aria-label="关闭" onClick={onClose}>
            <Icon name="close" size={16} />
          </button>
        </header>

        <div className="settings-body">
          <nav className="settings-nav" aria-label="设置分类">
            {SETTINGS_GROUPS.map((group) => (
              <div className="settings-group" key={group.id}>
                <div className="settings-group-label">
                  <Icon name={group.icon} size={13} />
                  <span>{group.label}</span>
                </div>
                {group.tabs.map((item) => (
                  <button
                    key={item.id}
                    type="button"
                    className={`settings-nav-item${tab === item.id ? " on" : ""}`}
                    onClick={() => setTab(item.id)}
                  >
                    {item.label}
                  </button>
                ))}
              </div>
            ))}
          </nav>

          <div className="settings-content">
            {tab === "vendors" && (
              <VendorsPanel providerId={providerId} providers={providers} onError={onError} onNotify={onNotify} />
            )}
            {tab === "gateway" && <GatewayPanel onError={onError} onNotify={onNotify} />}
            {tab === "usage" && <GatewayUsagePanel onError={onError} />}
            {tab === "failures" && <GatewayFailurePanel providerId={providerId} onError={onError} />}
            {tab === "logs" && <GatewayLogPanel onError={onError} />}
            {tab === "skills" && <SkillsPanel targetId={targetId} onError={onError} onNotify={onNotify} />}
            {tab === "environment" && (
              <EnvironmentPanel
                providerId={providerId}
                targetId={targetId}
                selectedTarget={selectedTarget}
                onError={onError}
                onNotify={onNotify}
                onOpenSessionSettings={onOpenSessionSettings}
              />
            )}
            {tab === "presets" && <PresetsPanel onError={onError} />}
            {tab === "diagnostics" && (
              <DiagnosticsPanel targets={targets} selectedTargetId={targetId} providers={providers} selectedProviderId={providerId} />
            )}
            {tab === "about" && (
              <AboutPanel version={version} onOpenLogDirectory={onOpenLogDirectory} onQuit={onQuit} />
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
```

同时把 `sessionSettings` 对话框原样沿用：`EnvironmentPanel` 新增的「WSL 会话目录」卡里放一个按钮「设置会话目录…」调用 `onOpenSessionSettings`（app.tsx 现有的 `setSessionSettings({...})`）；对话框本体（`session-settings-dialog.tsx`）与它的测试不动。

- [ ] **Step 7: 改 app.tsx 与 overlay-layer 接线，删旧文件**

1. `app.tsx`：`ConfigTab` → `SettingsTab`（import 从 `./config-overlay` 改为 `../features/settings/settings-center`）；`openConfig` 改名 `openSettings`；删除 `aboutOpen` / `appVersion` 状态与 `showAbout` 动作，改传 `onQuit: () => window.close()`、`onOpenLogDirectory: () => void api.appCommand("openLogDir")`；`appVersion` 改由 `SettingsCenter` 自己取（在 `SettingsCenter` 内 `useEffect` 调 `api.getAppVersion()`，失败显示「无法读取」）。
2. `overlay-layer.tsx`：渲染 `SettingsCenter` 取代 `ConfigOverlay`；从 `OverlaysState` 删除 `aboutOpen`/`appVersion`，从 `OverlaysActions` 删除 `closeAbout`。
3. 删除文件：`ui/src/app/config-overlay.tsx`、`ui/src/app/about-dialog.tsx`、`ui/src/app/about-dialog.test.tsx`。
4. `grep` 确认无残留引用：`grep -rn "ConfigOverlay\|ConfigTab\|about-dialog\|aboutOpen" ui/src` 应零输出。

- [ ] **Step 8: 补设置中心样式**

在 `ui/src/app/app.css` 追加（沿用旧 `.overlay` 的层级与遮罩思路，尺寸按 spec §10）：

```css
.settings-overlay { position: fixed; inset: 0; z-index: var(--ui-z-overlay); display: flex; align-items: center; justify-content: center; background: rgb(2 6 23 / 55%); padding: var(--ui-space-5); }
.settings-panel { width: min(1024px, calc(100vw - 48px)); height: min(660px, calc(100vh - 48px)); display: flex; flex-direction: column; overflow: hidden; border: 1px solid var(--ui-border-strong); border-radius: var(--ui-radius-xl); background: var(--ui-bg-modal); box-shadow: var(--ui-shadow); }
.settings-head { display: flex; align-items: center; gap: var(--ui-space-3); padding: var(--ui-space-3) var(--ui-space-5); border-bottom: 1px solid var(--ui-border); background: var(--ui-bg-modal-nav); flex: 0 0 auto; }
.settings-head-icon { width: 30px; height: 30px; border-radius: var(--ui-radius-md); display: inline-flex; align-items: center; justify-content: center; background: var(--ui-accent); color: #fff; }
.settings-head-text h2 { margin: 0; font-size: var(--ui-text-lg); }
.settings-head-text p { margin: 2px 0 0; font-size: var(--ui-text-xs); color: var(--ui-faint); }
.settings-head .icon-btn { margin-left: auto; }
.settings-body { flex: 1; display: grid; grid-template-columns: 224px minmax(0, 1fr); min-height: 0; }
.settings-nav { padding: var(--ui-space-3); border-right: 1px solid var(--ui-border); background: var(--ui-bg-modal-nav); overflow-y: auto; }
.settings-group { margin-bottom: var(--ui-space-4); }
.settings-group-label { display: flex; align-items: center; gap: var(--ui-space-1); padding: 0 var(--ui-space-2) var(--ui-space-1); font-size: var(--ui-text-xs); font-weight: 600; color: var(--ui-faint); }
.settings-nav-item { display: block; width: 100%; text-align: left; padding: var(--ui-space-2) var(--ui-space-3); border: 0; border-radius: var(--ui-radius-md); background: transparent; color: var(--ui-muted); font-size: var(--ui-text-sm); font-weight: 500; cursor: pointer; }
.settings-nav-item:hover { background: var(--ui-bg-card-hover); color: var(--ui-text); }
.settings-nav-item.on { background: var(--ui-accent-soft); color: var(--ui-accent); font-weight: 600; }
.settings-content { padding: var(--ui-space-5) var(--ui-space-5) var(--ui-space-6); overflow-y: auto; background: var(--ui-bg-modal); }
.panel-header { display: flex; align-items: flex-start; justify-content: space-between; gap: var(--ui-space-4); padding-bottom: var(--ui-space-3); margin-bottom: var(--ui-space-4); border-bottom: 1px solid var(--ui-border); }
.panel-header h3 { margin: 0; font-size: var(--ui-text-lg); }
.panel-header p { margin: var(--ui-space-1) 0 0; font-size: var(--ui-text-sm); color: var(--ui-muted); }
.panel-header-actions { display: flex; align-items: center; gap: var(--ui-space-2); flex: 0 0 auto; }
.icon-btn { display: inline-flex; align-items: center; justify-content: center; width: 28px; height: 28px; border: 0; border-radius: var(--ui-radius-md); background: transparent; color: var(--ui-muted); cursor: pointer; }
.icon-btn:hover { background: var(--ui-bg-card-hover); color: var(--ui-text); }
.about-brand { display: flex; align-items: center; gap: var(--ui-space-4); padding-bottom: var(--ui-space-4); border-bottom: 1px solid var(--ui-border); }
.about-logo { width: 52px; height: 52px; border-radius: var(--ui-radius-xl); display: inline-flex; align-items: center; justify-content: center; font-weight: 700; font-size: var(--ui-text-xl); color: #fff; background: linear-gradient(135deg, #059669, #14b8a6); }
.about-brand h3 { margin: 0; font-size: var(--ui-text-lg); }
.about-brand p { margin: var(--ui-space-1) 0 0; font-size: var(--ui-text-sm); color: var(--ui-muted); }
.about-cards { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--ui-space-3); margin: var(--ui-space-4) 0; }
.about-card { display: flex; flex-direction: column; gap: var(--ui-space-1); padding: var(--ui-space-3); border: 1px solid var(--ui-border); border-radius: var(--ui-radius-lg); background: var(--ui-bg-sidebar); font-size: var(--ui-text-xs); color: var(--ui-muted); }
.about-actions { display: flex; gap: var(--ui-space-2); }
```

- [ ] **Step 9: 环境面板加「WSL 会话目录」卡**

修改 `ui/src/app/config/environment-panel.tsx`：props 增加可选 `selectedTarget?: AiTarget | null` 与 `onOpenSessionSettings?: () => void`；在工具状态卡之后渲染：

```tsx
      {selectedTarget?.kind === "wsl" && onOpenSessionSettings && (
        <Card title="WSL 会话目录" description={`当前发行版：${selectedTarget.distro ?? "-"}（Codex 的 CODEX_HOME 覆盖）`}>
          <Button small onClick={onOpenSessionSettings} title="手动指定 WSL 内的 Codex 目录">
            设置会话目录…
          </Button>
        </Card>
      )}
```

（`Card`/`Button` 从 `../../components` 引入；该文件已用 `Card`。）同步更新 `environment-panel.test.tsx`：加一条「WSL 目标时显示设置会话目录按钮、本机目标不显示」。

- [ ] **Step 10: 全量门禁 + 截图**

```bash
cd ui && npm run typecheck && npm test && npm run build
cd .. && pwsh scripts/ui-check.ps1 -Name t5-settings
```

预期：全绿；截图里点活动栏…（ActivityBar 在 Task 6）——本步先用临时手段打开设置中心核对：`-Probe "document.querySelectorAll('.settings-nav-item').length"` 返回 10。

- [ ] **Step 11: 提交**

```bash
cd ui && git add -A src
git commit -m "feat(ui): 设置中心外壳（分组导航 + 关于面板）并修复打开会话目录命令名

九面板挂入新的两栏外壳，新增关于面板承接菜单栏的打开日志目录与退出；
环境面板加 WSL 会话目录入口。修 B1：openSessionFolder 调用后端不认的
codex:open-session-folder，改为 shell:open-session-folder 并补回归测试。"
```

---

### Task 6: 应用新壳 + 移除菜单栏

**这是本批次唯一「删入口」的任务。** 前置条件已满足：Task 5 让退出 / 打开日志目录 / 设置会话 / 关于都有了新家。本任务还要补两处**立刻会丢的入口**：`清理 Gateway 日志`（在 `gateway-log-panel.tsx` 加「清空全部日志」按钮）与 `导出`（在会话右键菜单加导出分区）。补完之后再删菜单文件，功能零丢失。

**Files:**
- Create: `ui/src/shell/app-shell.tsx` / `title-bar.tsx` / `activity-bar.tsx` / `status-bar.tsx`
- Move: `ui/src/app/attention-center.tsx` → `ui/src/features/attention/attention-center.tsx`（连同 `attention-center.test.tsx`，用 `git mv`）
- Modify: `ui/src/app/use-terminals.ts`（输入模式/输入框可见性上收）
- Modify: `ui/src/app/terminal-pane.tsx`（改受控）
- Modify: `ui/src/app/vendor-route-picker.tsx`（改成浮层式供应商切换）
- Modify: `ui/src/app/session-context-menu.tsx`（加导出三项）
- Modify: `ui/src/app/config/gateway-log-panel.tsx`（加清空全部日志）
- Modify: `ui/src/app/app.tsx`（换壳、删菜单、活动栏、主题按钮、铃铛）
- Modify: `ui/src/app/app.css`（新壳样式；删 `.menubar*` / `.sidebar.collapsed` 相关死规则）
- Delete: `ui/src/app/app-menubar.tsx`、`ui/src/app/app-menu-bar.tsx`、`ui/src/app/app-menus.ts`、`ui/src/app/app-menu-bar.test.tsx`、`ui/src/app/status-bar.tsx`（已迁移）

**Interfaces:**
- Consumes: Task 1 的 `useTheme`；Task 2 的 `Icon` / `StatusDot` / `Popover`。
- Produces:
  - `AppShell({ titleBar, activityBar, sidebar, main, statusBar }: { titleBar: ReactNode; activityBar: ReactNode; sidebar: ReactNode; main: ReactNode; statusBar: ReactNode })`——只负责骨架网格。
  - `TitleBar({ version, preference, onCycleTheme, attention, onSelectAttention, onDismissAttention })`（`attention` 形状与现有 `AttentionCenter` 的 `entries` 相同）。
  - `ActivityBar({ activity, onActivity, onOpenSettings })`，`activity: "sessions" | "workbench"`。
  - `StatusBar({ busy, searching, error, session, target, terminals, activeTerminalId, gateway, inputMode, onToggleInputMode })`。
  - `useTerminals()` 新增返回：`uiState: Record<string, { inputMode: TerminalInputMode; composerVisible: boolean }>`、`setInputMode(id, mode)`、`noteOutput(id)`。

- [ ] **Step 1: 输入模式上收（use-terminals + terminal-pane）**

修改 `ui/src/app/use-terminals.ts`：

```ts
/** 每路终端的输入侧界面状态。上收到这里是因为**状态栏也要读它**（spec §6）。 */
export type TerminalUiState = { inputMode: TerminalInputMode; composerVisible: boolean };

const [uiStates, setUiStates] = useState<Record<string, TerminalUiState>>({});

const setInputMode = useCallback((id: string, mode: TerminalInputMode) => {
  setUiStates((current) => {
    const state = current[id] ?? { inputMode: "composer" as const, composerVisible: false };
    // 切到自管输入时顺带把输入框显出来——否则「切过去了但看不见框」。
    return { ...current, [id]: { ...state, inputMode: mode, composerVisible: mode === "composer" ? true : state.composerVisible } };
  });
}, []);

/** 收到输出：按 `shouldRevealComposer` 决定是否把隐藏的输入框放出来（恢复历史会话的路径）。 */
const noteOutput = useCallback((id: string) => {
  setUiStates((current) => {
    const state = current[id];
    if (!state || state.composerVisible) return current;
    return shouldRevealComposer(state.composerVisible, true)
      ? { ...current, [id]: { ...state, composerVisible: true } }
      : current;
  });
}, []);
```

- `open()` 的入参加 `sessionId?: string`，启动成功后写入 `uiStates[id] = { inputMode: "composer", composerVisible: initialComposerVisible(sessionId) }`（`initialComposerVisible` 从 `composer-text.ts` 引入，语义不变：新会话可见、恢复会话隐藏）；
- `closeTerminal` 里顺手 `delete next[id]`；
- 返回里加 `uiStates / setInputMode / noteOutput`。

修改 `ui/src/app/terminal-pane.tsx`：删除本地 `inputMode`/`composerVisible` 状态与 `shouldRevealComposer` 调用，改为 props 传入 `inputMode: TerminalInputMode`、`composerVisible: boolean`、`onInputModeChange: (mode: TerminalInputMode) => void`、`onOutput: () => void`（在数据回调里调，替代原来的 `setComposerVisible`）。`session-workspace.tsx` 透传这四个 props。

- [ ] **Step 2: 写 ActivityBar / TitleBar / AppShell**

创建 `ui/src/shell/activity-bar.tsx`：

```tsx
import { Icon } from "../components";

/** 极窄活动栏（50px）：两个一级去处 + 底部的设置入口。 */
export function ActivityBar({
  activity,
  onActivity,
  onOpenSettings
}: {
  activity: "sessions" | "workbench";
  onActivity: (activity: "sessions" | "workbench") => void;
  onOpenSettings: () => void;
}) {
  return (
    <aside className="activity-bar" aria-label="功能区">
      <div className="activity-top">
        <button
          type="button"
          className={`activity-item${activity === "sessions" ? " on" : ""}`}
          title="AI 会话管理"
          aria-label="AI 会话管理"
          aria-pressed={activity === "sessions"}
          onClick={() => onActivity("sessions")}
        >
          <Icon name="chat" size={20} />
        </button>
        <button
          type="button"
          className={`activity-item${activity === "workbench" ? " on" : ""}`}
          title="工作台与运行概览"
          aria-label="工作台与运行概览"
          aria-pressed={activity === "workbench"}
          onClick={() => onActivity("workbench")}
        >
          <Icon name="chart" size={20} />
        </button>
      </div>
      <div className="activity-bottom">
        <button
          type="button"
          className="activity-item"
          title="偏好设置与系统配置中心"
          aria-label="偏好设置与系统配置中心"
          onClick={onOpenSettings}
        >
          <Icon name="settings" size={20} />
        </button>
      </div>
    </aside>
  );
}
```

创建 `ui/src/shell/title-bar.tsx`：品牌（渐变方块 + AI Visual Console + `v{version}` 徽章）· 拖拽留白（`style={{ WebkitAppRegion: "drag" } as CSSProperties}`）· 右侧主题按钮（`Icon` 按 `resolved` 用 `sun`/`moon`，label 用 `themePreferenceLabel(preference)`，`title="切换明亮 / 暗黑 / 跟随系统"`）· 铃铛（渲染现有 `AttentionCenter`，props 原样传）。

创建 `ui/src/shell/app-shell.tsx`：

```tsx
import type { ReactNode } from "react";

/**
 * 应用骨架网格：标题栏 /〔活动栏 + 侧栏 + 主区〕/ 状态栏。
 * 只管布局，不持有任何状态——状态全在 `app.tsx`，这样骨架本身可以单测渲染。
 */
export function AppShell({
  titleBar,
  activityBar,
  sidebar,
  main,
  statusBar
}: {
  titleBar: ReactNode;
  activityBar: ReactNode;
  sidebar: ReactNode;
  main: ReactNode;
  statusBar: ReactNode;
}) {
  return (
    <div className="app-shell">
      {titleBar}
      <div className="app-body">
        {activityBar}
        {sidebar}
        <div className="app-main">{main}</div>
      </div>
      {statusBar}
    </div>
  );
}
```

- [ ] **Step 3: 状态栏重做（shell/status-bar.tsx）**

从 `ui/src/app/status-bar.tsx` 迁移并重写内容（`git mv` 后重写）。结构：左组「状态字 · 会话编号（点击复制）· 更新 · 目录（截断 + title）· 目标 · 供应商（Popover 触发）」；右组「网关 · 输入模式胶囊」。保留：busy/searching 文案、错误条（`.err`）、`gatewayPortLabel` 的四态文案、窄窗降级（沿用现有媒体查询断点）。新增：

```tsx
      <button
        type="button"
        className={`input-mode-pill${inputMode === "composer" ? " on" : ""}`}
        onClick={onToggleInputMode}
        disabled={!activeTerminalId}
        title={terminalInputButtonTitle(inputMode)}
      >
        <Icon name={inputMode === "composer" ? "chat" : "keyboard"} size={12} />
        <span>{inputMode === "composer" ? "自管输入" : "终端原生输入"}</span>
      </button>
```

会话编号点击复制用现有 `api.copyText`，复制后 `notify`（上层已有 toast，props 加 `onNotify?: (message: string) => void`）。

- [ ] **Step 4: 供应商切换改浮层（vendor-route-picker.tsx）**

保留文件与数据逻辑（自己 `listVendors(provider)`、切换失败不预改本地状态），只把渲染改成 `Popover`：触发按钮显示「供应商: 名称 +（锁定徽章）+ ▾」；浮层内容：头部（当前供应商 + 「网关在线 / 未走网关」）、供应商列表（点击即 `onSwitch`，当前项带 `StatusDot tone="ok"` 与「活跃」、其余「候选」；健康态可选：打开浮层时 `api.getVendorHealth()` 成功就用它标「熔断中/异常」，失败就**不显示健康信息**，不报错）、底部锁定复选框（`checked={mode === "locked"}`，切换调 `onSetMode`）。直连（`terminal.vendorId` 为空）时按钮显示「直连（自己的密钥）」+ `Icon name="alert"` 提示。
更新 `vendor-route-picker.test.tsx`：断言「点触发按钮打开浮层」「列表项点击调 onSwitch」「锁定复选框调 onSetMode」。

- [ ] **Step 5: 补两处入口（导出 / 清空日志）**

1. `session-context-menu.tsx`：在「复制文件路径」之后插入导出分区（视觉上用一条 `.context-menu-label` 分隔标签「导出」+ 三个条目：导出 Markdown / 导出 JSON / 导出 HTML）。props 增加 `onExport?: (format: "markdown" | "json" | "html") => void`，动作缺失时整组不渲染（沿用「缺动作即禁用」的既有约定，`app-menus.ts` 的原注释精神）。更新其测试：断言三个条目存在且点击回传对应格式。
2. `config/gateway-log-panel.tsx`：面板头（或工具行）加按钮「清空全部日志」，点击先 `window.confirm("确定清空所有网关审计日志吗？")`，确认后调现有 `api.clearGatewayLogs()`，成功 `onNotify(\`已清理 N 条 Gateway 日志。\`)`。更新测试（用 `vi.spyOn(window, "confirm")`）。
3. `app.tsx`：把原 `AppMenuActions` 里的 `exportSession` / `clearGatewayLogs` 接到上面两处（`runExport` 复用；导出前 `console.setSelectedId(session.id)` 保证导出的是右键那一条）。

- [ ] **Step 6: app.tsx 换壳并删除菜单**

1. `app.tsx` 顶部：`import { AppShell } from "../shell/app-shell"; import { ActivityBar } from "../shell/activity-bar"; import { TitleBar } from "../shell/title-bar"; import { StatusBar } from "../shell/status-bar";`（旧的 `./status-bar` import 删除）；`import { AttentionCenter } from "../features/attention/attention-center";`。
2. 新增 `const [themeVersion, setThemeVersion] = useState("");`（挂载时 `api.getAppVersion()` 填充，供 TitleBar 徽章）。
3. JSX 替换：`<div className="shell">` 及 `AppMenubar` → `<AppShell titleBar={…} activityBar={…} sidebar={…} main={…} statusBar={…} />`。其中：
   - `titleBar` = `<TitleBar version={themeVersion} preference={theme.preference} resolved={theme.resolved} onCycleTheme={() => theme.setPreference(nextPreference(theme.preference))} attention={{...}} onSelectAttention={...} onDismissAttention={...} />`（attention 的两个回调原样搬自现在的 `AppMenubar` 传参）；
   - `activityBar` = `<ActivityBar activity={activity} onActivity={(next) => { setActivity(next); console.setPane(next); }} onOpenSettings={() => openSettings("vendors")} />`，`const [activity, setActivity] = useState<"sessions" | "workbench">("sessions");`
   - `main` = 现有的 `<div className="workarea">…<SessionWorkspace …/></div>` 内容（保持不动）；
   - `statusBar` = `<StatusBar … inputMode={activeUi?.inputMode ?? "composer"} onToggleInputMode={…} onNotify={toast.show} />`（`activeUi = terminals.uiStates[workspace.activeTab?.terminalId ?? ""]`）。
4. 删除：`AppMenubar` import 与使用、`AppMenuActions` 组装块（actions 对象整块）、`aboutOpen/appVersion`（Task 5 已删一半，这里清干净）、`menu` 会话右键菜单不受影响。
5. 删除文件（`git rm`）：`app-menubar.tsx`、`app-menu-bar.tsx`、`app-menus.ts`、`app-menu-bar.test.tsx`；`git mv app/attention-center.tsx features/attention/attention-center.tsx` 并同步 `git mv app/attention-center.test.tsx features/attention/attention-center.test.tsx`，修正其中的相对 import（`../types` → `../../types`）。
6. `grep -rn "app-menubar\|app-menu-bar\|app-menus\|AppMenubar\|ConfigOverlay" src` 必须零输出。

- [ ] **Step 7: 新壳样式（app.css）**

在 `app.css` 顶部替换 `.shell` / `.main` / `.sidebar` / `.menubar` / `.statusbar` 的布局段，新增：

```css
.app-shell { display: grid; grid-template-rows: 40px minmax(0, 1fr) 28px; height: 100vh; overflow: hidden; background: var(--ui-bg); }
.app-body { display: grid; grid-template-columns: 50px auto minmax(0, 1fr); min-height: 0; }
.app-main { min-width: 0; min-height: 0; display: flex; flex-direction: column; }

.title-bar { display: flex; align-items: center; gap: var(--ui-space-3); padding: 0 var(--ui-space-3); border-bottom: 1px solid var(--ui-border); background: var(--ui-bg-header); flex: 0 0 auto; }
.title-brand { display: flex; align-items: center; gap: var(--ui-space-2); font-weight: 600; }
.title-logo { width: 24px; height: 24px; border-radius: var(--ui-radius-md); display: inline-flex; align-items: center; justify-content: center; font-size: var(--ui-text-xs); font-weight: 700; color: #fff; background: linear-gradient(135deg, #059669, #14b8a6); }
.title-drag { flex: 1; align-self: stretch; -webkit-app-region: drag; }
.title-actions { display: flex; align-items: center; gap: var(--ui-space-1); }

.activity-bar { display: flex; flex-direction: column; justify-content: space-between; align-items: center; padding: var(--ui-space-3) 0; border-right: 1px solid var(--ui-border); background: var(--ui-bg-activity); }
.activity-top, .activity-bottom { display: flex; flex-direction: column; gap: var(--ui-space-1); align-items: center; }
.activity-item { position: relative; width: 36px; height: 36px; border: 1px solid transparent; border-radius: var(--ui-radius-md); display: inline-flex; align-items: center; justify-content: center; background: transparent; color: var(--ui-muted); cursor: pointer; }
.activity-item:hover { background: var(--ui-bg-card-hover); color: var(--ui-text); }
.activity-item.on { background: var(--ui-accent-soft); border-color: var(--ui-accent-border); color: var(--ui-accent); }
.activity-item.on::before { content: ""; position: absolute; left: -7px; width: 3px; height: 16px; border-radius: 0 3px 3px 0; background: var(--ui-accent); }

.statusbar { display: flex; align-items: center; gap: var(--ui-space-3); padding: 0 var(--ui-space-3); border-top: 1px solid var(--ui-border); background: var(--ui-bg-statusbar); font-size: var(--ui-text-sm); color: var(--ui-muted); overflow: hidden; white-space: nowrap; }
.input-mode-pill { display: inline-flex; align-items: center; gap: var(--ui-space-1); padding: 2px 10px; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-sm); background: var(--ui-bg-badge); color: var(--ui-muted); font-size: var(--ui-text-xs); font-weight: 600; cursor: pointer; }
.input-mode-pill.on { background: var(--ui-accent-soft); border-color: var(--ui-accent-border); color: var(--ui-accent); }
.input-mode-pill:disabled { opacity: 0.5; cursor: not-allowed; }
```

侧栏宽度改 310px（`.sidebar { width: 310px; ... }`，工作台侧栏在 Task 8 用 340px）。删除 `.sidebar.collapsed` 的折叠条规则（折叠改为活动栏下方隐藏侧栏时 `grid-template-columns: 50px 0 minmax(0,1fr)`；`Sidebar` 的 `collapsed` 分支与 `»` 按钮删除，折叠按钮改到侧栏头右侧的 `Icon name="panel-left"`）。

- [ ] **Step 8: 门禁 + 截图核对**

```bash
cd ui && npm run typecheck && npm test && npm run build
cd .. && pwsh scripts/ui-baseline.ps1 -Name b1-shell
```

预期：全绿。逐张看 `shots/b1-shell/`：标题栏/活动栏/侧栏/工作区/状态栏五区就位；菜单栏消失；点活动栏「设置」开设置中心；铃铛在标题栏右侧；状态栏右端有输入模式胶囊。

- [ ] **Step 9: 提交**

```bash
cd ui && git add -A src
git commit -m "feat(ui): 应用新壳（标题栏/活动栏/状态栏）并移除菜单栏

菜单栏八个动作全部迁到设置中心、右键菜单与面板（导出进会话右键菜单、
清空日志进请求日志面板）；关注中心迁到标题栏；输入模式上收到 use-terminals
供状态栏读写；侧栏宽度 360→310。"
```

---

### Task 7: 会话侧栏重做

**Files:**
- Create: `ui/src/features/sessions/sessions-sidebar.tsx`
- Create: `ui/src/features/sessions/session-card.tsx`
- Create: `ui/src/features/sessions/sidebar-search.ts`（搜索区渲染规则，纯逻辑）
- Create: `ui/src/features/sessions/sidebar-search.test.ts`
- Modify: `ui/src/app/sidebar-nav.ts` + `sidebar-nav.test.ts`（一级只剩两处，二级并入 chips）
- Modify: `ui/src/app/use-console.ts`（`view` 支持 `"favorites"`；去掉 search pane）
- Modify: `ui/src/app/session-row.tsx`（`QuickGroup` / `SearchResultRow` 保形，样式类改新）
- Modify: `ui/src/app/app.tsx`（侧栏替换为 `SessionsSidebar` / `SidebarWorkbench` 由活动栏切换）
- Delete: `ui/src/app/sidebar.tsx`（被新侧栏取代；`WslHomeForm` 移入 `features/sessions/wsl-home-form.tsx`）

**Interfaces:**
- Consumes: Task 2 组件；Task 6 的 `app-shell` 布局（侧栏在 `app-body` 第二列）。
- Produces:
  - `SessionsSidebar({ console, selectedTarget, onOpen, onResume, onMenu, gateway, providers, collapsed, onToggleCollapsed })`——`onOpen(target, sessionId)` = 打开标签（历史），`onResume` = 在终端中恢复。
  - `SessionCard({ session, active, checked, selectable, running, onOpen, onToggle, onFavorite, onBranch, onMenu })`。
  - `sidebar-nav.ts`：`SidebarPane = "sessions" | "workbench"`；`type SidebarView = "active" | "favorites" | "trash"`；保留 `supportsTrash` / `supportsBatchActions`。

- [ ] **Step 1: 改 sidebar-nav（纯逻辑先改 + 测试）**

`sidebar-nav.ts`：删除 `paneForViewNav` / `showsViewNav` / `showsSessionSearchRow` / `viewNavKey` / `ViewNavKey`（其职责被 chips 与活动栏取代），`SidebarPane` 收窄为 `"sessions" | "workbench"`，新增 `SidebarView`。同步重写 `sidebar-nav.test.ts`：保留 `supportsTrash` / `supportsBatchActions` 的能力矩阵断言（Qoder 无回收站/批量），新增「页面到列表内容」的映射断言（view=favorites → 渲染 quickAccess，不发列表请求）。

- [ ] **Step 2: use-console 适配**

`use-console.ts` 的三处改动：
1. `pane` 类型收窄（`setPane` 只接受两个值）；`view` 类型改为 `SidebarView`。
2. 列表加载 effect：`view === "favorites"` 时**不请求会话列表**（收藏夹数据来自 `quickAccess`，它自己按需刷新）；从 favorites 切回 active/trash 时正常刷新。
3. 删掉「搜索 pane」相关状态（`globalQuery`/`setGlobalQuery`/`globalSearching`/`globalResults`/`searchProviderId`/`searchTargetId` 保留——它们现在服务于搜索结果区的「在所有目标中搜索」与筛选，挂到 `query` 非空或显式触发时展示）；删除 `setPane("search")` 的调用点。更新 `use-console.test.ts`。

- [ ] **Step 3: 写侧栏搜索区规则（纯逻辑 + 测试）**

创建 `ui/src/features/sessions/sidebar-search.ts`：

```ts
export type SidebarListMode =
  | { kind: "list" }        // 正常的会话列表
  | { kind: "filtered" }    // 本地/后端过滤结果（query 非空）
  | { kind: "global" }      // 跨目标搜索结果
  | { kind: "favorites" };  // 收藏夹（quickAccess）

/**
 * 列表区该显示什么。
 *
 * 判定顺序即优先级：**全局搜索 > 收藏夹 > 过滤结果 > 普通列表**。
 * 收藏夹排在过滤之前的理由：它是活动栏之外唯一「换了一处地方」的视图，
 * 用户在收藏夹里打字是在收藏里筛，而不是跳到别处搜。
 */
export function resolveSidebarListMode(input: {
  view: "active" | "favorites" | "trash";
  query: string;
  globalSearching: boolean;
  globalQuery: string;
}): SidebarListMode
```

配 `sidebar-search.test.ts`：五条断言（各分支 + 优先级）。

- [ ] **Step 4: 写 SessionCard**

创建 `ui/src/features/sessions/session-card.tsx`。结构（class 用新名，样式在 Step 7）：

```tsx
      <div
        className={`session-card${active ? " on" : ""}`}
        onClick={onOpen}
        onContextMenu={(event) => { event.preventDefault(); onMenu(event.clientX, event.clientY); }}
        title={title}
      >
        <div className="session-card-top">
          <StatusDot tone={running ? "ok" : "idle"} pulse={running} size={8} />
          {selectable && (
            <input type="checkbox" className="session-card-check" checked={checked} aria-label={`选择 ${title}`} onClick={(event) => event.stopPropagation()} onChange={onToggle} />
          )}
          <span className="session-card-title">{title}</span>
          <span className="session-card-time ui-num">{formatWhen(session.updatedAt)}</span>
        </div>
        <div className="session-card-preview">{previewLine(session)}</div>
        <div className="session-card-bottom">
          <span className="session-card-meta ui-num">
            {messageCountLabel(session.messageCount, true)}
            {branchCount > 0 ? ` · ${branchCount} 分支` : ""}
          </span>
          <div className="session-card-actions">
            <button type="button" title="标记收藏" onClick={(e) => { e.stopPropagation(); onFavorite(); }}>
              <Icon name="star" size={13} />
            </button>
            <button type="button" title="消息与分支" onClick={(e) => { e.stopPropagation(); onBranch(); }}>
              <Icon name="branch" size={13} />
            </button>
            <button type="button" title="更多" onClick={(e) => { e.stopPropagation(); onMenu(e.clientX, e.clientY); }}>
              <Icon name="chevron-down" size={13} />
            </button>
          </div>
        </div>
      </div>
```

`title` 用 `displayTitle(session)`；`previewLine` / `messageCountLabel` / `formatWhen` 全部复用 `session-display.ts`（不要重写格式化）。`branchCount` 从 `session.metadata?.branch` 不可得时省略（有则显示）。写 `session-card.test.tsx`：断言标题/预览/时间渲染、收藏点击回传、勾选框阻止冒泡（点击不触发 `onOpen`）。

- [ ] **Step 5: 组装 SessionsSidebar**

自上而下（全部新 class，样式见 Step 7）：头部（网关胶囊 + 新建会话按钮 + 折叠按钮）、平台/目标选择器（`1. 平台` / `2. 目标` 标签 + 右侧状态）、目标不可用 Notice + `WslHomeForm`（从旧 `sidebar.tsx` `git mv` 出去成 `features/sessions/wsl-home-form.tsx`，样式类改新）、搜索行（`SearchInput` + 「全部会话」按钮改为搜索结果区的「在所有目标中搜索」）、过滤 chips（全部 (N) / ⭐ 收藏 (N) + 回收站图标按钮）、列表区（按 `resolveSidebarListMode` 渲染：SessionCard 列表 / QuickGroup / SearchResultRow / EmptyState）、批量条（沿用现有逻辑与按钮）、底部摘要（`{平台} · 共 {N} 条会话` + `⟳ 刷新`）。

保留的既有语义（逐条实现，不要丢）：平台选择器空值占位「请选择平台」（旧 `sidebar.tsx:200-211` 的注释解释了为什么必须有占位项，连同注释一起搬运）、目标不可用项标「（不可用）」、`emptySessionMessage` 的三态空文案、批量条的平台能力门控、`onMenu` 回传坐标。

- [ ] **Step 6: app.tsx 接线 + 删旧侧栏**

`app.tsx`：`Sidebar` → `SessionsSidebar`；活动栏切到 workbench 时渲染 `SidebarWorkbench`（Task 8 包一层 `WorkbenchSidebar`，本任务先直接渲染现有组件，位置参数照旧）；`onOpen={openSessionById}`、`onResume={resumeById}`。删除 `ui/src/app/sidebar.tsx`（先把 `WslHomeForm` 与注释搬走）。

- [ ] **Step 7: 侧栏样式**

`app.css` 新增（要点，全部 token）：`.sidebar` 保持 grid 列宽 310px；`.sidebar-head`（品牌行 + 图标按钮）；`.gateway-pill`（accent-soft 胶囊 + 状态点 + 等宽端口）；`.picker-label`（`1. 平台` 小标签 + 右端提示）；`.filter-chips`（Chip 复用 `.chip`，trash 用 `.icon-btn`）；`.session-card`（`padding: var(--ui-space-3); border: 1px solid var(--ui-border); border-radius: var(--ui-radius-lg); background: var(--ui-bg-card);`，`.on { background: var(--ui-bg-card-active); border-color: var(--ui-accent-border); }`，hover 显示 `.session-card-actions`（`opacity: 0` → `1`，`@media (hover: none)` 下常显））；`.session-card-title`（`font-weight: 600; font-size: var(--ui-text-md);` 省略号）；`.session-card-preview`（`font-size: var(--ui-text-xs); color: var(--ui-faint);` 单行省略）；`.session-card-meta`（`font-size: var(--ui-text-xs); color: var(--ui-muted);`）；`.sidebar-foot`（`border-top` + 摘要 + 刷新按钮）。

- [ ] **Step 8: 门禁 + 截图**

```bash
cd ui && npm run typecheck && npm test && npm run build
cd .. && pwsh scripts/ui-baseline.ps1 -Name b1-sessions
```

预期：全绿；截图核对侧栏头部三件、选择器两行、搜索、chips、卡片三行结构与悬停动作、底部摘要。

- [ ] **Step 9: 提交**

```bash
cd ui && git add -A src
git commit -m "feat(ui): 会话侧栏重做（网关胶囊/卡片/筛选 chips/底部摘要）

一级导航上收到活动栏，收藏与回收站改为 chips 与图标；会话卡三行结构 +
悬停动作；保留批量、平台能力门控与全部空态文案。"
```

---

### Task 8: 工作台侧栏重做

**Files:**
- Create: `ui/src/features/workbench/workbench-sidebar.tsx`
- Create: `ui/src/features/workbench/runtime-card.tsx` / `rate-limit-card.tsx` / `route-change-card.tsx` / `usage-card.tsx` / `attention-list-card.tsx`
- Create: `ui/src/features/workbench/workbench-cards.ts`（速率窗口与文案的纯逻辑）+ `.test.ts`
- Modify: `ui/src/app/sidebar-workbench.tsx` → 删除（逻辑被上面的卡取代；`getRateLimitWindows` / `sessionLabel` / `describeSwitchReason` 搬进 `workbench-cards.ts` 并保留其测试断言）
- Modify: `ui/src/app/app.tsx`（活动栏 workbench 分支渲染 `WorkbenchSidebar`）

**Interfaces:**
- Consumes: Task 2 组件；现有 `sidebar-workbench.tsx` 的全部数据装配（provider/target/session/gateway/vendors/lastSwitch/refreshToken）与 `gateway-insights` 的取数方式。
- Produces: `WorkbenchSidebar({ provider, target, session, gateway, vendors, lastSwitch, refreshToken, onClose, onOpenUsage })`。

- [ ] **Step 1: 搬运纯逻辑并补测试**

`git mv ui/src/app/sidebar-workbench.tsx ui/src/features/workbench/` 之后：把 `sessionLabel` / `getRateLimitWindows` / `describeSwitchReason` 移入 `workbench-cards.ts`，原测试文件（`sidebar-workbench.test.tsx`）改为 `workbench-cards.test.ts` 并保留全部断言 + 新增两条：**进度条 tone 规则**（`usedPercent >= 80` → `warn`，否则 `accent`）与**无数据文案**（缺 `rateLimits` 时返回空数组而不是 `[undefined]`）。

- [ ] **Step 2: 五张卡**

每张卡一个文件、只做展示（数据由 `WorkbenchSidebar` 取或透传）：

1. `runtime-card.tsx`：目标/平台徽章、当前模型、会话已消耗 tokens（`formatTokens`）、`ProgressBar value={usage.contextPercent}` + `已用 {percent}% / {contextWindow}`。
2. `rate-limit-card.tsx`：主/次窗口各一行（`usedPercent% (已用/上限)` + `ProgressBar` + 重置倒计时用 `Icon name="clock"` + `resetsAt` 格式化）；无数据显示「无数据」。
3. `route-change-card.tsx`：`供应商 A ➔ B`（accent 色）+ 原因 + 时间；无变更显示空态文案。
4. `usage-card.tsx`：请求数（成功率）+ 平均耗时；右上「用量明细 ↗」调 `onOpenUsage`。
5. `attention-list-card.tsx`：最近 Gateway 异常（沿用现有列表与「查看详情」入口 → 设置中心异常页）。

取数沿用现有方式（`useAsyncData` + `refreshToken` + 200ms 合并），**不新增轮询**。

- [ ] **Step 3: 组装 + 样式 + 门禁**

`WorkbenchSidebar`：头部（`运行概览与工作台` + 「实时监控」徽章 + 关闭按钮 → `onClose` 回会话）+ 五卡；宽度 340px（`.sidebar.workbench { width: 340px; }`）。跑：

```bash
cd ui && npm run typecheck && npm test && npm run build
cd .. && pwsh scripts/ui-baseline.ps1 -Name b1-workbench
```

- [ ] **Step 4: 提交**

```bash
cd ui && git add -A src
git commit -m "feat(ui): 工作台侧栏重做（运行上下文/速率限制/路由变更/今日用量/最近异常）

五张卡各自成文件、只做展示；纯逻辑（速率窗口、进度条 tone、无数据文案）有单测。"
```

---

### Task 9: 标签栏与画布重做

**Files:**
- Create: `ui/src/features/workspace/tab-bar.tsx`
- Create: `ui/src/features/workspace/workspace-stage.tsx`
- Create: `ui/src/features/workspace/terminal-view.tsx`
- Create: `ui/src/features/workspace/history-view.tsx`
- Create: `ui/src/app/terminal-theme.ts` + `terminal-theme.test.ts`（xterm 明暗主题）
- Modify: `ui/src/app/session-workspace.tsx`（改用新组件；`SessionBody` 逻辑搬进 `history-view.tsx`）
- Modify: `ui/src/app/terminal-pane.tsx`（主题由 props 决定，改受控）
- Modify: `ui/src/app/app.css`
- Delete: `ui/src/components/tabs.tsx`（新 TabBar 取代；删除前 `grep -rn "components/tabs\|\"Tabs\"\|TabItem" ui/src` 确认无其他引用），同步从 `components/index.ts` 移除导出

**Interfaces:**
- Consumes: Task 3/4 的 `WorkspaceTabItem` / `resolveStageContent`；Task 2 的 `Icon` / `Badge` / `StatusDot`。
- Produces:
  - `TabBar({ items, activeKey, messageCount, canResume, onActivate, onClose, onContextMenu, onNewSession, onFind, onOpenMessages })`。
  - `WorkspaceStage({ stage, terminals, session, messages…, onResumeSession, onError, resolvedTheme })`。
  - `terminalThemeFor(resolved: "light" | "dark"): ITheme`。

- [ ] **Step 1: xterm 主题（纯逻辑 + 测试）**

创建 `ui/src/app/terminal-theme.ts`：

```ts
import type { ITheme } from "@xterm/xterm";

/**
 * xterm 的明暗主题。
 *
 * 值必须与 `tokens.css` 的 `--term-*` **逐字一致**（两处都改才是完整的改动）：
 * xterm 用 canvas 画字符，读不到 CSS 变量，只能把值抄一份在这里。
 * 测试把这件事钉住：断言两份值相同（测试里直接从 tokens.css 读）。
 */
export function terminalThemeFor(resolved: "light" | "dark"): ITheme {
  if (resolved === "dark") {
    return {
      background: "#0d0f17",
      foreground: "#e2e8f0",
      cursor: "#10b981",
      selectionBackground: "rgba(16, 185, 129, 0.28)"
    };
  }
  return {
    background: "#f8fafc",
    foreground: "#1e293b",
    cursor: "#0f172a",
    selectionBackground: "rgba(5, 150, 105, 0.22)"
  };
}
```

创建 `terminal-theme.test.ts`：读 `src/styles/tokens.css` 文本，用正则取出 `--term-bg` / `--term-text` / `--term-cursor` 在 `:root,`（明亮）与 `[data-theme="dark"]` 两段里的值，断言与 `terminalThemeFor` 的 `background/foreground/cursor` 完全一致。**这条测试就是防「改了 CSS 忘了改 xterm」**。

- [ ] **Step 2: 写 TabBar**

创建 `ui/src/features/workspace/tab-bar.tsx`。要点：左侧横向滚动的标签组（`overflow-x: auto`），每个标签 `active` 类决定「卡片底 + 边框 + 阴影」；内容 = `StatusDot`（`running` 时 `pulse`）+ 标题 + 平台 `Badge`（`providerId` 有值时显示，文案用平台 label 的首段，如 `Codex`）+ 关闭 `Icon name="close"`（hover 才显形，`@media (hover: none)` 常显）；标签标题后缀用 `item.suffix`。右端：`＋`（`onNewSession`，`title="新建会话 (Ctrl+T)"`）、`Icon name="search"`（`onFind`）、`消息与分支 (N)` 按钮（`Icon name="branch"` + `Badge`）。标签右键调 `onContextMenu(key, x, y)`。写 `tab-bar.test.tsx`：断言激活态类、关闭回调、消息计数、`＋` 调 `onNewSession`。

- [ ] **Step 3: 写画布三件**

1. `terminal-view.tsx`：会话信息条（标题 · 短编号 · 目标 · 供应商+模式 · 网关 Base URL；数据不可得的那一项**不渲染**）+ 终端栈容器（所有 `TerminalPane` 常挂载、非前台的 `.hidden`，沿用现有 `terminal-stack` 机制）+ 终端搜索条浮层（`Ctrl+F`，用现有 `TerminalSearchBar`，`readLines` 从 xterm buffer 取——沿用 `terminal-pane.tsx` 现有的暴露方式，**不要重写搜索**）。
2. `history-view.tsx`：把 `session-workspace.tsx` 里的 `SessionBody` 整体搬来并重做视觉：用户消息 = 引述块（`background: var(--ui-accent-soft); border-left: 3px solid var(--ui-accent); border-radius: 0 var(--ui-radius-lg) var(--ui-radius-lg) 0;`）、助手 = 无框正文、工具/系统 = 等宽小字块；顶部「加载更早的消息」；每条消息 hover 显示「从此处分支」；头部：标题 + `messageCountLabel` + `UsagePills`（搬过来）+ **「▶ 继续会话」按钮**（`onResumeSession`，这是 Task 3 行为变化后的显式恢复入口，`title="在终端中恢复这个会话"`）+ `BranchPanel`（血缘条，样式对齐 demo 的 `根会话 ➔ 当前 ➔ 子分支` 形态，逻辑与测试不动）。
3. `workspace-stage.tsx`：按 `stage.kind` 渲染三者（`terminal` → `TerminalView`；`history` → `HistoryView`；`empty` → 引导态：`没有打开的会话` + 提示「从左侧选择一个会话，或新建一个会话」）。

- [ ] **Step 4: session-workspace.tsx 收口 + terminal-pane 主题**

`session-workspace.tsx` 变成薄装配层：`<section className="workspace"><TabBar …/><WorkspaceStage …/></section>`，props 与现在一致，把 `stage` 传下去。删除 `SessionBody` / `UsagePills` / `sessionLabel`（已搬走）。`WorkspaceTrailing` 删除（被 TabBar 右端取代）；`canStartCodex` 保留（`app.tsx` 仍用它判断 `＋` 是否可用）。

`terminal-pane.tsx`：xterm 初始化时 `theme: terminalThemeFor(props.resolvedTheme)`，并在 `resolvedTheme` 变化时 `terminal.options.theme = terminalThemeFor(props.resolvedTheme)`（**不重建终端**——重建会清掉缓冲）。

- [ ] **Step 5: app.tsx 传参 + 样式**

`app.tsx`：给 `SessionWorkspace` 传 `resolvedTheme={theme.resolved}`、`onFind`（打开当前终端的搜索条——由 `TerminalView` 内部管理 `Ctrl+F` 与按钮触发，`app.tsx` 只需转发一个 `findSignal` 计数 prop）、`onNewSession={beginNewSession}`、`onOpenMessages={() => setDetailSession(activeSession)}`。`app.css` 追加标签栏与画布样式（`.tab-bar` / `.tab-pill` / `.tab-pill.on` / `.stage` / `.terminal-info-strip` / `.history-*`），并删除旧 `.tabs` / `.tab` / `.session-head` / `.thread` 中不再使用的规则。

- [ ] **Step 6: 门禁 + 截图**

```bash
cd ui && npm run typecheck && npm test && npm run build
cd .. && pwsh scripts/ui-baseline.ps1 -Name b1-stage
pwsh scripts/ui-check.ps1 -Name b1-dark -Probe "document.querySelector('.title-actions button')?.click()"
```

预期：全绿；明亮基线五张就位；`b1-dark` 截图是暗色（标题栏按钮循环到暗黑），终端底色随之变深。

- [ ] **Step 7: 提交**

```bash
cd ui && git add -A src
git commit -m "feat(ui): 现代流体标签栏与画布（终端视图/历史视图/空态）

标签含状态点、平台徽章与悬停关闭；右端查找与消息与分支入口；
终端主题随明暗联动（值与 tokens.css 一致并有测试钉住）。"
```

---

### Task 10: 输入区重做 + 模型选择

**Files:**
- Move: `ui/src/app/composer-input.tsx`、`composer-text.ts`、`composer-attachments.ts`、`composer-image-preview.tsx` 及各自测试 → `ui/src/features/workspace/composer/`（`git mv`，修正 import 路径）
- Create: `ui/src/features/workspace/composer/model-select.tsx` + `.test.tsx`
- Create: `ui/src/features/workspace/composer/models.ts` + `.test.ts`
- Modify: `ui/src/features/workspace/composer/composer-input.tsx`（视觉 + 附件下拉 + 去掉输入模式按钮 + 挂模型选择）
- Modify: `ui/src/ipc/commands.ts`（新增 `listModels`）
- Modify: `ui/src/app/use-terminals.ts`（`TerminalSession` 加 `targetId?`，`open()` 时写入）
- Modify: `ui/src/app/app.css`

**Interfaces:**
- Consumes: Task 6 上收的输入模式（状态栏负责切换，输入区不再有切换按钮）。
- Produces:
  - `resolveModelSource(input: { providerId: string; vendorId?: string; targetId?: string }): { kind: "vendor"; vendorId: string } | { kind: "qoder"; targetId: string } | null`。
  - `ModelSelect({ terminalId, providerId, vendorId, targetId, onSent, onError })`。
  - `commands.ts`：`listModels(targetId: string): Promise<Array<{ id: string }>>`。

- [ ] **Step 1: 模型来源规则（纯逻辑 + 测试）**

创建 `ui/src/features/workspace/composer/models.ts`：

```ts
/**
 * 模型列表从哪来。
 *
 * - 走网关（`vendorId` 存在）→ 那份供应商的模型（`vendor:list-models`）；
 * - Qoder 没有供应商候选池能力（`vendorManagement=false`），它的模型从 CLI 自己列
 *   （`models:list`，需要 targetId）——与旧平台一致（`listModels` 只在 Qoder 走）；
 * - 其余情况**不显示模型选择**（旧平台同样用 `modelSelectionSupported` 门控）。
 *   列不到模型时宁可没有这一项，也不要一个永远失败的下拉。
 */
export function resolveModelSource(input: {
  providerId: string;
  vendorId?: string;
  targetId?: string;
}): { kind: "vendor"; vendorId: string } | { kind: "qoder"; targetId: string } | null {
  if (input.providerId === "qoder") {
    return input.targetId ? { kind: "qoder", targetId: input.targetId } : null;
  }
  return input.vendorId ? { kind: "vendor", vendorId: input.vendorId } : null;
}
```

配 `models.test.ts`：四条断言（有供应商 → vendor；Qoder 有目标 → qoder；Qoder 无目标 → null；无供应商的非 Qoder → null）。

- [ ] **Step 2: 补 IPC 桥 + 终端记 targetId**

`commands.ts` 加（命令名与后端 `crates/aivc-ipc/src/commands.rs` 的 `models:list` 一字不差；**同时给 `commands.test.ts` 补一条同样形态的断言**）：

```ts
/** 列出某个目标下 CLI 自己能识别的模型（目前只有 Qoder 支持）。 */
export const listModels = (targetId: string) =>
  invoke<Array<{ id: string }>>("models:list", { targetId });
```

`use-terminals.ts`：`TerminalSession` 加 `targetId?: string`；`open()` 的 options 加 `targetId?: string` 并写入列表项；`openTerminal` 传 `target?.id`。

- [ ] **Step 3: ModelSelect（含测试）**

创建 `model-select.tsx`：默认渲染一个等宽小下拉（`<select>`）；**首次展开时才取数**（避免每开一个终端就发一次网络请求）；取数中显示「加载中…」；失败显示「无法获取模型」并允许再试（重新展开即重试）；选中的项写入终端 `api.writeTerminal(terminalId, \`/model ${id}\r\`)`（旧平台语义：立即生效）。`model-select.test.tsx`：mock `api`，断言「未展开不取数」「选中后写 `/model xxx`」「取数失败显示文案且不抛错」。

- [ ] **Step 4: 输入区视觉与附件下拉**

`composer-input.tsx`：
1. 结构改为 demo 形态：`.composer-card`（`border: 1px solid var(--ui-border-strong); border-radius: var(--ui-radius-lg); background: var(--ui-bg-input);` + `:focus-within { border-color: var(--ui-accent); box-shadow: 0 0 0 3px var(--ui-focus); }`），内含 `<textarea className="composer-text">` 与底部工具行；
2. 两个附件按钮合并为一个「📎 附件 ▾」（`Icon name="paperclip"`）触发 `Popover`，菜单两项：添加图片 / 添加文件（**回调逻辑与文案原样保留**，包括 `.md` 过滤器那个缺陷的修复说明）；
3. 删除 `onToggleInputMode` 按钮与相关 props（状态栏接管）；
4. 工具行左侧插入 `<ModelSelect …/>`（`resolveModelSource` 返回 null 时不渲染）；右侧保留「Enter 发送」提示与发送按钮；
5. `resolvedTheme` 无需传入（纯 CSS）。
更新 `composer-input.test.tsx`：删掉输入模式按钮的断言，新增「附件下拉打开后出现两项」「model-select 在无供应商时不渲染」。

- [ ] **Step 5: 门禁 + 截图 + 提交**

```bash
cd ui && npm run typecheck && npm test && npm run build
cd .. && pwsh scripts/ui-check.ps1 -Name b1-composer
cd /d/code/ai/aivc-rs/ui && git add -A src && git commit -m "feat(ui): 输入区重做并补齐模型选择

附件合并为单个下拉；输入模式切换移交状态栏；新增模型选择（供应商模型或
Qoder models:list，选中写 /model —— 旧平台有而 Rust 版缺失的能力）。"
```

---

### Task 11: 消息与分支弹框

**Files:**
- Create: `ui/src/features/workspace/messages-modal.tsx`
- Create: `ui/src/features/workspace/messages-modal.test.tsx`
- Modify: `ui/src/app/branch-panel.tsx`（改为可复用「血缘条」形态：横向 `根 → 当前 → 子`，props 不变，测试断言更新为新的 DOM 结构）
- Modify: `ui/src/app/session-detail-container.tsx` / `session-detail-dialog.tsx` → 逻辑并入 `messages-modal.tsx` 后删除（保留 `use-session-detail.ts` 与其测试）
- Modify: `ui/src/app/overlay-layer.tsx` / `app.tsx`（`detailSession` 渲染 `MessagesModal`）
- Delete: `ui/src/app/session-detail-dialog.test.tsx`（断言移植进 `messages-modal.test.tsx`）

**Interfaces:**
- Consumes: `use-session-detail`（分页与竞态保护，**不要重写**）、`Message` 渲染的既有角色规则（`session-row.tsx` 的 `Message`，本任务把它搬进 `messages-modal.tsx` 并重做视觉）。
- Produces: `MessagesModal({ session, messages, total, page, pageSize, loading, onPageChange, onPageSizeChange, onReload, onBranch, canBranch, onOpenSession, branchParent, branchChildren, onExport, onClose })`。

- [ ] **Step 1: 结构（先写测试）**

`messages-modal.test.tsx` 断言：① 头部有标题与 `ID:` 短编号；② 血缘条渲染「根会话 → 当前会话」（无父时只显示当前）；③ 点某条消息的「从此处分支」回传 `sourceLine`；④ 点子分支调 `onOpenSession`；⑤ 「导出」按钮回传 markdown/json/html 三种格式（用 popover 菜单）；⑥ 分页按钮按 `onPageChange` 回传页码与 `onPageSizeChange` 回传页大小。

- [ ] **Step 2: 实现**

按 demo 形态实现：头部（`Icon name="branch"` + 「会话消息历史与分支演进」+ `ID: …` 徽章 + 关闭）、血缘条（`BranchPanel` 新形态）、消息区（头像 我=蓝底/AI=accent 底 + 角色时间 + 内容卡；>4000 字折叠——**沿用现有 `Message` 的折叠规则**）、底部（`共 N 条历史消息` + 「导出 ▾」Popover + 关闭）。用 `size="lg"` 的 `Dialog` 或自绘 `.messages-*` 外壳（与设置中心同思路：模态尺寸与结构特殊，自绘更清晰；若用 `Dialog` 必须保持 `closeOnOverlay=false` 的既有约定）。

- [ ] **Step 3: 接线 + 门禁 + 提交**

`app.tsx`：`detailSession` 打开 `MessagesModal`；`onBranch` 用现有 `console.branchFromMessage`；`onExport` 用 `runExport`；`onOpenSession` 用 `beginResumeSession`（点父/子会话要能跳过去——沿用现有容器里的实现）。

```bash
cd ui && npm run typecheck && npm test && npm run build
cd .. && pwsh scripts/ui-check.ps1 -Name b1-messages -Probe "document.querySelectorAll('.messages-card').length"
git -C ui add -A src && git -C ui commit -m "feat(ui): 消息与分支弹框重构

血缘条（根→当前→子）+ 消息卡 + 分页 + 导出；分页与竞态逻辑沿用 use-session-detail。"
```

---

### Task 12: 批次一收尾

- [ ] **Step 1: 全量门禁**

```bash
cd ui && npm run typecheck && npm test && npm run build
```

预期：全绿（预计测试文件 40 → 48 左右、用例数量增长 ≥40）。

- [ ] **Step 2: 明暗两套截图核对（原尺寸看图）**

```bash
cd .. && pwsh scripts/ui-baseline.ps1 -Name b1-final
pwsh scripts/ui-check.ps1 -Name b1-final-dark-sessions -Probe "document.querySelector('.title-actions button')?.click()"
```

逐张核对（照 doc 29 的教训：**判断缺陷必须看原尺寸图或数据**）：五区就位、无死按钮、无未定义 token 导致的透明/继承色、暗色下终端与画布可读、状态栏窄窗降级（把窗口缩到 980px 再截一张）。

- [ ] **Step 3: 死代码清查**

```bash
cd ui && grep -rn "\.tabs\b\|\.tab\b\|\.menubar\|sidebar\.collapsed\|session-head\|system-dock" src --include=*.css --include=*.tsx | head -30
```

删除确认不再被引用的规则与文件；`grep -rn "TODO\|FIXME" src` 检查没有留下待办。

- [ ] **Step 4: 写重做记录（旧平台仓库）**

在 `D:\code\ai\docs\rust-rewrite\` 新增 `39-ui-redesign-batch1.md`：批次一的范围、行为变化（会话行单击语义）、新目录结构、明暗基线截图路径、发现的偏离项（demo 与后端能力不一致处）、批次二待办清单。在旧平台仓库提交：

```bash
cd /d/code/ai && git add docs/rust-rewrite/39-ui-redesign-batch1.md && git commit -m "docs: aivc-rs 界面重做批次一记录"
```

- [ ] **Step 5: 批次一提交汇总核对**

```bash
cd ui && git log --oneline -12 && git status --porcelain
```

预期：工作区干净；12 个提交左右（每任务 1–2 个），信息可读。

---

## 批次二 / 批次三纲要（各自开始时另写完整计划）

**批次二（设置中心 10 面板重排 + 其余弹框统一）** —— 任务草案：
1. 供应商面板：表格列按 demo 重排（名称/端点、厂商、费率、健康延时、候选池状态、操作）+ 工具条四按钮 + 行内余额状态；
2. 网关面板：三卡重排（端口与熔断韧性集中保存 / 外部网关与端点清单与认证说明 / 故障规则 + 在线测试引擎）+ 总开关胶囊；
3. 用量 / 异常 / 日志面板：面板头 + 范围选择 + 汇总卡 + 表格统一；日志页的「清空全部日志」视觉收口；
4. 技能 / 环境 / 预设 / 诊断面板：面板头统一 + 卡片化（环境页含 WSL 会话目录卡）；
5. 其余弹框统一：新建会话 / 重命名 / 复制 / 会话设置 / 退出确认 / 关注中心 / 右键菜单（三种）/ 导出提示；
6. 收尾：设置中心逐页截图（明暗）+ 门禁。

**批次三（收尾）** —— 任务草案：
1. 暗色逐屏走查与修正（含终端、卡片、状态点、徽章在暗色下的对比度）；
2. 死代码与类型清理（含删除 `session-row.tsx` 中不再使用的导出）；
3. 重建基线集（`shots/baseline-light/`、`shots/baseline-dark/`，旧基线留档）；
4. 更新 `docs/rust-rewrite/39-…` 或新增记录：完成状态、遗留项、与 demo 的差异清单。

## 计划自查记录（写作时已完成）

- **Spec 覆盖**：§5 设计系统→T1/T2；§6 骨架→T6；§7 会话侧栏→T7；§8 工作台→T8；§9.1 标签模型→T3/T4；§9.2 画布→T9；§9.3 输入区与模型选择→T10；§9.4 消息与分支→T11；§10 设置中心→T5（外壳）+ 批次二（面板重排）；§11 入口迁移→T5/T6；§12 修复 B1→T5、B2→T1、B3 随批次；§13 验证→每任务 + T12；§14 批次划分→文首总览 + 纲要。
- **类型一致性**：`WorkspaceTabLike`（T3）→ `WorkspaceTab`（T4）→ `WorkspaceTabItem`（T4 产出、T9 消费）；`StageContent`（T3 产出、T4 接线、T9 渲染）；`SettingsTab`（T5 产出、T5 消费）；`TerminalUiState`（T6 产出、T6 状态栏消费）；`resolveModelSource`（T10 产出、T10 消费）。命名均与本计划各任务块一致。
- **占位符扫描**：无 TBD/TODO；所有代码步骤含可执行代码或精确的既有代码锚点（文件 + 行号/函数名）；图标路径给全。
- **与 spec 的两处有意偏差（已确认可接受）**：① spec §5.4 列了 `Tag`，本计划不单独实现——它的用法（平台标识）由 `Badge` 覆盖，等真正出现「更小的标签」需求再拆；② spec §10 要求面板按 demo 重排 + `PanelHeader`，批次一只做外壳与关于页（`PanelHeader` 已建），十个面板的重排整体放在批次二，避免同一批里既改骨架又改十个面板、验证面失控。
- **已知取舍（执行者需知）**：T5 的 `SettingsCenter` 里 `sessionSettings` 相关 props 采用「与环境面板的 `onOpenSessionSettings` 打通、对话框本体不动」的方案；T9 的 `findSignal` 用计数器而非事件总线（一个组件内的两个入口，不需要更重的机制）。
