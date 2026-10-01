import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./app/app";
import { ErrorBoundary } from "./app/error-boundary";
// 顺序有意义：令牌 → 组件 → 应用。后者可以覆盖前者的布局，反过来则不行。
import "./styles/tokens.css";
import "./components/components.css";
import "./app/app.css";

const container = document.getElementById("root");
if (!container) {
  // 入口页写错时，静默失败会表现为「窗口一片空白」——那是最难查的一种。
  throw new Error("找不到 #root 容器：index.html 与入口脚本不匹配");
}

createRoot(container).render(
  <StrictMode>
    {/* 整个界面崩掉时也要有出口：此时菜单之外唯一能点的就是兜底里的「重试」。 */}
    <ErrorBoundary label="界面出错">
      <App />
    </ErrorBoundary>
  </StrictMode>
);
