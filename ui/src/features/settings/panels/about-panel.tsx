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
        {/* 退出走浏览器窗口关闭（与 app.tsx 原「退出」动作同一条路），
            后端因此仍有机会弹退出确认——两套退出语义是不行的。 */}
        <Button variant="danger" onClick={onQuit} title="退出应用（会先确认）">
          退出控制台
        </Button>
      </div>
    </div>
  );
}
