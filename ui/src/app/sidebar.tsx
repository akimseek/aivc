import { useState } from "react";

import {
  Button,
  ChipGroup,
  EmptyState,
  Field,
  List,
  Notice,
  SearchInput,
  Select,
  TextInput,
  Toolbar,
  ToolbarSpacer
} from "../components";
import type { AiTarget } from "../types";
import { emptySessionMessage } from "./session-display";
import { QuickGroup, SearchResultRow, SessionRow } from "./session-row";
import type { useConsole } from "./use-console";

/** 侧栏需要的那部分 console 状态与动作。用具名类型而不是 `any`，字段改名时会编译报错。 */
export type SidebarConsole = Pick<
  ReturnType<typeof useConsole>,
  | "busy"
  | "providers"
  | "providerId"
  | "selectProvider"
  | "targets"
  | "targetId"
  | "selectTarget"
  | "pane"
  | "setPane"
  | "view"
  | "setView"
  | "query"
  | "setQuery"
  | "searching"
  | "sessions"
  | "selectedId"
  | "selectedIds"
  | "toggleSelected"
  | "toggleSelectAll"
  | "toggleFavorite"
  | "reloadTargets"
  | "configureWslCodexHome"
  | "clearWslCodexHomeOverride"
  | "quickAccess"
  | "globalQuery"
  | "setGlobalQuery"
  | "globalSearching"
  | "searchTargetId"
  | "setSearchTargetId"
  | "globalResults"
  | "restoreSelected"
  | "purgeSelected"
  | "deleteSelected"
  | "toggleSelectAll"
>;

/**
 * 侧栏。
 *
 * # 为什么把它整个抽出来
 *
 * `app.tsx` 里这一块有 **260 行内联 JSX**（平台/目标选择、目标状态说明、
 * 两级导航、三种列表、批量条、空状态）。它和「工作区」是两件事，
 * 混在一个文件里既读不下去，也没法单独测。
 *
 * 抽出来之后 `app.tsx` 只剩「谁在什么时候调什么」，而这一块可以按
 * 「给了这些状态，渲染出什么」来断言。
 *
 * # 三种 pane 的空状态各不相同
 *
 * 这一点必须显式写出来：`sessions` 的空要说清「没选目标 / 目标进不去 / 真没有」，
 * `quick` 的空要说「还没有收藏」，`search` 的空要说「输入关键词」。
 * 一句笼统的「没有数据」会让用户不知道下一步做什么。
 *
 * # 行点击只回传 `sessionId`
 *
 * 抽成 `onResume(target, sessionId)` 而不是把 `AiSession` 整个递出去：
 * 侧栏不关心会话内容，只关心「用户点了哪一条」。
 */
