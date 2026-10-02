# 44 · P8 打包：Windows 安装包实测

> 2026-10-02。承接 42 号 §五.5（「打包前置已就位，`bundle.active` 仍为 `false`」）。
> 本批把打包从「配置就绪」推到「**安装包实测产出**」。
> 范围：本机可验的 **Windows nsis + msi**；dmg / AppImage / deb 与性能基准属 P8 的
> 其余部分，环境不具备（需要对应平台的构建机），见 §五。

## 一、交付物与核对

| 产物 | 路径（`aivc-rs/` 下） | 体积 | SHA256 |
| --- | --- | --- | --- |
| NSIS 安装器 | `target/release/bundle/nsis/AI 可视化控制台_0.1.0_x64-setup.exe` | 5,820,772 B（5.55 MiB） | `0d6c2e24…f3c1` |
| MSI 安装器 | `target/release/bundle/msi/AI 可视化控制台_0.1.0_x64_zh-CN.msi` | 8,351,744 B（7.96 MiB） | `35321dfe…c038` |
| 裸二进制（同一份代码） | `target/release/aivc-app.exe` | 22,178,816 B（21.2 MiB） | —— |

> 安装包随代码演进会按 §三 配方重打；上表是首轮 P8 构建的产物快照。行为修正
> （45 号：单击会话行=进入终端）之后重打过一轮，哈希见 45 号 §三。

对照 `02-rust-architecture.md` §11「安装包体积 ≤ 15 MB」：**两个安装器都在目标内**
（NSIS 5.55 MiB / MSI 7.96 MiB；体积差来自压缩率——裸 exe 21.2 MiB）。

## 二、配置定稿（aivc-rs commit `6fb138c`，`crates/aivc-app/tauri.conf.json`）

- `bundle.active: true`（原 `false`）；`targets: "all"`——Windows 上展开为 nsis + msi。
- `build.beforeBuildCommand: "npm --prefix ../ui run build"`——打包前自动重建前端，
  杜绝「改了前端忘了重建、安装包里是旧界面」。
- `bundle.windows.wix.language: "zh-CN"`——理由见 §四.3。
- 图标沿用 P2-A 补齐的 `icon.ico` + `icon.icns` + 三档 PNG。

## 三、构建配方（可复现）

> 面向「自己动手打包」的操作页见 `47-build-and-package.md`（2026-10-02 起打包由用户自行执行，本仓库不再代打）。

```bash
cd aivc-rs/crates/aivc-app
# 工作区隔离工具链（与 verify.ps1 同一套）+ 离线
export CARGO_HOME='D:\code\ai\aivc-rs\.toolchain\cargo'
export RUSTUP_HOME='D:\code\ai\aivc-rs\.toolchain\rustup'
export RUSTUP_TOOLCHAIN='stable-x86_64-pc-windows-msvc'
export CARGO_NET_OFFLINE=true
# GitHub 直连不可达时（本机实测）：给 tauri 打包工具的下载套镜像
export TAURI_BUNDLER_TOOLS_GITHUB_MIRROR='https://gh-proxy.com'
npx --yes @tauri-apps/cli@^2 build          # CLI 2.12.1；tauri crate 2.11.6
```

- 只打某一种：`--bundles nsis` 或 `--bundles msi`（可分次；产物同标准 `all` 流程）。
- 打**包工具**由 CLI 自动下载并缓存于 `%LOCALAPPDATA%\tauri\`（NSIS、WixTools314），
  首次构建需网络；之后离线可复现。
- 本仓库没有 `src-tauri`：app 目录就是 `crates/aivc-app`，在**该目录内**执行 CLI
  （或在根目录用 `--config` 指路）。

## 四、三个实测踩过并修复的坑（按踩坑顺序）

1. **beforeBuildCommand 的相对路径落错地方**：本仓库布局（app 目录 = `crates/aivc-app`，
   无 `src-tauri`）下，CLI 以 **app 目录的父目录**为前端工程根跑钩子。初写
   `npm --prefix ../../ui run build` 实测落在 `D:\code\ai\ui` → `ENOENT`；改 `../ui` 后正常。
2. **GitHub 直连不可达，打包工具下载超时**：CLI 从 `github.com/...releases/...`
   下载 NSIS 3.11、`nsis_tauri_utils.dll`、WiX 3.14 二进制（本机 `curl` 直连
   12s 超时；报错为 `failed to bundle project: timeout: global`）。CLI 支持
   `TAURI_BUNDLER_TOOLS_GITHUB_MIRROR`（另有 `_TEMPLATE` 变体），设为
   `https://gh-proxy.com` 后实际请求形如 `https://gh-proxy.com/https://github.com/...`，
   下载与哈希校验均通过。
3. **MSI `LGHT0311`：中文产品名 vs en-US 代码页**：`light.exe` 报
   「A string … not available in the specified database code page '1252'」×多处
   （产品名「AI 可视化控制台」含 CJK）。CLI 吞掉了 light 的 stderr，手工复跑
   `light.exe -ext WixUIExtension -ext WixUtilExtension -cultures:en-US -loc locale.wxl -o … main.wixobj`
   取到原文后确认。`bundle.windows.wix.language: "zh-CN"`（代码页 936）修复；
   产物名随之变为 `…_x64_zh-CN.msi`。NSIS 侧无此题（Unicode 安装器）。

另有一次 `os error 32`（文件占用，发生在刚链完 exe 后的复制阶段，疑杀软实时扫描）
——原样重试即过，无代码/配置改动；记录在此备查。

## 五、验证边界与未做事项（如实）

- **已验**：两个安装器由最终提交的源码 + 配置产出（release 编译 + 打包全套跑通，
  产物体积/哈希已记录；nsis 先于 `wix.language` 一项构建，该项为 msi 专属配置、
  不影响 nsis 产物；其余配置与代码一致）。
