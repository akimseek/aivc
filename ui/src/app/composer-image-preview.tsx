import { useCallback, useEffect, useState } from "react";

import { Button, Dialog } from "../components";
import * as api from "../ipc/commands";
import { clampZoom, stepZoom, zoomLabel, type ComposerAttachment } from "./composer-attachments";

/**
 * 图片预览。
 *
 * # 与旧平台一致的四件事
 *
 * **放大 / 缩小 / 复制 / 关闭**。旧平台还有一个「关闭」在旁边，这里保留。
 *
 * # 为什么要单独一个弹层而不是在列表里放大
 *
 * 附件列表在输入框上方，只有几十像素高。图看不清楚，而「这张图我挑对了吗」
 * 正是要放大才能回答的问题。
 *
 * # 图片数据从哪来
 *
 * 不在附件里存 data URL：几十张图全存着会把内存吃掉，而且列表里只用得到缩略图。
 * 打开预览时才向后端要一次（`shell:read-image`），关掉就丢。
 */
export function ComposerImagePreview({
  attachment,
  onClose,
  onError
}: {
  attachment: ComposerAttachment;
  onClose: () => void;
  onError: (message: string) => void;
}) {
  const [zoom, setZoom] = useState(1);
  const [dataUrl, setDataUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState("");
  /** 复制成功的一次性提示——复制是「做完没有任何反馈」的典型动作。 */
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void api
      .readImage(attachment.path)
      .then((result) => {
        // 关得快时不要往里塞数据：那时组件已经没了。
        if (!cancelled) setDataUrl(result.dataUrl);
      })
      .catch((problem: unknown) => {
        if (!cancelled) {
          setFailed(problem instanceof Error ? problem.message : String(problem));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [attachment.path]);

  const copyImage = useCallback(() => {
    void api
      .copyImage(attachment.path)
      .then(() => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), 2000);
      })
      .catch((problem: unknown) =>
        onError(problem instanceof Error ? problem.message : String(problem))
      );
  }, [attachment.path, onError]);

  return (
    <Dialog
      title={`预览 ${attachment.name}`}
      focusPanel={false}
      onClose={onClose}
      panelClassName="composer-preview-panel"
      footer={
        <>
          <Button small onClick={() => setZoom((value) => stepZoom(value, "out"))} title="缩小">
            缩小
          </Button>
          <span className="row-meta">{zoomLabel(zoom)}</span>
          <Button small onClick={() => setZoom((value) => stepZoom(value, "in"))} title="放大">
            放大
          </Button>
          <Button small onClick={() => setZoom(1)} title="恢复原始大小">
            原始大小
          </Button>
          <span className="spacer" />
          <span className="row-meta">{copied ? "已复制到剪贴板" : ""}</span>
          <Button small onClick={copyImage} title="把这张图放进系统剪贴板">
            复制
          </Button>
          <Button small onClick={onClose}>
            关闭
          </Button>
        </>
      }
    >
      <div className="composer-preview-stage">
        {failed ? (
          // 读失败要**说清是读失败**，而不是显示一块空白让人以为图就是这样。
          <p className="row-meta">{failed}</p>
        ) : dataUrl ? (
          <img
            src={dataUrl}
            alt={attachment.name}
            style={{ transform: `scale(${clampZoom(zoom)})` }}
          />
        ) : (
          <p className="row-meta">正在读取…</p>
        )}
      </div>
      {/* 路径写出来：用户常常要把它复制到别处，而预览是最容易想到来这里找的地方。 */}
      <p className="row-meta mono" title={attachment.path}>
        {attachment.path}
      </p>
    </Dialog>
  );
}
