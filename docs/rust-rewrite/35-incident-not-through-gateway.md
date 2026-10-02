# 事故与排查：终端「没走网关」，请求直连了第三方

> 用户报告：在终端里发「你好」全部报错，`url: https://gateai.cc/v1/responses`，
> `403 INSUFFICIENT_BALANCE`；以及物理机 `codex` 报
> 「start the Windows daemon from a non-elevated terminal」。

---

## 一、问题 1：请求为什么直连了第三方

### 决定性证据（应用自己的日志）

`target/debug/logs/app.log`：

```text
02:48:35  INFO  aivc_app: 本地 Gateway 已启动 address=http://127.0.0.1:50064
02:48:40  DEBUG aivc_ipc::router: 收到 IPC 命令 command="terminal:start"
02:48:40  INFO  aivc_ipc::route: 该平台没有可用的供应商，本次终端不注入路由 provider="codex"
02:49:47  DEBUG aivc_ipc::router: 收到 IPC 命令 command="terminal:start"
02:49:47  INFO  aivc_ipc::route: 该平台没有可用的供应商，本次终端不注入路由 provider="codex"
```

**网关在跑**（50064 端口），但两次开终端都**没有建立路由**——因为**没有可用供应商**。

### 于是链路是这样走的

| 条件 | 注入的配置 | 请求去向 |
| --- | --- | --- |
| **有**可用供应商 | 6 项 `-c` 覆盖，含 `model_providers.akim_gateway.base_url=http://127.0.0.1:<port>/gateway/…` | 本地网关 → 用**供应商的密钥**转发 |
| **没有**可用供应商（本次） | **一条都不注入** | Codex 用它自己 `~/.codex/config.toml` 里的 `akim_gateway` → `base_url = https://gateai.cc/v1` |

用户的 `~/.codex/config.toml` 里恰好也把这个 provider 命名为 `akim_gateway`：

```toml
model_provider = "akim_gateway"
[model_providers.akim_gateway]
base_url = "https://gateai.cc/v1"
```

**所以名字对上了、地址却是配置文件里的**——请求根本没经过本应用的网关，
`INSUFFICIENT_BALANCE` 是那个中转站返回的，花的是用户自己的余额。

### 为什么没有可用供应商

`vendor:list` 在两个可能的运行根上都是**空**（各查了一次，见下）。四种成因在界面上
给的是同一句话，因此新增了一个排查探针 `scripts/probes/vendor-audit.js`：

```powershell
# 只读地看用户真实数据（不删任何东西）
.\scripts\app-probe.ps1 -Probe probes\vendor-audit.js -RunRoot D:\code\ai
```

它会打出每家供应商的 `providerId` / `enabled` / **密钥长度**（只报长度，不打密钥），
以及网关开关与端口。

### ⚠️ 我这里必须承认一处不确定

**我无法确定用户原本有没有配过供应商。** 原因是旧的 `app-probe.ps1` 会执行
`Remove-Item logs, data`，而开发构建把应用数据放在**工作目录**下——探针的工作目录
正是工程根 `D:\code\ai\aivc-rs`。也就是说：

- 如果用户曾以 `D:\code\ai\aivc-rs` 为工作目录运行过应用，**那份数据会被探针删掉**；
- 现在 `aivc-rs\data` 不存在，而 `target/debug/data` 存在且**供应商为空**。

两条事实都成立，但**哪一条导致了「没有供应商」，我无法证明**。这正是下面前两条修复
要解决的问题——不再让这种不确定性有出现的可能。

---

## 二、问题 2：提权守护进程报错 —— **由我的探针造成**

```text
codex.exe  app-server --listen unix:// --managed-daemon   ← 正在运行
codex-cli 0.157.1                                          ← 版本被升级过
```

**经过**：跑「恢复会话」验收探针时，Codex 弹了升级提示
（`Update available · 0.156.1 → 0.157.1 … enter continue · esc skip`），
探针**先按了回车** → 它真的执行了 `npm install -g @openai/codex`，
把用户的 Codex 从 **0.156.1 升到 0.157.1**。

我在当轮日志里看到了 `🎉 Update ran successfully!`，但**没有意识到那已经改动了用户环境**，
只是把后续按键改成了 esc。用户随后在物理机终端里撞上的报错，根源就在这一下：

- Codex 0.157 起默认启用 Windows 后台守护进程（`app-server --managed-daemon`）；
- 用户 `config.toml` 里有 `[windows] sandbox = "elevated"`；
- 该模式下要求守护进程从**非提权终端**启动，一旦由提权进程拉起过，
  之后所有共享客户端都会报「shared clients must not inherit administrator privileges」。

**处置建议**（未擅自改动用户环境）：`codex --no-daemon`，或结束残留的两个
`codex.exe` 后用非提权终端启动；若要回到 0.156.1，`npm install -g @openai/codex@0.156.1`。

---

## 三、三条修复

| # | 修复 | 落点 |
| --- | --- | --- |
| 1 | **探针不再可能删用户数据** | `scripts/app-probe.ps1` |
| 2 | **「没走网关」必须显眼** | `ui/src/app/vendor-route-picker.tsx` + `crates/aivc-ipc/src/route.rs` |
| 3 | **探针绝不确认会改动环境的提示** | `scripts/probes/resume-e2e.js` |

### 1. 探针改成「隔离运行根 + 只读排查」

- **默认**：每次新建一个**临时目录**作为工作目录，因此每次都是干净首次运行，
  而工程目录根本不在这条路径上——不需要（也不允许）删任何东西；
- 新增 `-RunRoot`：**只读地**用指定目录，用于排查用户实际现象（本文件第一节就是用它查的），
  这条路径**不删任何东西**；
- **守卫**：临时运行根若落在工程目录内，脚本**立刻抛错**。这样将来有人改回
  「用工程目录 + 清 data」时，是**报错**而不是又一次悄悄删掉用户的数据。

### 2. 「没走网关」从一句中性状态变成警示

改之前状态栏写的是「供应商 **直连**」——用户看到它以为一切正常。现在：

```text
⚠ 未走网关 · 直连（用自己的密钥）
```

并且：

- 断言升级为「不只是有字」：`warn` 类 + 明说「未走网关」+ 点出「用自己的密钥」；
- 后端那两条「不注入路由」的日志从 `INFO` 提到 **`WARN`**，并补上 `terminal` 字段，
  措辞改成「**未接入网关：CLI 将直连它自己配置的上游**」——原来的话（「不注入路由」）
  在实际排查时根本没被看见。

### 3. 探针绝不确认会改动环境的提示

- **升级提示一律选「跳过」**，注释里写明理由与代价：按回车会真的执行安装，
  而探针**没有资格改动用户安装的 CLI 版本**；
- 目录信任（只写本地配置、不改变安装）才允许按回车；
- 判据仍然是「读屏幕再决定按键」，不假设顺序。

---

## 四、给用户的结论

1. **你的终端确实没走网关**，原因是**没有可用供应商**——日志里有明确一行。
   在应用里确认那家 Codex 供应商存在、启用、且填了 API Key 之后，路由就会建立；
   判断方法：日志出现 `已为终端绑定网关路由 … base_url="http://127.0.0.1:…"`。
2. **那条守护进程报错是我造成的**（我的探针误按回车升级了你的 Codex）。
   我为此加了一条硬规则：探针不再确认任何会改动环境的提示。回滚命令见上文。
3. 「没走网关」在界面上现在是**警示**而不是中性的「直连」——这一条是最该早做的。