- **未做：安装/卸载实跑**。安装会写注册表与 Program Files，超出本批范围，
  留待专门机器或人工执行。安装时 WebView2 走默认 `downloadBootstrapper`
  ——缺 WebView2 的机器安装阶段需联网引导（产品决策，如需离线可改 `embedBootstrapper`/`offlineInstaller`）。
- **未做：安装包内应用自检**（启动、网关、终端在安装形态下的走查）——依赖上一条。

## 六、P8 其余部分（不在本批）

| 项 | 状态 | 说明 |
| --- | --- | --- |
| dmg（macOS） | 未做 | Tauri 不做跨平台打包，需 macOS 构建机（02 §10.5 CI matrix 已定义 `tauri build` 步骤） |
| AppImage / deb（Linux） | 未做 | 同上，需 Linux 构建机 |
| 性能基准（02 §11 七项指标） | 未做 | 需要基准脚本与对照环境；冷启动/RSS/会话列表/详情分页/WSL stat/网关 P99/并发 200 |
| 剩余对拍 | 已另有覆盖 | 终态对照见 43 号；43 号的结论与缺口修复（`skill:delete`）已闭环 |

## 七、与 42 号的关系

- 42 §五.5 由本批**清零**（active→true + 双安装器实测）。§五 其余条目的清零情况见
  42 号文档 §五 的就地批注（2026-10-02）。
- 本批同时清掉的存量缺陷：粘贴图片同毫秒互相覆盖（commit `7ae43d8`）、初始提交
  误入的 0 字节文件（commit `e8b13e1`）——发现过程与证据见提交信息与 SDD 账本。

## 八、安装实测（2026-10-02）：MSI 在 Program Files 下普通用户跑不起来（已确诊，修复待决策）

§五曾记「安装/卸载实跑未做」。本次按普通用户对两个安装器做了完整装机实测：

| 安装形态 | 落点 | 普通用户启动 | 数据目录 |
| --- | --- | --- | --- |
| MSI（`ALLUSERS=1`，per-machine） | `C:\Program Files\AI 可视化控制台` | **失败退出**（stderr：`启动失败：创建目录失败：…\data`） | `data/`、`logs/` 均未创建 |
| NSIS（per-user） | `%LOCALAPPDATA%\AI 可视化控制台` | 正常启动运行 | `data\`（`aivc.db` / `aivc-settings.json` / `cache`）、`logs\`（`app.log` / `performance.log`）全就绪 |

- 复现细节：非提权静默安装返回 **1925**（「没有足够的权限为该计算机的所有用户完成安装」，
  提权后安装成功）；写权限单独用 `touch` 验证为 `Permission denied`；失败启动
  **无残留进程**（WebView2 子进程随主进程回收）。
- 根因：运行根 = exe 所在目录（`runtime_root.rs` 的既有口径，为对齐旧平台而保留）
  ——旧平台从未在 Program Files 下验证过，这是**首次实测**。
- **修复方案（待决策）**：A) Installed 形态探测 exe 目录可写性，不可写则回退
  `%LOCALAPPDATA%\aivc`（可同时解决 macOS `.app` 包内不可写的同源问题）；
  B) 只发 per-user（NSIS），放弃 MSI；C) 保持现状 + 文档警示。

### 八·补记：修复与复验（2026-10-02 当日）

- **修复**（aivc-rs `7bdfa25`）：打包形态运行根不可写时回退 `%LOCALAPPDATA%\aivc`
  （macOS→`~/Library/Application Support/aivc`、Linux→`$XDG_DATA_HOME|~/.local/share/aivc`）；
  开发根与 `AIVC_RUNTIME_ROOT` 覆盖不参与回退。7 条单测 + workspace 全量测试 + verify 全绿。
- **复验（等价条件的真机验证，免提权）**：把发布构建 `aivc-app.exe` 放进一个用
  ACL 拒写（Deny CreateFiles/WriteData）的目录——与 Program Files 的失败条件等价——
  普通用户启动：**进程正常运行**（修复前同条件即退）、stderr 干净、数据落在
  `%LOCALAPPDATA%\aivc\`（`data/` 含 aivc.db、`logs/` 含 app.log）、不可写目录零写入。
- **真机复验（2026-10-02 补完，真实 Program Files 场景）**：一条提权链「卸载残留实例 →
  清 `HKCU\Software\aivc` → 重装」后 `InstallLocation = C:\Program Files\AI 可视化控制台\`
  （注册键假设坐实）；按普通用户启动：**正常运行**、stderr 干净、数据全部落在
  `%LOCALAPPDATA%\aivc\`（aivc.db / settings / cache / logs）、Program Files **零写入**。
  修复前同条件即报「创建目录失败」退出——**回退修复在真实场景下闭环**。
- **过程发现（交付运维须知）**：Tauri 的 MSI 模板会 `RegistrySearch`
  `HKCU\Software\<manufacturer>\<product_name>`（注释写明「这是 nsis 安装器的记录方式」）
  ——**NSIS 卸载不会清这个键**。因此「先装 NSIS、卸载、再装 MSI」会把 MSI 装到 NSIS
  留下的旧目录（本次复测就撞上了：MSI 被重定向到 `%LOCALAPPDATA%`）。排查同类问题时
  先看这个键；默认路径复现前先清它。
- **收尾状态**：测试安装**已全部卸载**（Program Files 与 `%LOCALAPPDATA%` 均无残留、
  注册表卸载项 0 条、`HKCU\Software\aivc` 亦清）；安装包产物保留在
  `target/release/bundle/`（`nsis/` + `msi/`）。
