//! Tauri 构建脚本。
//!
//! `tauri_build::build()` 读取 `tauri.conf.json`，把窗口尺寸、标题、`frontendDist`
//! 等配置编成 `generate_context!()` 需要的元数据。**没有它 `generate_context!` 无法编译**，
//! 因此这个文件是「能启动出窗口」的必要条件。
//!
//! 注意：`tauri.conf.json` 的 `frontendDist` 指向**本工程自己的前端产物**
//! （`ui/dist`）。前端源码在 `ui/`，用 Vite 构建；`cargo build` **不会**替你构建它
//! ——改了界面要先在 `ui/` 里跑 `npm run build`。

fn main() {
    tauri_build::build();
}
