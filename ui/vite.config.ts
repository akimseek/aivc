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
    // 产物会被**嵌进二进制**（`frontendDist`），sourcemap 在这里只是白增体积
    // （主包一份 map 就近 2 MB）。开发态由 dev server 的 map 覆盖；真要定位
    // 构建产物时临时改成 "hidden" 再发一版调试包。
    sourcemap: false
  },
  server: {
    // 与旧工程（5173）错开，避免同时开着两个 dev server 时端口打架。
    port: 5273,
    strictPort: true
  }
});
