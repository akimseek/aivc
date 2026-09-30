import { Icon } from "../components";

/**
 * 极窄活动栏（50px）：两个一级去处 + 底部的设置入口。
 *
 * # 为什么一级去处只有两个
 *
 * 「工作台」与会话列表是同一批数据的两种看法，此前它们挤在侧栏的 chip 里
 * （一级导航与二级筛选长得很像）。搬到活动栏之后**层级一眼可辨**：
 * 这里换的是整块侧栏，不是列表的筛选。
 *
 * 设置放在底部：它是低频、全局的，与上面两个「看什么」不是一类。
 */
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
