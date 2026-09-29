import { useState } from "react";

import * as api from "../ipc/commands";
import type { AiSession, AiTarget } from "../types";
import { BranchPanel } from "./branch-panel";
import { SessionDetailDialog } from "./session-detail-dialog";
import { useSessionDetail } from "./use-session-detail";

/**
 * 会话详情的**自主容器**。
 *
 * # 为什么它自己持有状态，而不是让 `app.tsx` 管
 *
 * `app.tsx` 已经要管标签、弹层、导出、命令面板…再往里塞一个「详情开着哪一条、
 * 第几页、每页多少条、加载中」会让它回到 1400 行那种状态。
 *
 * 这里只暴露两个东西：**当前要看哪一条**（受控）与**怎么关**。
 * 分页、取数、分支面板全在里面，`app.tsx` 不需要知道。
 *
 * # 为什么「打开详情」是受控的
 *
 * 打开的入口有两处（会话头的「详情」按钮、以后可能有的右键菜单），
 * 而关闭之后要**回到原来的视图**——所以「开着哪一条」这个事实必须由上层持有，
 * 否则关掉之后上层不知道要不要恢复什么。
 */
export function SessionDetailContainer({
  session,
  target,
  onClose,
  onOpenSession,
  resolveParent,
  onBranch,
  canBranch = true,
  onError
}: {
  /** 要看的会话；`null` 表示不打开。 */
  session: AiSession | null;
  target: AiTarget | null;
  onClose: () => void;
  /** 从分支面板打开另一条会话。 */
  onOpenSession: (session: AiSession) => void;
  /** 从已加载列表里找父会话。 */
  resolveParent: (sessionId: string) => AiSession | undefined;
  onBranch: (sourceLine: number) => void;
  /** 平台支不支持分支。原样往下传，见 `SessionDetailDialog`。 */
  canBranch?: boolean;
  onError?: (message: string) => void;
}) {
  const [pageSize, setPageSize] = useState(50);
  const detail = useSessionDetail(target, session, pageSize);

  if (!session) return null;

  return (
    <>
      {detail.error && onError && (
        // 读不到时**说一声**再继续显示空列表：静默失败会让用户以为会话真的没有消息。
        <SessionDetailError message={detail.error} />
      )}
      <SessionDetailDialog
        session={session}
        messages={detail.messages}
        // 后端的 `messageCount` 为 0 表示「本次无从得知」，那时退回列表里的下界。
        total={detail.total || session.messageCount}
        page={detail.page}
        pageSize={pageSize}
        loading={detail.loading}
        onPageChange={detail.setPage}
        onPageSizeChange={(size) => {
          setPageSize(size);
          detail.setPage(1);
        }}
        onReload={detail.reload}
        onBranch={(sourceLine) => {
          onBranch(sourceLine);
          onClose();
        }}
        canBranch={canBranch}
        onClose={onClose}
        branchPanel={
          <BranchPanel
            session={session}
            resolveParent={resolveParent}
            listChildren={(parentSessionId) =>
              api.listSessionChildren(parentSessionId, target?.id)
            }
            onOpen={(next) => {
              onClose();
              onOpenSession(next);
            }}
          />
        }
      />
    </>
  );
}

/**
 * 详情读取失败时的一句话。
 *
 * 它不是弹层、也不阻塞：面板照常显示空列表，而这句话说明**为什么是空的**。
 * 反过来（把失败显示成「没有消息」）会让用户以为那条会话真的没内容。
 */
function SessionDetailError({ message }: { message: string }) {
  return (
    <div className="export-hint" role="status">
      会话详情读取失败：{message}
    </div>
  );
}
