import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./app/app";
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
    <App />
  </StrictMode>
);
