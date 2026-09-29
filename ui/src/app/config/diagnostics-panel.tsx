import { EmptyState } from "../../components";
import type { AiTarget } from "../../types";

/**
 * 诊断：把「每一道门的结果」集中到一处。
 *
 * 这一页存在的理由很具体：**用户遇到问题时，界面上通常只有一句结论**，
 * 而真正有用的是「哪些目标可用、各自为什么不可用」。这些信息本来就有
 * （目标自带 `detail`），只是一直没有一个地方把**全部目标**并排列出来。
 */
export function DiagnosticsPanel({
  targets,
  selectedTargetId
}: {
  targets: AiTarget[];
  selectedTargetId: string;
}) {
  if (targets.length === 0) {
    return <EmptyState message="还没有探测到任何目标。" hint="先在上方选择平台。" />;
  }
  return (
    <>
      <div className="row-meta">
        共 {targets.length} 个目标 · {targets.filter((target) => target.available).length} 个可用
      </div>
      <table className="data-table">
        <thead>
          <tr>
            <th>目标</th>
            <th>类型</th>
            <th>状态</th>
            <th>探测结果</th>
          </tr>
        </thead>
        <tbody>
          {targets.map((target) => (
            <tr key={target.id}>
              <td>
                {target.label}
                {target.id === selectedTargetId && <span className="tag ok">当前</span>}
              </td>
              <td>{target.kind === "wsl" ? `WSL · ${target.distro ?? "-"}` : "本机"}</td>
              <td>
                <span className={`tag ${target.available ? "ok" : "off"}`}>
                  {target.available ? "可用" : "不可用"}
                </span>
              </td>
              {/* `detail` 是后端给出的四态说明，**原样显示**——它已经是最好的解释。 */}
              <td className="mono">{target.detail ?? "-"}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}