export function Sidebar({
  console: state,
  selectedTarget,
  onResume,
  onMenu
}: {
  console: SidebarConsole;
  selectedTarget: AiTarget | null;
  onResume: (target: AiTarget, sessionId: string) => void;
  onMenu: (x: number, y: number, sessionId: string) => void;
}) {
  const trash = state.view === "trash";
  const selectedCount = state.selectedIds.length;
  const quickCount = state.quickAccess.favorites.length + state.quickAccess.recent.length;

  return (
    <aside className="sidebar">
      <div className="sidebar-head">
        <div className="title">
          <span className={`status-dot${state.busy ? " off" : ""}`} />
          <h1>会话</h1>
        </div>
      </div>

      <div className="pickers">
        <Field label="平台">
          <Select
            value={state.providerId}
            onChange={(event) => state.selectProvider(event.target.value as never)}
          >
            {state.providers.length === 0 && <option value="">加载中…</option>}
            {state.providers.map((provider) => (
              <option key={provider.id} value={provider.id}>
                {provider.label}
              </option>
            ))}
          </Select>
        </Field>

        <Field label="目标">
          <Select
            value={state.targetId}
            disabled={state.targets.length === 0}
            onChange={(event) => state.selectTarget(event.target.value)}
          >
            {state.targets.length === 0 && <option value="">暂无可用目标</option>}
            {state.targets.map((target) => (
              <option key={target.id} value={target.id}>
                {/* 不可用的目标在选择列表里就标出来——等选中了才发现，用户已经先点了一次。 */}
                {target.available ? target.label : `${target.label}（不可用）`}
              </option>
            ))}
          </Select>
        </Field>

        {/* 目标状态说明。后端给出了四态 detail，界面必须显示：
            否则用户只看到「列表是空的」，分不清是**没有会话**还是**这个目标进不去**。 */}
        {selectedTarget && (
          <div className={`target-detail${selectedTarget.available ? "" : " bad"}`}>
            <span title={selectedTarget.detail ?? ""}>
              {selectedTarget.detail ?? "（无状态说明）"}
            </span>
          </div>
        )}
      </div>

      {selectedTarget && !selectedTarget.available && (
        <div className="sidebar-notice">
          <Notice
            message={selectedTarget.detail || "此目标当前不可用。"}
            actions={
              <>
                <Button small onClick={() => void state.reloadTargets()}>
                  重新探测
                </Button>
                {selectedTarget.kind === "wsl" && selectedTarget.distro && (
                  <WslHomeForm
                    distro={selectedTarget.distro}
                    onSave={(home) =>
                      void state.configureWslCodexHome(selectedTarget.distro!, home)
                    }
                    onClear={() => void state.clearWslCodexHomeOverride(selectedTarget.distro!)}
                  />
                )}
              </>
            }
          />
        </div>
      )}

      <div className="list-head">
        <Toolbar>
          {/* 一级导航。**包一层**是为了在 CSS 里和二级拉开轻重——两级用的是同一个
              `ChipGroup`，不区分的话看上去完全一样，用户分不出哪层是哪层。 */}
          <div className="pane-nav">
            <ChipGroup
              value={state.pane}
              onChange={(value) => state.setPane(value as "sessions" | "quick" | "search")}
              options={[
                { value: "sessions", label: "会话" },
                { value: "quick", label: "快捷" },
                { value: "search", label: "搜索" }
              ]}
            />
          </div>
          <ToolbarSpacer />
          <span className="row-meta">
            {state.pane === "sessions"
              ? `${state.sessions.length} 条`
              : state.pane === "quick"
                ? `${quickCount} 条`
                : `${state.globalResults.length} 条`}
          </span>
        </Toolbar>

        {state.pane === "sessions" && (
          <Toolbar>
            {/* 二级视图。比一级轻一档：它是**同一批内容的筛选**，不是另一处地方。 */}
            <div className="view-nav">
              <ChipGroup
                value={state.view}
                onChange={(value) => state.setView(value === "trash" ? "trash" : "active")}
                options={[
                  { value: "active", label: "活跃" },
                  { value: "trash", label: "回收站" }
                ]}
              />
            </div>
          </Toolbar>
        )}

        {state.pane === "sessions" && (
          <SearchInput
            value={state.query}
            onChange={state.setQuery}
            placeholder="搜索标题、编号、正文…"
            busy={state.searching}
          />
        )}

        {state.pane === "search" && (
          <>
            <SearchInput
              value={state.globalQuery}
              onChange={state.setGlobalQuery}
              placeholder="在所有目标里搜索…"
              busy={state.globalSearching}
            />
            {/* 范围收窄：不只是少显示结果，而是**少发请求**——
                全量搜完再在前端过滤掉，那些请求已经付过代价了。 */}
            <Field label="搜索范围">
              <Select
                value={state.searchTargetId}
                onChange={(event) => state.setSearchTargetId(event.target.value)}
              >
                <option value="">全部目标（{state.targets.length}）</option>
                {state.targets.map((target) => (
                  <option key={target.id} value={target.id}>
                    {target.available ? target.label : `${target.label}（不可用）`}
                  </option>
                ))}
              </Select>
            </Field>
          </>
        )}
      </div>

      {state.pane === "sessions" && selectedCount > 0 && (
        <div className="batchbar">
          <span>已选 {selectedCount} 项</span>
          <ToolbarSpacer />
          <Button small onClick={state.toggleSelectAll}>
            全选
          </Button>
          {trash ? (
            <>
              <Button small onClick={() => void state.restoreSelected()}>
                还原
              </Button>
              <Button small variant="danger" onClick={() => void state.purgeSelected()}>
                彻底删除
              </Button>
            </>
          ) : (
            <Button small variant="danger" onClick={() => void state.deleteSelected()}>
              移入回收站
            </Button>
          )}
        </div>
      )}

      <List
        empty={
          <>
            {state.pane === "sessions" && state.sessions.length === 0 && (
              <EmptyState
                message={
                  state.query.trim()
                    ? "没有匹配的会话。"
                    : emptySessionMessage(state.targetId, selectedTarget, trash)
                }
              />
            )}
            {state.pane === "search" && state.globalResults.length === 0 && (
              <EmptyState
                message={
                  state.globalQuery.trim()
                    ? "没有匹配的会话。"
                    : "输入关键词，在所有目标里搜索。"
                }
                hint={
                  state.globalQuery.trim()
                    ? "后端会先匹配摘要，摘要不中再读整个文件——因此结果可能稍慢。"
                    : undefined
                }
              />
            )}
          </>
        }
      >
        {state.pane === "sessions" &&
          state.sessions.map((session) => (
            <SessionRow
              key={session.id}
              session={session}
              active={session.id === state.selectedId}
              checked={state.selectedIds.includes(session.id)}
              onOpen={() => {
                if (selectedTarget) onResume(selectedTarget, session.id);
              }}
              onToggle={() => state.toggleSelected(session.id)}
              onFavorite={() => void state.toggleFavorite(session)}
              onMenu={(x, y) => onMenu(x, y, session.id)}
            />
          ))}

        {state.pane === "quick" && (
          <>
            <QuickGroup
              title="收藏"
              items={state.quickAccess.favorites}
              empty="还没有收藏。在会话行上点 ☆ 收藏它。"
              onOpen={(target, session) => onResume(target, session.id)}
            />
            <QuickGroup
              title="最近打开"
              items={state.quickAccess.recent}
              empty="还没有打开过会话。"
              onOpen={(target, session) => onResume(target, session.id)}
            />
          </>
        )}

        {state.pane === "search" &&
          state.globalResults.map((result) => (
            <SearchResultRow
              key={`${result.target.id}:${result.session.id}`}
              target={result.target}
              session={result.session}
              onOpen={() => onResume(result.target, result.session.id)}
            />
          ))}
      </List>
    </aside>
  );
}

/**
 * 手动指定 WSL 内的 Codex 目录。
 *
 * # 为什么它必须出现在「目标不可用」的告警里
 *
 * 发行版进不去时，自动探测**注定失败**；而用户往往知道自己的 Codex 装在哪。
 * 这条路径此前只藏在菜单里（甚至压根没有入口），于是「目标不可用」变成了死路。
 *
 * 保存后必须**重新探测**——否则界面还停在旧结果上，用户会以为没生效。
 */
function WslHomeForm({
  distro,
  onSave,
  onClear
}: {
  distro: string;
  onSave: (codexHome: string) => void;
  onClear: () => void;
}) {
  const [draft, setDraft] = useState("");
  return (
    <div className="wsl-home">
      <TextInput
        value={draft}
        placeholder={`${distro} 内的路径，例如 /home/你/.codex`}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && draft.trim()) onSave(draft.trim());
        }}
      />
      <div className="wsl-home-actions">
        <Button small disabled={!draft.trim()} onClick={() => onSave(draft.trim())}>
          指定并重新探测
        </Button>
        <Button small onClick={onClear} title="清除人工指定，回到自动探测">
          清除
        </Button>
      </div>
    </div>
  );
}
