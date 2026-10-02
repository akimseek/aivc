# 47 · 构建与打包操作手册

> 2026-10-02。**照着做就能出安装包。** 44 号记的是我们这边的实测过程与坑；本手册是
> 面向「自己动手打包」的操作页。后续不再代打包。

## 一、前置（一次性）

| 依赖 | 说明 |
| --- | --- |
| Rust 工具链 | 仓库内自带隔离工具链 `.toolchain/`（cargo/rustup + `stable-x86_64-pc-windows-msvc`）。**换机器**时按普通 rustup 装 MSVC target 即可，并去掉脚本里的隔离路径。 |
| Node ≥ 20.19 | `ui/` 前端构建（engines 已声明；根 `.nvmrc` 为 22）。 |
| 网络 | npm registry 可达（`npx` 拉 tauri CLI）。**GitHub 直连不可达时**打包工具链走镜像，见下。 |
| 打包工具（首次自动下载） | tauri CLI 会下载 NSIS 3.11 / WiX 3.14 到 `%LOCALAPPDATA%\tauri\`（首次构建需网络，之后离线可复现）。 |

## 二、三步构建（手动路径）

```bash
cd D:/code/ai/aivc-rs

# 步骤 0：隔离工具链环境（本机）
export CARGO_HOME='D:\code\ai\aivc-rs\.toolchain\cargo'
export RUSTUP_HOME='D:\code\ai\aivc-rs\.toolchain\rustup'
export RUSTUP_TOOLCHAIN='stable-x86_64-pc-windows-msvc'
export PATH="/d/code/ai/aivc-rs/.toolchain/cargo/bin:$PATH"
# 只想离线构建时再加上：
export CARGO_NET_OFFLINE=true

# 步骤 1：前端产物（tsc 类型检查 + vite 构建 → ui/dist）
cd ui && npm install        # 首次
npm run build

# 步骤 2：release 二进制（前端产物是**编译期嵌入**的，必须先做步骤 1）
cd .. && cargo build --release -p aivc-app --offline
#   → target/release/AI.exe（双击即可运行：数据写在自己旁边）

# 步骤 3：安装包（会自动重跑步骤 1 的前端构建，再 release 编译、再打 nsis + msi）
cd crates/aivc-app
npx --yes @tauri-apps/cli@^2 build
```

> 只想要安装包时，**直接做步骤 3 就行**——`tauri.conf.json` 配了
> `beforeBuildCommand`，前端与 release 编译都由它接管。分成三步是为了「只出裸
> 二进制快速试跑」的场景。

**镜像（GitHub 不可达时）**：给步骤 3 追加一个环境变量——

```bash
export TAURI_BUNDLER_TOOLS_GITHUB_MIRROR='https://gh-proxy.com'
```

CLI 会把工具下载请求改写成 `https://gh-proxy.com/https://github.com/…` 的形状。

**只打一种安装器**：`npx --yes @tauri-apps/cli@^2 build --bundles nsis`（或 `msi`）。

## 二·补、应用图标（可选）

图标源是矢量文件 `crates/aivc-app/icons/icon.svg`（与界面品牌渐变同源）。改完源文件后，
在 **`crates/aivc-app` 目录下**重生成全平台格式（ico / icns / 各档 PNG）：

```bash
cd crates/aivc-app
npx --yes @tauri-apps/cli@^2 icon icons/icon.svg
```

生成后需重新构建二进制/安装包才会带上新图标（图标嵌在 exe/应用包资源里）。

## 三、产物与安装形态

| 产物 | 路径 | 形态 |
| --- | --- | --- |
| 裸二进制 | `target/release/AI.exe` | 双击可跑；数据写在自己旁边（可写时） |
| NSIS 安装器 | `target/release/bundle/nsis/AI 可视化控制台_0.1.0_x64-setup.exe` | **per-user**：装 `%LOCALAPPDATA%\AI 可视化控制台`，**免管理员**，数据写在安装目录旁 |
| MSI 安装器 | `target/release/bundle/msi/AI 可视化控制台_0.1.0_x64_zh-CN.msi` | **per-machine**：装 `C:\Program Files\…`，需管理员；数据目录不可写时**自动回退** `%LOCALAPPDATA%\aivc` |

- WebView2 采用默认「下载式引导」：缺 WebView2 的机器在**安装阶段**需要联网。
- 安装包体积量级：nsis ≈ 5.5 MiB / msi ≈ 8 MiB（2026-10-02 的 0.1.0）。

## 四、改版本号

两处一起改（保持一致）：`crates/aivc-app/tauri.conf.json` 的 `"version"` 与根
`Cargo.toml` 里 workspace 的 `version`。产物文件名会跟着变。

## 五、实测过的坑（照做可避）

1. **中文文件名的 MSI + 提权静默安装**：命令行参数会被编码打坏（msiexec 报 1639）。
   要提权静默装就先把 msi 复制成 **ASCII 名**（如 `aivc.msi`）再装；双击图形安装没有
   这个问题。
2. **先装 NSIS、卸载、再装 MSI**：MSI 会读 `HKCU\Software\aivc\<产品名>` 里
   **NSIS 记忆的安装目录**并装到那里（模板的 RegistrySearch；NSIS 卸载不会清这个
   键）。想让它落回 Program Files，先删这个键。
3. **数据目录权限**：exe 所在目录不可写时（Program Files 的常态），应用会探测并把
   `data/`、`logs/` 回退到 `%LOCALAPPDATA%\aivc`（日志里有一条 warn 记录两处路径）。
   想验证回退：装 MSI 后普通用户启动，看 `%LOCALAPPDATA%\aivc\` 是否出现。
4. **先改前端再只跑 `cargo build`**：前端是**编译期嵌入**的——只跑 cargo 不会带上
   新界面。要么先 `npm run build`，要么直接走 `tauri build`（它会代跑）。
5. 自写辅助 `.ps1` 时必须 **UTF-8 BOM**（PowerShell 5.1 对无 BOM 中文注释会在行尾
   「併吞」下一行；`verify.ps1` 有专门检查）。

## 六、提交前的自检（可选但推荐）

```bash
cd ui && npx tsc --noEmit && npx vitest run && npm run build   # 前端三连
cd .. && powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify.ps1 -Quick
#   ↑ 八段：BOM、探针选择器、IPC 命令名、fmt、分层、check、clippy×2
```

完整的 workspace 测试：`cargo test --workspace --offline`（隔离工具链环境下）。
