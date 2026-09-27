import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// 本工程的前端。产物落在 `ui/dist`，由 `crates/aivc-app/tauri.conf.json` 的
// `frontendDist` 指向——**不再借用旧工程的 `dist/renderer`**。
export default defineConfig({
  plugins: [react()],
  // 相对路径：Tauri 用自定义协议加载页面，以 `/` 开头的绝对路径会 404。
  base: "./",
  build: {
    outDir: "dist",
    emptyOutDir: true,
    // 出问题时能看出是哪个模块，值得这几百 KB。
    sourcemap: true
  },
  server: {
    // 与旧工程（5173）错开，避免同时开着两个 dev server 时端口打架。
    port: 5273,
    strictPort: true
  }
});
