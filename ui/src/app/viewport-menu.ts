import { useLayoutEffect, useRef, useState } from "react";

/**
 * 把浮层夹在视口内。
 *
 * # 为什么需要它
 *
 * 右键菜单是按鼠标位置开在 `position: fixed` 上的。在靠近右边缘或下边缘的地方右键，
 * 菜单会有一半跑到视口外——那些项点不到，而用户看不出是为什么。
 *
 * # 为什么是 `useLayoutEffect`
 *
 * 菜单的尺寸要渲染完才量得到，而夹取必须在浏览器绘制**之前**完成，
 * 否则会先看到菜单在错的位置上闪一下再跳回来。
 *
 * # 为什么抽成独立文件
 *
 * 会话菜单、标签菜单、终端文本菜单三处都要同一件事。
 * 抄三份的后果是只有一份会被修（例如加了窗口 resize 的监听），
 * 而另外两处在窗口缩小之后又开始被切掉。
 */
export function useViewportMenuPosition(x: number, y: number) {
  const menuRef = useRef<HTMLDivElement | null>(null);
  const [position, setPosition] = useState({ x, y });

  useLayoutEffect(() => {
    const clamp = () => {
      const element = menuRef.current;
      if (!element) return;
      const rect = element.getBoundingClientRect();
      const margin = 8;
      const maxX = Math.max(margin, window.innerWidth - rect.width - margin);
      const maxY = Math.max(margin, window.innerHeight - rect.height - margin);
      setPosition({
        x: Math.min(Math.max(x, margin), maxX),
        y: Math.min(Math.max(y, margin), maxY)
      });
    };
    clamp();
    // 窗口缩小之后原来的位置可能又跑出去了，因此跟着 resize 重算。
    window.addEventListener("resize", clamp);
    return () => window.removeEventListener("resize", clamp);
  }, [x, y]);

  return { menuRef, position };
}
