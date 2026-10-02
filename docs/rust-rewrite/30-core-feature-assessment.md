# 30 · 评估：我最近做的事是不是在推动核心功能

> **状态更新（本轮完成）**：核心缺口已补齐。原来「供应商 + 本地网关」对终端**完全不生效**
> ——不是没接线，是**路由的生产者根本不存在**。现在生产者、分派、WSL 形态、路由生命周期
> 都在，并且有 5 层端到端证据（含 2 处假绿检查）。详见 §6.4–§6.8。

> 用户提出的问题（2026-09-26）：
>
> > 整个旧平台的核心就是接入 codex / claude / gemini / qoder 等厂商 CLI 做统一处理，
> > 然后供应商 + 本地网关。选择重构就是想**把这几个功能独立出来，为接入其他厂商 CLI
> > 做准备**。你现在做的是不是在推动这几个核心功能落地？

## 一句话回答

**没有。** 最近几轮（第 1–14 轮重设计目标）做的是 **CSS 层面的视觉打磨** ——
token 阶、侧栏底色、导航形态、消息气泡。它既不是核心功能，甚至**也不是「让核心功能
可被使用」的那部分工作**，而只是把外壳修得好看。

## 1. 核心功能实际在哪（用代码量说话）

| crate | 行数 | 属于核心吗 |
| --- | ---: | --- |
| `aivc-provider` | **15075** | ✅ **核心**：四个厂商的会话读取、分支、导出 |
| `aivc-infra` | 10559 | ✅ 数据库、配置、文件存储 |
| `aivc-core` | 7015 | ✅ 领域模型（`ProviderId`、会话、网关规则） |
| `aivc-ipc` | 5767 | ✅ 命令面（把核心暴露给界面） |
| `aivc-tooling` | 5371 | ✅ CLI 安装、供应商模型与余额 |
| `aivc-gateway` | 3999 | ✅ **核心**：本地网关 |
| `aivc-app` | 2996 | 组装根 |
| `aivc-terminal` | 2259 | ✅ 终端运行时（跑厂商 CLI） |

**核心不是没做——它在 Rust 侧已经很大了。** 我最近动的是 `ui/` 里的几个 CSS 文件。

## 2. 关键问题：重构的目的是「为接入新厂商做准备」，这个接缝好不好？

这是判断重构是否成功的地方。查了加第 5 个厂商要动多少处：

**`ProviderId::Qoder` 在 14+ 个文件里出现 52 次**（去掉测试仍有约 15 处生产代码）：

| 文件 | 每加一个厂商要改什么 |
| --- | --- |
| `aivc-core/src/target.rs` | `ProviderId` 枚举、`capabilities()` |
| `aivc-ipc/src/validate.rs:145` | **字符串 → ProviderId 解析** |
| `aivc-ipc/src/terminal.rs:547` | **同一件事再写一遍** |
| `aivc-ipc/src/cli.rs:115` | **同一件事再写第三遍** |
| `aivc-gateway/src/routes.rs:143` | **同一件事再写第四遍** |
| `aivc-tooling/src/vendor_models.rs:109` | 模型列表与鉴权方式 |
| `aivc-tooling/src/vendor_balance.rs:320` | 余额查询的鉴权方式 |
| `aivc-tooling/src/cli_env/core.rs:38,49` | 最低 Node 版本、版本号环境变量名 |
| `aivc-terminal/src/status.rs:403` | 提示符正则 |
| `aivc-terminal/src/command.rs:219` | 环境变量注入 |
| `aivc-gateway/src/upstream.rs:68` | 上游鉴权头 |
| `aivc-provider/src/<厂商>/` | **5 个新文件**（parser / provider / paths / branch / mod） |

### 两个具体缺陷

1. **「字符串 → ProviderId」写了四遍**（`validate.rs`、`terminal.rs`、`cli.rs`、
   `routes.rs`）。四处可以各自漂移——`terminal.rs` 与 `validate.rs` 的 `other =>` 兜底
   行为如果不一致，就会出现「校验说认识、启动说不认识」这类只在特定入口复现的 bug。
   这正是本工程反复出问题的**同一个形状**。

2. **`match ProviderId` 散在 5 个 crate 里**，且多数带 `|` 合并的臂
   （`ProviderId::Codex | ProviderId::Qoder`）。加第 5 个厂商时，**编译器不会告诉你
   漏了哪一处**——因为 `|` 臂不会因为新增枚举值而变红。

**所以：这个接缝目前是「能加，但靠人记」，不是「编译器会拦住」。** 而重构想解决的
正是这件事。

## 3. 界面这一侧：核心功能能不能被用到？

| 核心能力 | 界面入口 | 状态 |
| --- | --- | --- |
| 启动各厂商 CLI 终端 | `+ Codex` / `+ Shell` | ⚠️ **只有 Codex**。`terminal.rs` 里非 Codex 直接报「暂不支持启动 xxx 终端」 |
| 会话读取 / 分支 / 导出 | 会话列表 + 详情 | ✅ 可用 |
| 供应商管理 | 配置 → 供应商 | ⚠️ 表单式列表，**没有 L15 要求的表格**（列宽拖拽、分页、候选池开关、余额刷新、故障规则弹框） |
| 本地网关 | 配置 → 网关；状态栏显示端口 | ⚠️ 有端口/规则/外部 API，**没有工作台概览**（L4/L5 未做） |
| 网关全链路（请求经网关转发） | — | ❓ **未核对** |

**最要紧的一条**：`terminal.rs` 里**只有 Codex 能启动**，其余三个厂商点了会报错。
也就是说「统一平台处理各厂商 CLI」在**启动终端**这一环只做了四分之一。
而我最近 14 轮没有碰过它。

## 4. 结论

| 问题 | 回答 |
| --- | --- |
| 最近的工作在推动核心吗？ | **没有。** 是 CSS 打磨 |
| 核心功能本身做了吗？ | **大部分做了**（provider 15k 行、gateway 4k 行） |
| 重构的目标（为接新厂商做准备）达成了吗？ | **部分**：有 `SessionProvider` trait，但接缝**靠人记**，且字符串解析写了四遍 |
| 核心功能用户能用吗？ | **部分**：会话可用；**终端只能起 Codex**；供应商/网关是表单而非能力完整的界面 |

## 5. 应该做什么（按对核心的价值排序）

### 第 1 批 · 让「统一平台」真的统一（当前只做了 1/4）

1. **`terminal:start` 支持 claude / gemini / qoder**
   —— 现在非 Codex 直接报错。这是「统一平台」最直接的一环，而且后端
   `command.rs` 已有各厂商的构造逻辑（`provider_environment`、`supports_gateway`），
   缺的是把它接到 `start` 上。
2. **四个字符串解析点合成一处**
   —— 一处 `ProviderId::parse`，其余三处转调。这是「加厂商靠人记」的直接原因之一。

### 第 2 批 · 让厂商接入变成**编译器会拦住**的事

3. 把散落的 `match ProviderId` 改成**穷尽匹配**（去掉 `|` 合并臂，或引入
   `ProviderDescriptor` 表，一个厂商一行）。
4. 写一份「接入新厂商」的检查清单 + 一个**能被门禁检查**的测试
   （例如断言 `ProviderId::ALL` 的每一项在 descriptor 表里都有一行）。

### 第 3 批 · 把核心能力暴露到界面

5. **L15 供应商表格**（列宽拖拽、分页、候选池开关、余额刷新、故障规则弹框）
6. **L4/L5 工作台**（网关概览）

### 不再做的事

- **停止 UI 视觉打磨**（token、间距、导航形态、消息气泡）。它不影响任何一个核心功能。

## 6. 🔴 已定位：核心缺口的**确切位置**（下一步直接改这里）

查了 `aivc-terminal/src/command.rs` 与 `aivc-ipc/src/terminal.rs`，结论比「只支持 Codex」
更严重。

### 6.1 构造器**已经齐了**，缺的是分派

`command.rs` 里现成的构造器：

| 厂商 | 走网关 | 该用哪个构造器 |
| --- | :-: | --- |
| Codex | ✅ | `build_codex_command`（路由走命令行配置，见 `codex_route_config_args`） |
| Gemini | ✅ | `build_gateway_cli_command`（env `GOOGLE_GEMINI_BASE_URL` / `GEMINI_API_KEY`） |
| Claude | ✅ | `build_gateway_cli_command`（env `ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN`） |
| Qoder | ❌ | `build_qoder_command` |

依据：`command.rs:205 provider_environment`、`:228 supports_gateway`。

**而 `aivc-ipc/src/terminal.rs::start` 里是**：

```rust
if provider != ProviderId::Codex {
    return Err(failed(format!("暂不支持启动 {} 终端。", provider.as_str())));
}
let command = build_codex_command(&params, None, &extra, cfg!(windows));
```

### 6.2 更要紧的一条：**`route` 写死 `None`**

`build_codex_command(&params, None, …)` —— 第三个参数是 `ResolvedRoute`，**永远是 `None`**。

也就是说：**启动终端时，供应商路由与本地网关完全没接上。** CLI 起来之后走的是它自己的
默认上游（或它自己的登录态），**不是界面上选的供应商，也不经过本地网关**。

这解释了为什么「供应商 + 本地网关」在界面上像是有、实际不生效——**它们没有接到终端
启动这条路上**。而「启动厂商 CLI 并让它走统一网关」正是整个应用的核心。

### 6.3 🔴 根因已确认：**没有任何生产代码产生 `ResolvedRoute`**

全仓搜索 `ResolvedRoute {`：**只有一处构造点，而且在测试里**
（`aivc-terminal/src/command.rs:392` 的 `fn route()`）。

也就是说整条「供应商 → 路由 → CLI 环境变量」的链路：

| 环节 | 状态 |
| --- | --- |
| `ResolvedRoute` 类型 | ✅ 有（`command.rs:54`） |
| 消费它的函数 | ✅ 有（`provider_environment`、`codex_route_config_args`、`build_gateway_cli_command`） |
| **产生它的函数** | ❌ **不存在** |
| 把它传给终端启动 | ❌ `start` 里写死 `None` |

**零件都在，缺的是把它们拼起来的那个人。** 所以界面上「供应商 + 本地网关」看起来有、
实际不生效——它们从来没接到终端启动这条路上。

`ResolvedRoute` 只需要两样东西：

```rust
pub struct ResolvedRoute {
    pub base_url: String,     // 网关路由地址
    pub local_token: String,  // 本地路由令牌
}
```

两样**都已经有来源**：`GatewayPortStatus { enabled, configuredPort, activePort }` 与
`getExternalApiStatus() → hasToken`。缺的是一个把它们组合成 `ResolvedRoute` 的函数。

**并且有一条容易漏的约束**（`command.rs:52` 的注释已经写明）：

> WSL 目标下 `127.0.0.1` 指向 WSL 自身而非宿主，因此地址必须**由调用方先探测再传入**。

也就是说「探测宿主在 WSL 里可达的地址」这一步必须做，否则 WSL 目标下的终端会连不上
网关，而本机目标正常——这种「只在一种目标上坏」的问题最难查。

### 6.4 改动清单（**已完成**）

| 步 | 动作 | 位置 | 状态 |
| :-: | --- | --- | :--: |
| 1 | 路由生产者 `RouteSource` / `GatewayRouteSource` | `aivc-ipc/src/route.rs`（新） | ✅ |
| 2 | WSL 目标下把 `127.0.0.1` 换成**宿主可达地址** | 同上；复用已有的 `WslAdapter::detect_gateway_host` | ✅ |
| 3 | `start` 按 `provider` 分派构造器 | `aivc-ipc/src/terminal.rs` + `aivc-terminal::build_command` | ✅ |
| 4 | 把路由**替换那个 `None`** | 同上 | ✅ |
| 5 | Qoder 不注入路由 | `build_command` 内部过滤，调用方不必各自记得 | ✅ |
| 6 | **端到端验证** | 见 6.6 | ✅ |
| 7 | 门禁 + 基线 | `scripts/verify.ps1` 全绿 | ✅ |

### 6.5 这一轮实际做了什么（可核对）

**A. 路由生产者（原来完全不存在）**

`aivc-ipc/src/route.rs`：

- `RouteSource` trait：`create(provider, distro, vendorId, terminalId)` / `destroy(routeId)`；
- `GatewayRouteSource` = 网关状态 + 供应商仓储 + WSL 适配器三者拼起来；
- 路由编号**直接用终端编号**——`SwitchNotice` 用路由编号当「哪台终端」的标识，
  两者不同名时界面收到故障切换通知也不知道该提醒哪一台；
- 供应商挑选走 `list_route_eligible`（启用 + Key 非空 + 地址非空）。**这是对现网的收紧**：
  现网 `createVendorRoute` 只看 `enabled`，于是「启用了但没填 Key」的供应商会生成一条
  必定失败的路由——终端能起来、每次请求都报上游错误；
- 「没有可用供应商」「网关没启动」返回 `None`（终端直连官方 API），**不是错误**，
  但都 `tracing::info!` 留痕——否则「配了供应商却不生效」查不出线索。

**B. 四个平台 × 两种目标**

`aivc-terminal/src/command.rs` 新增唯一入口 `build_command(provider, params, route, extra, target)`，
两个维度都在一处穷举（没有通配分支）。补齐了**此前完全缺失的 WSL 构造器**：
`build_wsl_codex_command` / `build_wsl_gateway_cli_command` / `build_wsl_qoder_command`。
`terminal.rs` 里那句 `if provider != ProviderId::Codex { return Err("暂不支持…") }` 删除。

**C. 两个此前没说过的真实缺陷**

| 缺陷 | 症状 | 修法 |
| --- | --- | --- |
| `CODEX_API_KEY` / `CODEX_ACCESS_TOKEN` **没有被删除** | PTY 默认继承宿主环境，而 `env` 只能覆盖不能抹掉。Codex 优先读这两个变量，于是**网关被静默绕过、请求记在用户自己的额度上** | `TerminalCommand` 新增 `remove_env`，`session.rs` 走 `CommandBuilder::env_remove`（**先删后设**） |
| `portable-pty` 在 Windows 上**静默丢弃**不存在的 `cwd` | 它 `current_directory()` 里 `.filter(Path::is_dir())` 会把不是目录的 cwd 丢掉、改用 `USERPROFILE`。于是「在还没建的目录里开终端」得到的是一个开在**用户主目录**里的终端，没有任何提示 | `PtySession::spawn` 先 `create_dir_all`；建不出来就**明确报错** |

**D. 界面**

`terminal:start` 此前**不传 `targetId`**，后端因此无法判断目标是本机还是 WSL。现已补上；
`startTerminal` 的类型也补全了 `targetId` / `vendorId` / `cliArgs` / `useCodexCwdFlag`。

**E. 顺手修掉的一个偶发缺陷（与核心功能无关，但会掩盖核心功能的验证）**

改完之后我把整套测试跑了 10 遍，**10 次里有 1 次失败**，而且每次是不同的测试、
症状一样：`未解析到目标：codex:local`。查到根因：

`App::start` 里有一句 `tokio::spawn(Self::warm_targets(...))`。预热的「首次启动」分支会去
**真实探测**目标，而探测结果按平台**整片替换**缓存（`merge_cached_targets`）。测试注入的
合成目标恰好也叫 `codex:local`——后台预热一旦跑在注入之后，就把它换成真机上那个，
或者真机探测不到本机目标时**整个清掉**。

值得注意的是：`warm_targets` 的文档早就写着「预热跑在后台任务里时测试无法确定性地等它
结束，抽出来之后测试可以直接 `await`」——**只是 `spawn` 一直留在 `start` 里，那句话没有落实**。

修法是把装配与预热分开：`App::start` 不再 `spawn`，由生产入口 `main.rs` 显式
`tauri::async_runtime::spawn(App::warm_targets(context))`，测试不启动。
修完 **12/12 通过**，整套工作区测试连跑 3 遍全绿。

> 这一条不修的话，「核心功能端到端验证」的可信度是要打折的：一个 1/10 随机失败的套件，
> 红的时候看起来像新代码的问题，绿的时候又不知道是不是刚好躲过了。

**这一条还有个后续，值得单独记下来**：修完之后我**真的启动了应用**，
结果进程启动即崩——

```text
thread 'main' panicked at crates\aivc-app\src\lib.rs:308:9:
there is no reactor running, must be called from the context of a Tokio 1.x runtime
```

原因是我把 `tokio::spawn` 从 `App::start`（在 `block_on` 里，有运行时）搬到了一个
同步包装方法 `App::spawn_warmup()`（在 `block_on` 之外，没有运行时）。
**所有单元测试都是绿的**——因为它们跑在 `#[tokio::test]` 里，自带运行时。
后来改成 `async fn` 又被 clippy 挡下（没有 `await`）。

最终形态是把 spawn 交回调用方，并且**刻意不提供包装方法**：让调用方明确地提供运行时，
比包装一层更安全。

> 这是本轮唯一一次「门禁全绿但产品起不来」。抓到它的不是任何测试，是**真的把应用启动了一次**。

### 6.6 端到端证据（这是本轮的重点）

| 层 | 测试 | 它证明什么 |
| --- | --- | --- |
| 命令 | `aivc-terminal` 3 条 | 四个平台在本机与 WSL 下都能拼出命令；路由注入的变量名逐个钉死 |
| 路由 | `route::tests::the_route_the_terminal_gets_is_one_the_gateway_actually_accepts` | **真网关 + 真供应商仓储**：终端拿到的地址就是网关上的那条路由；它拿到的令牌能通过鉴权（错的令牌返回 401）；终端关掉后同一条地址返回 404 |
| 命令装配 | `terminal::tests::the_created_route_is_actually_written_into_the_command` | 路由**真的进了命令**——这正是原漏洞的形状：创建了却不交给命令构造器，症状与完全没有生产者一模一样 |
| 路由装配 | `router::tests::a_started_terminal_registers_a_real_route_on_the_gateway` | `with_gateway` 真的装上了真实路由源；用现成的 `vendor:route-switch` 当探针，不需要为测试新增生产 API |
| 装配根 | `end_to_end::starting_a_terminal_registers_a_route_the_gateway_knows_about` | 走**真实命令面**（`App::dispatch`）验证整条链路 |

**两处关键断言都做了假绿检查**（把修复回退，确认测试变红且只有它变红）：

- 把 `build_command(…, route, …)` 改回 `None` → `the_created_route_is_actually_written_into_the_command` 红；
- 去掉 `with_gateway` 里的 `install_routes` → 路由装配测试红，失败信息正是
  `Ipc(InvalidParameter { name: "routeId" })`。

### 6.7 仍然未核对

- ~~**网关自身的转发与故障转移**没有端到端证据~~ —— **这句话上一轮写错了**，见 §7。
  网关层其实一直有 `a_retryable_status_fails_over_to_the_next_vendor` 等端到端测试。
  真正缺的是「熔断之后还会不会再打坏供应商」「候选池来自真实数据库」这些，本轮已补。
- ~~**界面还不能为终端挑供应商**~~ —— 本轮完成，见 §8。
- **手动的 WSL 运行时验证**：WSL 构造器有单元测试，但**没在本机 WSL 里真跑过**。
  本机的 Ubuntu-22.04 进不去（`退出码 -1`），Ubuntu-24.04 可达但尚未手工验证终端形态。
- **界面入口只覆盖「选中目标」这一条路**：工作台里按平台直接开终端的路径还没接。
- **`candidate-pool` 这个切换原因尚未产生**：现网还有一个「候选池变了」的提示，
  本实现只发 `manual` / `mode` / `failure` 三种。

---

## 8 · 界面上的「这台终端用哪个供应商」（本轮）

### 8.1 先读清楚老平台的模型，而不是自己设计

老平台的做法（`use-tab-vendors.ts` + `vendor-route-picker.tsx` + `embedded-terminal.tsx`）：

1. **启动时界面不挑供应商**，由后端按候选池选一家，并把 `vendorId` / `vendorMode`
   **放在启动结果里返回**；
2. 标签页绑定这个返回值（`bindTabVendor`）；
3. 之后手动切换走 `setVendorRouteMode(terminalId, vendorId, mode)`；
4. `gateway:vendor-switched` 事件更新标签页，**自动故障转移走的是同一条路径**。

这个设计是对的，我直接沿用：界面并不知道哪家的 Key 有效、哪家被禁用，让它猜只会猜错，
然后显示一个根本不存在的绑定。

### 8.2 两个缺口

| 缺口 | 症状 |
| --- | --- |
| `terminal:start` **没有返回** `vendorId` / `vendorMode` | 界面无从知道后端选了谁，状态栏只能空着 |
| **自动故障转移从不通知界面** | 请求正由第二家服务，状态栏却写着第一家。**用户看不到任何异常，只有账单对不上** |

第二条是这次查代码才发现：`notify_switch` 只被手动切换与模式变化调用，
`handle_inner` 里那两条转移路径**只是改了本地变量**。现网是会发这条事件的。

### 8.3 改了什么

**后端**

| 改动 | 位置 |
| --- | --- |
| `terminal:start` 返回 `vendorId` / `vendorMode`（没有绑定时两者缺席 = 直连） | `aivc-ipc/src/terminal.rs` |
| `GatewayState::notify_failover`：自动转移也通知，`reason = "failure"` | `aivc-gateway/src/server.rs` |
| 两条转移路径都调用它（连接失败、以及上游返回可重试状态码） | 同上 |

**前端**

| 改动 | 位置 |
| --- | --- |
| `TerminalSession` 增加 `providerId` / `vendorId` / `vendorMode` / `lastSwitch` | `ui/src/app/use-terminals.ts` |
| 订阅 `gateway:vendor-switched`，按 `terminalId` 更新 | 同上 |
| 启动结果里的供应商被绑到这路终端上 | 同上 |
| `switchVendor` / `setRouteMode`（**失败时不改界面**） | 同上 |
| 状态栏的供应商选择器 + 模式开关 + 「已自动切换」提示 | `ui/src/app/vendor-route-picker.tsx`（新） |
| `CHANNEL_GATEWAY_VENDOR_SWITCHED`、`switchVendorRoute`、`setVendorRouteMode` | `ui/src/ipc/commands.ts` |

**「没有绑定」也被显式表达**：显示「供应商 直连」，而不是一个空的选择器——
用户需要能区分「没配供应商」与「配了但不生效」。

### 8.4 真实运行时验收（本轮最硬的一条）

不是截图看个大概，而是**驱动真实界面 + 读真实 DOM + 对真实日志**：

```text
CDP 存一个 codex 供应商        → id = 7e7bfa3c-…
CDP 点击「+ Codex」开终端
后端日志                       → 已为终端绑定网关路由 provider="codex"
                                 route="term-muilfprh-1" vendor="7e7bfa3c-…"
状态栏 .status-vendor 文本     → 供应商 验证供应商 动态
下拉框 options / selected      → ["验证供应商"] / "7e7bfa3c-…"
```

也就是：**数据库里的供应商 → 终端启动 → 网关路由 → 界面绑定 → 状态栏显示**，
整条链路在真实运行中走通。

另外单独验证了「没有可用供应商」那一支：状态栏显示 **「供应商 直连」**，后端日志正是
上一轮加的那句留痕 `该平台没有可用的供应商，本次终端不注入路由 provider="codex"`。

### 8.5 🔴 运行时又抓到一个数据丢失隐患

第一次做上面那个验收时，我手工写了一个设置文件来启用网关，启动后日志出现：

```text
ERROR aivc_infra::config::settings: 设置文件解析失败，已降级为默认配置
    path=…\data\aivc-settings.json error=expected value at line 1 column 1
```

查下去发现那个文件开头是 `EF BB BF`——**UTF-8 BOM**。`serde_json` 不接受开头的 BOM，
于是它走了「文件损坏」分支：**把原文件改名备份、并把设置换成默认值**。

这不是「文件读不了」这么轻：用户用记事本之类的编辑器（「UTF-8 with BOM」是不少编辑器的
默认选项）手改一下设置文件，回来会发现目标缓存、网关端口、窗口偏好**全没了**，
只有一行 ERROR 日志能解释。

修法是解析前去掉开头的 BOM。**false-green 检查过**：去掉这一行，新测试立刻红
（`left: 0, right: 18080`——设置确实被默认值覆盖了）。

修完再跑一次真实运行时验收：设置文件仍以 `EF BB BF` 开头，而
**解析错误 0 条、损坏备份 0 个**，选择器照常工作。

---

## 7 · 网关故障转移：端到端证据

### 7.1 先更正上一轮的一个错误判断

我上一轮写「网关自身的转发与失败转移**没有端到端证据**」。**这是错的**——
`aivc-gateway/src/server.rs` 里早就有用 wiremock + 真实 `serve` + 真实 HTTP 的一组测试：

| 已有测试 | 它证明的 |
| --- | --- |
| `a_request_is_forwarded_with_the_vendor_key` | 真的转发，且用的是供应商自己的 Key |
| `the_local_token_is_never_forwarded_to_the_upstream` | 本地令牌不会漏给上游 |
| `a_retryable_status_fails_over_to_the_next_vendor` | 429 之后转移到第二家 |
| `a_locked_route_never_fails_over` | 锁定模式不转移 |
| `a_successful_request_is_recorded` / `a_rejected_request_is_still_recorded` | 每条路径都留记录 |
| `the_recorded_path_never_contains_the_access_key` | 记录里不含密钥 |

我当时是**看了行数就下结论**（「3999 行但没证据」），没有去读它的测试模块。
这与之前那次「看文件表面就说菜单栏不存在」是同一类错误，值得记下来。

### 7.2 真正缺的是什么

不是「能不能转移」，而是下面四条——它们都跨**多个请求**或跨**层**，单请求的测试看不见：

| 缺口 | 为什么重要 |
| --- | --- |
| **熔断之后，后续请求还打不打坏掉的那家** | 「自动故障转移」的价值不是救回这一次，而是让接下来一段时间**直接跳过**已知坏掉的供应商。`failover.rs` 只单测了 `is_circuit_open`，没有人验证过它真的在转发路径上生效 |
| **同一供应商要重试到阈值才切换** | 429 常常是瞬时的。一失败就换会把请求白白挪到更差的候选上 |
| **全部候选都失败时，回放上游真实错误** | CLI 靠状态码与错误体决定重试还是提示用户。网关自己编个 502 会把这些信息抹掉 |
| **候选池来自真实数据库** | 网关层测试用的是内存 `Vec`；真实路径是 `RepositorySource → VendorRepo → SQLite`，中间还有资格过滤、`sort` 排序、健康状态 hydrate |

### 7.3 新补的五条端到端测试

都在 `aivc-gateway/src/server.rs`，另加一条在 `aivc-app/tests/end_to_end.rs`：

| 测试 | 断言 |
| --- | --- |
| `a_failing_vendor_is_retried_up_to_the_threshold_before_switching` | 阈值 2 时第一家收到 **2** 次、第二家 **1** 次 |
| `an_open_circuit_keeps_later_requests_away_from_the_broken_vendor` | 第一个请求把熔断打开后，**第二个请求不再打到第一家**（计数不再增长） |
| `when_every_candidate_fails_the_real_upstream_error_is_replayed` | 客户端拿到 **429 + 上游正文**，而不是 502 |
| `a_connection_failure_fails_over_to_a_healthy_vendor` | 走 `send()` 返回 `Err` 的分支（与「上游返回错误状态码」是两条不同的路径） |
| `failover_works_with_vendors_read_from_the_real_database` | 真实 `vendor:save` 进库、真实命令面起网关、上游真的被问过一次、**重试不会被记成两次请求** |

### 7.4 🔴 又抓到一个真实缺陷：故障转移在记录里是隐形的

写 `a_failover_is_visible_in_the_request_record` 时它**直接失败**：

```text
assertion `left == right` failed: 记录里的供应商应当是最终成功的那一家
  left: "v1"
 right: "v2"
```

根因：`handle_inner` 里有**两条**转移路径，而它们不一致——

| 路径 | `switch_vendor` + `record_retry` |
| --- | --- |
| 连接失败（`send()` 返回 `Err`） | ✅ 有 |
| **上游返回可重试状态码**（429 等） | ❌ **没有** |

后果：

1. 记录里的 `vendorId` 仍是**第一家**——也就是那个**从没服务成功过**的供应商；
2. `switched` 永远是 `false`；
3. 界面上的故障转移视图因此什么都不显示；
4. 用量报表把这次成功的请求算在了一家没服务过它的供应商头上——**用户的账对不上**。

修法是让第二条路径与第一条完全一致。这条测试是**先写、先红、再修**的，
所以它抓到的是真缺陷而不是我补的断言。

> 顺带说明为什么它此前没被发现：`a_successful_request_is_recorded` 只断言「有记录」，
> `a_retryable_status_fails_over_to_the_next_vendor` 只断言「客户端拿到了 200」。
> **两条测试都通过，而它们之间那条缝正好是这个缺陷。**

---

## 10 · WSL 运行时验证（本轮）——并抓到一个「主平台一次都没起来过」的缺陷

### 10.1 做法

为 WSL 目标（Ubuntu-24.04）真实开启终端，读**真实 DOM + 真实终端输出 + 真实日志**，
再用 `wsl.exe` 从发行版内部反查网关。

### 10.2 🔴 最重要的发现：Codex 从来没起来过

第一次跑 WSL 终端，xterm 里打出的不是 TUI，而是 **Codex 自己的报错**：

```text
Error: invalid type: string "true", expected a boolean
in `model_providers.akim_gateway.requires_openai_auth`
```

根因：`codex_route_config_args` 把六项配置**统一 JSON 引号化**了。但
`requires_openai_auth` 是**布尔**，写成 `"true"` 时 Codex 直接拒绝启动。

对照老平台 `terminal-sessions.ts:697`：

```js
`model_providers.${CODEX_ROUTE_PROVIDER}.requires_openai_auth=true`   // ← 刻意不带引号
```

其余五项都用 `JSON.stringify(...)` 引号化，**唯独这一项不是**——移植时我把它「统一」掉了。

**更值得记的是它为什么没被测出来**：当时那条 `route_config_injects_all_six_entries`
**断言的正是引号形状**：

```rust
args.contains(&"model_providers.akim_gateway.requires_openai_auth=\"true\"".to_owned())
```

也就是说，**测试把缺陷锁死了**。类型对、门禁全绿、测试全过，而主平台的 CLI 一次都没起来过。
抓到它的唯一方式是**真的把一个终端跑起来看它打什么**。

修完之后：

```text
╭────────────────────────────────────────────────╮
│ >_ OpenAI Codex (v0.156.0)                     │
│ model: GPT-5.6-Sol high /model to change        │
│ directory: /mnt/d/code/ai                       │
╰────────────────────────────────────────────────╯
```

`directory: /mnt/d/code/ai` 是 **Linux 挂载路径**——这是「它确实跑在发行版里」的独立证据。

新增 `requires_openai_auth_is_a_bare_boolean`，同时断言「有裸 `true`」与「**不能有引号**」。

### 10.3 宿主地址：先证明 `127.0.0.1` 是**对的**，再补上回退的可见性

日志显示 WSL 终端拿到的地址是 `http://127.0.0.1:<port>/gateway/codex/…`。
而 `command.rs` 的注释专门警告过：NAT 模式下 `127.0.0.1` 在发行版里指向它自己。
所以必须判断这是**正确探测结果**还是**静默回退**。

实测（宿主开监听 → 发行版内连它）：

```text
listener_up=True
WSL reach 127.0.0.1 → REACHABLE
```

即这台机器是 **mirrored 网络**，`127.0.0.1` 本来就可达——探测脚本的第一项检查成功，
返回它就是对的。

但这里有个真实盲点：**「探测失败」与「mirrored 下本来就对」返回的是同一个值**。
不区分的话，NAT 模式下探测失败会表现为「终端正常启动、每次请求都连不上网关」，
而日志里那句地址看起来完全正常——**只有一种目标会坏，且没有任何线索**。
因此 `detect_gateway_host` 的两条回退路径现在都会 `warn!`，并说明后果。
（另外把 `build_route_url` 的 `base_url` 与 `distro` 加进了绑定路由的日志——
没有它，「绑定了路由」和「绑定到了哪里」是两件事。）

### 10.4 端到端证据：从**发行版内部**反查网关

这是本轮最干净的一条。启动应用（网关在监听）→ 为 WSL 目标开终端 → 从发行版内 curl：

| 调用方 | 请求 | 结果 |
| --- | --- | --- |
| Windows | 真实路由，无令牌 | **401** |
| **WSL 发行版内** | 真实路由，无令牌 | **401** |
| **WSL 发行版内** | 不存在的路由 | **404** |

结论：**交给 WSL 终端的那个地址，发行版真的连得上，而且网关真的认这条路由**——
既不是「发行版里连不上」，也不是「网关不认识」。这正是 `command.rs:52` 警告的那个风险点。

### 10.5 已知未尽事项

- **Codex 真的发出请求这一步没有单独验证**。它取决于 Codex 自身行为（需要用户输入提示），
  而本机发行版里 `bash -lc` 解析到的 `codex` 是 `/mnt/d/tools/nvm/nodejs/codex`——
  **Windows 的 node 脚本**，在 Linux 下执行会报 `This: not found`（`bash -ic` 下解析到的
  是能用的那个，TUI 因此起得来）。这是该机器的 PATH 状况，不是本项目的缺陷，但它意味着
  「目标可用」的判定（看 `~/.codex/sessions`）与「CLI 真的能跑」是两件事。
- 订阅 `terminal:data` 从页面侧收集输出这一步没成功（Tauri 的 `__TAURI__` 是冻结对象，
  无法包装 `invoke`；直接 `event.listen` 也没收到事件）。这不影响上面的结论——
  10.4 的证据不依赖它。

### 11 · 下一轮的建议顺序

---

## 11 · 系统排查「看起来整齐的统一」（本轮）

方法：把老平台的 `terminal-sessions.ts` 与 `crates/aivc-terminal/src/command.rs`
**逐行对照**，重点找两类东西——「老平台刻意不一致而移植统一了」与
「老平台多做了而移植丢掉了」。

> 我原本把这项拆给两个子代理并行做，**两个都没跑完**。因此下面的结论全部是我自己
> 逐行读出来的，两侧都带行号可核对。

### 11.1 核对结果

**Codex 路由配置的六项**（老平台 693–699 vs `codex_route_config_args`）

用 `grep '=true|=false|=\${JSON'` 把老平台的全部模板行列出来，**六项里只有一行是裸值**：

| 键 | 老平台 | Rust | 结论 |
| --- | --- | --- | --- |
| `model_provider` | `JSON.stringify(...)` | 引号化 | 一致 |
| `.name` | `JSON.stringify(...)` | 引号化 | 一致 |
| `.wire_api` | `JSON.stringify("responses")` | 引号化 | 一致 |
| `.requires_openai_auth` | **裸 `true`** | ~~`"true"`~~ → 裸 `true` | **§10 已修** |
| `.env_key` | `JSON.stringify(...)` | 引号化 | 一致 |
| `.base_url` | `JSON.stringify(route.baseUrl)` | 引号化 | 一致 |

**环境变量删除**（老平台 `buildTerminalEnvironment` 207–216 vs `codex_env_removals`）

老平台在 `overrides.OPENAI_API_KEY` 为真时才 `delete CODEX_API_KEY / CODEX_ACCESS_TOKEN`；
Rust 是 `if route.is_none() { return 空集 }`——**条件等价**（有路由 ⇔ 有 OPENAI_API_KEY）。一致。

**`env K=V …` 前缀**（老平台 `buildGatewayInvocation` 714–722 vs `gateway_invocation`）

老平台：有路由时 `buildShellCommand("env", [...entries, command, ...args])`，无路由时裸命令。
Rust 逐字同构。一致。（注意：`KEY=VALUE` 是作为**参数**传给 `env` 的，因此由 shell 引号规则
统一处理——这一处「统一」是**对的**，因为两侧都统一。）

**Qoder**（老平台 630–662 vs `build_qoder_command`）

`--resume` → `--cwd`（仅 `useCwdFlag` 时）→ 用户参数；`qodercn`；不注入任何网关变量。一致。

### 11.2 🔴 找到并修掉：本机 Windows 终端的 `cwd` 没有做挂载路径换算

老平台 `toWindowsShellCwd`（289–293）：

```js
const converted = wslMountPathToWindowsPath(cwd);
if (path.win32.isAbsolute(converted)) return converted;
return os.homedir();          // 换不出绝对路径就退回主目录，不猜
```

Rust **有** `aivc_core::path::wsl_mount_path_to_windows_path`（`path.rs:228`），
但**终端命令构造从来没调用过它**。后果具体且可复现：

1. 界面把「当前会话的目录」直接当工作目录传下来（`openTerminal(selectedTarget, selectedSession?.cwd)`）；
2. 一个 **WSL 会话**的目录是 `/mnt/d/code/ai`；
3. 用户此时选中的若是**本机目标**，本机终端就拿到这个 POSIX 路径；
4. `PtySession::spawn` 启动前会 `create_dir_all`（这是我上一轮加的），于是在当前盘符下
   **凭空多出一个** `D:\mnt\d\code\ai`；
5. 终端开在那里，而用户要的是自己的项目目录。

修法：`local_cwd` / `windows_local_cwd`，对**本机 Windows 的三个构造器统一**处理，
换不出绝对路径时退回主目录。**false-green 检查过**：去掉换算后测试立刻红
（`left: "/mnt/d/code/ai"` / `right: "D:\\code\\ai"`）。

> **与现网的一处有意差别**：现网只对 Codex 与 Qoder 做了这一步，Gemini / Claude 没有。
> 那两处是**遗漏**而不是有意的差别——同样的输入在它们那里会照样创建那个多余的目录树。
> 按「老工程不合理的地方要用正确方式实现」，这里统一处理。

### 11.3 ⚠️ 找到但**没有**修：Windows 上 Codex 可执行文件的解析

老平台 `resolveWindowsCodexCommand`（486–510+）会解析出**绝对路径**：
先 `where codex.cmd`，再扫 PATH，再扫 `%APPDATA%\npm`、`%LOCALAPPDATA%\npm`、
`Program Files\nodejs`、以及 `NVM_HOME\*` 各版本目录。它的注释写明了理由：

> Electron launched from Explorer may inherit a stale PATH after Node/Codex was installed.
> Resolve the npm shim explicitly so a portable copy of the application works on another
> Windows machine as well.

**Rust 侧直接硬编码 `codex.cmd`，完全依赖子进程继承到的 PATH。**

| | 老平台 | Rust |
| --- | --- | --- |
| 可执行文件 | `where` + PATH + npm/nvm 目录解析出的绝对路径 | 字面量 `codex.cmd` |
| PATH 陈旧时（安装后未重启资源管理器） | 仍能启动 | 报「不是内部或外部命令」 |

这台机器正是那个场景：`codex.cmd` 在 `D:\tools\nvm\nodejs`（nvm 管理）。
**没有修**，因为它需要一套跨平台的候选搜索，我这一轮没做——在这里明确记为
**已知差异**，而不是让它悄悄留在代码里。

### 11.4 顺带确认的事

- 本机 CLI 安装情况：`codex.cmd` / `qodercn.cmd`（nvm 下，`.ps1`/`.cmd`/裸文件都在）、
  `claude.exe`（`~/.local/bin`，真 exe）、**`gemini` 未安装**。
- 因此 `cli_program` 里的字面量在当前 PATH 下都能命中；Gemini 这条线**在本机无法做运行时验证**。

### 11.5 Claude 这条线的运行时验证（首次）

用界面流程（注入一个 Claude 目标 → 选中 → 点开终端）真跑了一次 `claude.exe`：

- 路由绑定成功：`provider="claude" base_url="http://127.0.0.1:50261/gateway/claude/term-…"`；
- 状态栏显示 **`供应商 Claude供应商 动态`**；
- **Claude Code 真的起来了**，输出里有它自己的模型目录告警：
  `"deepseek-v4.1-flash" isn't described by this version's model catalog…`

也就是说：`ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN` 注入到位、CLI 真的在跑。

**但两个 CLI 都没有真的向网关发出 HTTP 请求**（`requestCount: 0`），
因为交互式 TUI 需要用户输入提示，而我的工具链驱动不了 TUI：

- 直接 `terminal:write` 推提示：拿不到 `terminalId`（Tauri 的 `__TAURI__` 是冻结对象，
  无法包装 `invoke`），换成订阅 `terminal:data` 也没收到事件；
- `cliArgs: "-p hi"` 走非交互模式：同样没有请求到达网关，而当时看不到终端输出，
  因此**无法判断原因**。

这一环**没有验证**，如实记下。它是「CLI 真的发请求」的最后一步，
需要的是能驱动终端的验证手段（或一个真的会开口的用户）。

### 12 · 🔴 本机 Windows + 网关的 Codex **从来没启动过**（已修）

#### 12.1 现象

把终端里的 Codex 输出读出来，是它自己的参数解析报错：

```text
error: unexpected argument '"model_provider=^"akim_gateway""' found
```

注意那个 **`^`** —— 它是 `cmd.exe` 的转义字符，本不该出现在 Codex 看到的参数里。
这也解释了此前一个反常的对比：**WSL 下的 Codex 正常启动，本机 Windows 下的连启动都做不到**。

#### 12.2 先做实验，不推理

判断依据是 `aivc-ipc/tests/cmd_quoting_probe.rs`：它造一个**与 `codex.cmd` 同构**的
批处理（把 `%*` 原样往下传），让子进程把实际收到的文本写进文件。
因为 `codex.cmd` 是 npm shim，会把 `%*` **原样转发**给 `node`，
所以「cmd 交给批处理的 `%*` 是什么」就是问题的全部。

> 这个实验本身踩了两个坑，都值得记：不替 xterm.js 应答 ConPTY 的光标查询（DSR）时
> **子进程连第一步都走不完**；`PowerShell` 会吞掉 `cargo test` 的 `--` 分隔符，
> 因此改用断言而不是 `--nocapture` 打印。

#### 12.3 机制（已确认）

三个解析器，引号规则互不兼容：

| 层 | 规则 |
| --- | --- |
| 我们（`cmd_quote`） | `"` `^` `&` `\|` `<` `>` `%` 前加 `^` |
| `portable_pty` 的 `append_quoted`（`cmdbuilder.rs:702`） | MSVC/CRT：含空格或 `"` 就整体加 `"`、内部 `"` 变 `\"` |
| `cmd.exe` | 不把 `\` 当转义 |

第四层才是决定性的：`codex.cmd` 把 `%*` 原样转给 node，由 **node 的 CRT** 解析。
所以参数最终由 **CRT 规则**决定——而那正是 `portable_pty` 已经替我们做的事。

老平台的 `cmd_quote` / `build_cmd_command` 与 Rust **逐字相同**（连 oracle 对拍都有），
差别在**传输**：老平台交给 node-pty，而 node-pty 不会二次引号。

#### 12.4 修法

`aivc_terminal::command` 新增 `windows_command`：**程序与每个参数原样分开**交给
`portable_pty`，由它按 CRT 规则引号化；不再自己拼一条 cmd 命令行。
`build_cmd_command` 保留（为了与现网可对拍），但加了显式警告：**终端命令构造不应再调用它**。

新增回归测试 `a_windows_terminal_leaves_quoting_to_the_pty`：
断言参数里带着**未转义的 `"`**，且**不再出现 cmd 层的 `^`**。

### 12.5 🎯 整条链的最终证据（本轮达成）

修完之后，让真实 Codex 以非交互方式跑一次。**Codex 自己的输出**：

```text
ERROR: unexpected status 503 Service Unavailable: {"error":"gateway vendor unavailable"},
url: http://127.0.0.1:57877/gateway/codex/t-e2e/responses
```

——**真实 CLI 正在向我们创建的那条路由发请求**，网关回的是**我们自己的**错误文案。
网关侧的记录：

```text
requestCount: 30, failureCount: 30, averageDurationMs: 203
```

即：**数据库里的供应商 → 终端启动 → 创建路由 → 注入 CLI → 真实 Codex 发 HTTP 请求
→ 网关鉴权通过 → 转发失败（上游是故意打不通的端口）→ 记录进用量日志**。
「供应商 + 本地网关」这条核心链路至此**全程有客观证据**。

同一份日志还顺带证明了**熔断在真实流量下生效**：最早的 3 条记录带 `vendorId`，
之后的 `vendorId` 为空——那是熔断打开后 `choose_vendor` 选不出供应商、直接 503。
这正是 §7 那组测试描述的行为，现在被真实 CLI 观测到了。

> 顺带澄清一个此前卡住我的问题：`codex exec` 最初也不发请求，原因是它拒绝在
> **未被信任的目录**里运行（`Not inside a trusted directory and --skip-git-repo-check
> was not specified.`），与我们的代码无关。补上该参数后立刻打通。

### 14 · 让子进程找得到 npm 装的 CLI（本轮，已修并验证）

#### 14.1 补的是 §11.3 记下的那个差异

老平台的 `resolveWindowsCodexCommand` 会把候选**解析成绝对路径**（`where` → PATH →
`%APPDATA%\npm` → `%LOCALAPPDATA%\npm` → `Program Files\nodejs` → `%NVM_HOME%\*`），
它是为「从资源管理器启动时继承了陈旧的 PATH」写的——之后再装的 Node / CLI 不在里面，
而 Windows 上 `cmd.exe` 只认 PATH。

**移植里丢了这一步**，直接硬编码 `codex.cmd` 靠 PATH。

#### 14.2 换了一种做法：追加 PATH，而不是解析绝对路径

新增 `aivc-terminal/src/locate.rs`：把上列目录**追加**到子进程的 PATH。
三条理由：

1. **不引入误判。** `cmd.exe` 的查找规则还包括 `PATHEXT` 与当前目录，手工枚举必然
   覆盖不全；一旦我们判「没找到」而它其实找得到，终端就白起了。老平台那边会**抛错**，
   所以它承担了这个风险。
2. **一次覆盖四个平台。** 老平台只解析了 Codex，Gemini / Claude / Qoder 没有——
   而那三个同样可能是 npm 装的 `.cmd` shim。
3. **顺序与老平台一致。** 这些目录排在 PATH **之后**，PATH 里已有的仍优先
   （老平台也是先查 PATH 候选）。

这是**有意偏离**老平台实现的一处，理由如上。

纯函数（`fallback_dirs` / `appended_path`）与 I/O（`process_env` / `extend_path`）分开：
环境里有没有装 nvm 是碰运气的事，不该让测试依赖它。

#### 14.3 验证

- 单测 8 条：顺序、空值跳过、PATH 优先、**大小写与末尾分隔符都算重复**、只加新增项；
- 运行时：日志里 `已把 npm / nvm 目录补进子进程 PATH added=7 total=49`——
  正好是 `%APPDATA%\npm`、`%LOCALAPPDATA%\npm`、两个 `Program Files\nodejs`、
  以及 nvm 的三个版本目录；
- 同一轮里 Codex 照常运行（`codex-cli-exec 0.156.1`），证明注入没有破坏启动。

### 16 · 把验证手段固化（本轮）

#### 16.1 为什么值得做

「启动应用 → 开终端 → 读它到底输出了什么 → 查网关收到了什么」这套动作，
最近三轮里我**每次都在临时拼**，而且它三次救场：定位 Windows 引号缺陷、拿到 CLI 的真实
输出、证明请求到达网关。临时拼的代价不只是慢——**每次都可能漏掉某个关键步骤**
（我确实漏过：忘了应答 DSR，于是「终端没有输出」，而进程其实好好地跑着）。

#### 16.2 交付物：一条命令

```powershell
.\scripts\app-probe.ps1 -Probe probes\gateway-e2e.js -Log '绑定网关路由|补进子进程 PATH|panic'
```

| 文件 | 作用 |
| --- | --- |
| `scripts/probe-helpers.js` | 页面侧助手：`invoke` / `watch`（**订阅输出并应答 DSR**）/ `rows` / 等待目标 / 选值 / 读状态栏 |
| `scripts/probes/gateway-e2e.js` | 旗舰场景：存供应商 → 非交互启动 CLI → 读终端输出 → 查网关用量 → **判定** |
| `scripts/app-probe.ps1` | 编排：干净启动、**等 CDP 真的应答**（不是猜一个 sleep）、跑探针、打印命中的日志行、**总是收尾** |

它把手工时最容易漏的四件事变成默认行为：干净状态启动、等端点就绪而不是固定 sleep、
无论成败都收尾、顺手打印日志里命中的行——很多结论要靠后端日志才下得了。

#### 16.3 一次真实的运行结果

```text
verdict: "PASS：CLI 的请求到达了网关"
usage:   requestCount: 29, failureCount: 29
terminalTail: "OpenAI Codex v0.156.1 … provider: akim_gateway … ERROR: Reconnecting… 5/5"
日志:    已为终端绑定网关路由 … base_url="http://127.0.0.1:49971/gateway/codex/probe-e2e"
         已把 npm / nvm 目录补进子进程 PATH added=7 total=48
```

Codex 自己的横幅里那句 **`provider: akim_gateway`** 是额外收获：它证明注入的
`-c model_provider=…` 确实生效了，而不只是「命令构造出来了」。

#### 16.4 🔴 同一类坑今天咬了我两次，因此加进了门禁

`scripts/*.ps1` **必须带 UTF-8 BOM**：Windows PowerShell 会按 ANSI 读没有 BOM 的文件，
中文注释变乱码，而且常常在字符串终止符上炸开、报一句看不懂的语法错误。

两次都是这样：一次是新写脚本（`write` 工具不写 BOM），一次是**用会去掉 BOM 的编辑器
改过之后**——第二次尤其冤，因为改的正是这个脚本本身。

现在 `verify.ps1` 会检查全部 `scripts/*.ps1`。
**false-green 检查过**：去掉一个脚本的 BOM，门禁立刻报 `缺少 BOM：app-probe.ps1` 并失败。

> 同一轮还修了一个同族问题：`Get-Content -Raw` 没带 `-Encoding utf8`，
> 于是含中文的探针源码被按 ANSI 读进页面，表现为「探针抛异常」而看不出原因。

### 18 · 补齐第三种切换原因：`candidate-pool`（本轮）

#### 18.1 先读清楚老平台的语义

`vendor-gateway.ts:346-350`：

```js
if (vendor.id !== route.vendorId) {
  // 当前供应商被关闭、删除或熔断时才会在请求前切换；这不是请求失败，不显示异常提示。
  route.vendorId = vendor.id;
  notifyVendorRouteChange(route, routeVendor && isCircuitOpen(routeVendor.id) ? "failure" : "candidate-pool");
}
```

三个要点，缺一个都不对：

1. 它发生在**请求发出之前**——原供应商被禁用 / 删除 / 熔断，候选池换了人；
2. **要写回路由**（`route.vendorId = vendor.id`）；
3. 原因分两种：原供应商**被熔断**是 `failure`，其余是 `candidate-pool`。
   界面只对前者显示异常提示（`use-tab-vendors.ts:74`：`if (reason === "candidate-pool") return;`）。

#### 18.2 移植时漏掉了什么

漏的是**第 2 点**，而且它是静默的：`handle_inner` 手里的路由是
`routes.get(&route_id).cloned()` 的**克隆**，改它不影响注册表。现网直接改内存里的
route 对象，所以「写回」在那边是免费的——移植成克隆之后，必须显式补一次写回。

后果：**功能正常、界面永远不对**。请求由新的供应商服务，状态栏却一直写着那个已经被
禁用或删除的供应商。这比报错难查得多——没有异常，只有「显示的和实际的不一样」。

#### 18.3 修法

选完供应商之后判断 `attempt_vendor.id != route.vendor_id`，是则查一次原供应商的熔断
状态、据此选 `"failure"` 或 `"candidate-pool"`，然后
`switch_route_vendor(route_id, vendor_id, Some(reason))` —— 它本来就同时做
「写回 + 通知」，只是以前只被手动切换调用。它的文档已更新为列明两条调用路径。

前端不需要改：`VendorSwitch` 的原因类型早就含 `candidate-pool`，
而「已自动切换」的徽标只在 `reason === "failure"` 时显示——与现网一致。

#### 18.4 验证

两条新测试：

| 测试 | 断言 |
| --- | --- |
| `a_pool_switch_is_persisted_on_the_route_and_notified` | 原供应商被**禁用** → 由第二家服务、通知原因 `candidate-pool`、**且路由已写回 v2** |
| `a_pool_switch_after_a_circuit_opens_is_reported_as_a_failure` | 原供应商**被熔断** → 原因里有 `failure` |

**false-green 检查过**：把写回那一段短路掉，第一条测试立刻红
（`left: 0, right: 1`——通知一次都没发）。

### 20 · 再一次「刻意不一致被统一掉」：转发的请求头（本轮）

#### 20.1 两边的实现

老平台 `vendor-gateway.ts:855-866`：

```js
for (const [name, value] of Object.entries(request.headers)) {
  if ([…剔除列表…].includes(name)) continue;
  for (const item of Array.isArray(value) ? value : [value]) headers.append(name, item);  // ← 追加
}
if (vendor.providerId === "claude") headers.set("x-api-key", vendor.apiKey);              // ← 替换
else if (vendor.providerId === "gemini") headers.set("x-goog-api-key", vendor.apiKey);
else headers.set("authorization", `Bearer ${vendor.apiKey}`);
```

**两种不同的操作**：透传的头用 `append`，凭据用 `set`。

而 Rust 的 `headers_from` 把两者**统一**成了 `HeaderMap::insert`：

```rust
for (name, value) in build_upstream_headers(pairs, vendor) {
    result.insert(name, value);   // ← 覆盖
}
```

#### 20.2 后果

`HeaderMap::insert` 是**替换**：客户端**重复的同名头被合并成一条、只剩最后一个**。
同名重复是合法且常见的（`anthropic-beta` 就可能分多条发），代理把它们合并不是等价变换。

凭据那一侧的结果**碰巧**仍然正确（客户端的凭据已被剔除，没有同名头可撞），
但它依赖的是「刚好没有同名头」而不是「显式替换」——**那是会在别处坏掉的那种对**。

#### 20.3 修法

`build_upstream_headers` 改为返回 `UpstreamHeaders { forwarded, credential }`：
**把「透传的头」与「凭据」在类型上分开**，让调用方不可能再把它们混为一谈。
`headers_from` 相应地：透传的用 `append`，凭据用 `insert`。

#### 20.4 验证

- 单测 `repeated_client_headers_are_kept_in_order`：成对列表里两条同名头都在；
- **端到端** `repeated_client_headers_reach_the_upstream_unchanged`：
  真的发两条 `anthropic-beta` 过去，断言 **wiremock 收到的上游请求里两条都在**。
  这条是必要的——单测只看得到成对列表，**看不到写进 `HeaderMap` 的那一步**。

**false-green 检查过**，而且它把缺陷原样复现了：把 `append` 改回 `insert`，上游只收到
`["second-flag"]`（`left: ["second-flag"]` / `right: ["first-flag","second-flag"]`）。

#### 20.5 顺带核对的一处：剔除列表

老平台剔除 7 项，Rust 剔除 9 项（多 `keep-alive`、`proxy-authorization`）。
多出来的是逐跳/代理头，丢掉更正确——这一处**不是**漏移植。

`join_upstream_url` 与老平台 `joinUpstreamUrl` 逐条对齐：协议白名单、拒绝查询串/片段、
`/v1` 去重。一致。

### 21 · 扫响应侧：`accept-encoding` 必须由网关接管（本轮）

#### 21.1 先核对老平台的响应侧（**不是缺陷**）

`vendor-gateway.ts:487-490` 只丢 3 个头（`content-length` / `transfer-encoding` /
`connection`），而 Rust 的 `replay` 用 `should_drop_header`（9 项）。多丢的那些
（`host` / `authorization` / `x-api-key` / `x-goog-api-key` / `keep-alive` /
`proxy-authorization`）在响应侧基本不会出现，无害。

顺带确认了一件**做对了**的事：`http::response::Builder::header` 是**追加**语义，
因此重复的响应头（例如多条 `set-cookie`）能原样回传——与请求侧那个 `insert` 缺陷正好相反。

#### 21.2 🔴 但换掉传输层之后，一个新的前提没了

**老平台的 undici 默认自动解压**（gzip / deflate / br），所以它把 `accept-encoding`
照常透传给上游，读到的正文**始终是明文**。

而 Rust 的客户端是：

```toml
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "stream", "json"] }
```

**没开任何解压**。于是「上游压缩」与「网关读得懂正文」变成互斥的两件事。而网关恰恰
要从响应体里读两样东西：

| 要读的东西 | 用途 | 压缩之后的后果 |
| --- | --- | --- |
| 错误文本 | 失败转移规则按正文匹配 | **规则静默失效** |
| 用量 | 报表与计费 | **静默失准** |

还有第三条更隐蔽的：`probe_response` 靠 `text.contains("\n\n")` 判断「SSE 第一个事件
已完整，可以停止探测」。压缩体上永远找不到这个边界，于是**一直缓冲到上限**——
流式响应的首 token 被拖慢。这一条是**性能**问题而不是正确性问题，因此最难发现。

#### 21.3 修法

网关**自己接管** `accept-encoding`：转发时剔除它，让上游返回未压缩的正文。

与逐跳头分开成一个常量（`GATEWAY_CONTROLLED_HEADERS`）而不是塞进 `DROPPED_HEADERS`：
两者理由不同，混在一起会让「为什么丢这个头」变得含糊。

代价是「上游 → 网关」这一段不再压缩。要恢复它需要让客户端能解压，而那需要
`brotli` / `zstd` / `async-compression` 等依赖——**当前离线环境里只有 `flate2`**，
因此没走那条路。

#### 21.4 验证

| 断言 | 内容 |
| --- | --- |
| `accept_encoding_is_taken_over_by_the_gateway` | 透传列表里不含它，且它**不在**逐跳头列表里（两者理由不同） |
| `the_upstream_never_sees_accept_encoding` | 真的带 `accept-encoding: gzip, deflate, br` 发过去，断言 **wiremock 收到的上游请求里没有它** |

**false-green 检查过**：去掉接管后两条同时红，且**只有这两条**。

#### 21.5 这一类问题已经出现两次

「**传输层换了、老平台的隐含前提没了**」——这是本项目里回报最高的一条线索：

| 老平台的前提 | 换掉之后 | 后果 |
| --- | --- | --- |
| node-pty 不做二次引号 | `portable_pty` 按 MSVC 规则引号化 | 本机 Windows 的 Codex **完全起不来** |
| undici 默认自动解压 | `reqwest` 不解压 | 失败转移规则与用量统计**静默失效** |

两次都不是「哪一行写错了」，而是**换了一个库、而它的默认行为和原来不一样**。

### 22 · 重定向策略：老平台跟随，移植时改成了不跟随（本轮）

#### 22.1 差异

老平台**从不设置 `redirect`**（全仓 grep 过），也就是 Node `fetch` 的默认：**跟随**。
而 Rust 在网关的 HTTP 客户端上写的是 `Policy::none()`。

两者的代价方向相反：

| 策略 | 好处 | 代价 |
| --- | --- | --- |
| 跟随（老平台） | 凭据流向正确：网关用**供应商自己的密钥**去请求新地址 | 重定向目标若是本机，会绕回网关自己打转 |
| 不跟随（移植） | 不会绕圈 | **重定向被原样交给 CLI，而 CLI 会自己跟过去——带着它手里的本地路由令牌发给第三方主机** |

不跟随那一侧的「本地路由令牌被送出去」是移植时没想到的：当时只看到「跟随会绕过自我
代理判定」，没看到不跟随把凭据决策推给了下游。

#### 22.2 修法：跟随，但不跟随回本机

`Policy::custom`：重定向目标是本机（`127.0.0.1` / `localhost` / `::1` / `0.0.0.0`）
就停，否则跟随，并设 10 跳上限（与 `reqwest` 默认一致）。

顺带一提：这个规则比两边都好——既保住了凭据流向，又堵住了绕圈。

**单测抓到我一个真错误**：IPv6 主机的 `host_str()` 带方括号（`[::1]`），
直接 `parse::<IpAddr>()` 会失败，于是 `::1` 不被认作本机。已修。

#### 22.3 ⚠️ 一处**已知的验证缺口**（如实记）

「跟随非本机重定向」这一半**没有端到端测试**：`wiremock` 只监听 loopback，
它给出的每一个重定向目标都落在「本机、不跟随」那一侧；要端到端测「跟随」需要
一个非 loopback 的测试服务，而 `MockServerBuilder::listener()` 要求自备监听器、
绑定本机网卡在 CI 里并不总是允许。

因此策略的前一半只由**纯函数单测**保证（`loopback_redirect_targets_are_recognized`），
后一半有端到端测试（`a_redirect_back_to_the_gateway_is_not_followed`）。
**这是缺口，不是「已经验过了」。**

#### 22.4 过程里踩到的一个更值得记的坑

我最初写的两条端到端测试**都通不过假绿检查**：把策略改回 `Policy::none()`，
两条**照样通过**。原因是探测用的 `reqwest::Client::new()` **自己会跟随重定向**——
它把网关交出来的 307 又跟了一遍，于是断言看到的是**客户端**的行为。

修法：探测必须用 `Policy::none()` 的客户端，才观察得到网关自己的状态码。
为此加了 `post_raw()` 助手，并把这一点写进了测试文档。

> 这条比缺陷本身更值得记：**假绿检查不是形式**。它当场证明了我那两条测试什么也没测。

### 23 · 超时语义：老平台的超时**不转移**（本轮，确诊并有意保留差异）

#### 23.1 两边分别是什么

| | 超时的作用域 | 超时之后 |
| --- | --- | --- |
| 老平台 | **整轮共享一个总预算**（`AbortSignal.timeout` 建在重试循环**之前**，每次尝试复用） | **终止**——`isRetryableFetchError` 里 `if (… || timeoutSignal.aborted) return false;` |
| Rust | **每次尝试各自一个**（`reqwest` 的客户端级超时） | **可重试**，于是会故障转移 |

超时值是 `upstream_timeout_ms = 600_000`（10 分钟）。所以：

- 老平台：一台卡住的上游把整次请求耗光 10 分钟，然后放弃，**不做任何转移**；
- Rust：放弃卡住的那家、转给下一家；最坏总耗时是「尝试数 × 10 分钟」（默认最多 4 次）。

#### 23.2 判断：老平台这一处**不合理**，保留 Rust 的行为

「卡住就不转移」是**功能缺失**，而不是有意的取舍——故障转移正是网关存在的理由，
而最需要转移的场景恰恰是「上游不该答却一直不答」。老平台那两行代码让这个场景
完全落空，还让用户白等 10 分钟。

Rust 的代价（最坏 N×）是**可算的**，而且调用方（CLI）有自己的超时会先放弃。

因此这一处**不向老平台对齐**，并把理由写进测试文档。

#### 23.3 验证

`a_hanging_vendor_is_abandoned_and_the_next_one_serves`：
第一家延迟 10 秒、超时压到 300ms，断言客户端 **200 秒级拿到第二家的响应**、
且卡住的那家**只被尝试一次**。

**false-green 检查过，而且它把老平台的行为演了出来**：把超时改成不可重试
（即老平台那行 `timeoutSignal.aborted → false`），测试立刻红——
客户端拿到的是 **502**，也就是「卡住的上游把请求吃掉、没有任何转移」。

#### 23.4 这一类线索的第三例

| 老平台的前提 | 换掉之后 | 后果 | 处理 |
| --- | --- | --- | --- |
| node-pty 不做二次引号 | `portable_pty` 按 MSVC 规则引号化 | 本机 Windows 的 Codex 起不来 | 已修 |
| undici 默认自动解压 | `reqwest` 不解压 | 失败转移与用量统计静默失效 | 已修 |
| 超时是**整轮共享且终止性**的 | `reqwest` 是**每尝试**且可重试 | 行为变了（这里是**变好**） | 有意保留 |

前两例是「移植时丢了前提」，这一例是「移植时恰好改对了」——但**只有把它写下来、
并用测试钉住，它才算是被验证过的差异而不是巧合**。

### 24 · 核对 `aivc-tooling` 的 HTTP 路径：**这一轮没有找到缺陷**

这条线索前三轮连续产出，所以这一轮我把它走完了。结论是**对齐的**，如实记下来——
「查过、没问题」和「没查」是两件事。

#### 24.1 结构

`aivc-tooling` 用的是注入的 `HttpFetcher` trait（模型查询 / 余额查询都走它），
真正的传输在 `aivc-ipc/src/http_fetcher.rs`。因此逻辑层能脱离 HTTP 测试，**设计是对的**。

#### 24.2 超时：对齐，而且更细

| | 老平台 | Rust |
| --- | --- | --- |
| 上层查询超时 | `MODEL_LIST_TIMEOUT_MS = 15_000`（`vendor-manager.ts:680`）、`BALANCE_TIMEOUT_MS = 15_000`（`vendor-balance.ts:14`） | `MODEL_LIST_TIMEOUT_MS` / `BALANCE_TIMEOUT_MS` = `15_000`，**逐字一致** |
| 单次请求超时 | `setTimeout(() => controller.abort(), 15_000)` 包住每次请求 | `REQUEST_TIMEOUT = 12s` |

Rust 的 12 秒**故意**比上层的 15 秒短，执行器的文档写明了理由：让超时先在这里触发，
从而得到一个**明确的状态码**，而不是让上层在自己的计时器上看到「没有响应」。
**核对了那句话是真的**：`aivc-core/src/constants.rs:108` 与两个查询模块里都有 15_000。
这不是缺陷，是一处**有理由的细化**。

#### 24.3 重定向：与网关**相反**的策略，两边都有理由

| | 策略 | 理由（各自代码里写着） |
| --- | --- | --- |
| 网关 | 跟随（本机除外） | 不跟随会把重定向推给 CLI，而 CLI 会带着**本地令牌**跟过去 |
| 查询执行器 | **不跟随** | 中转站的重定向常指向登录页，跟过去拿到 HTML，报出的错与真实原因无关 |

老平台这两个查询都是 `fetch` 默认：**跟随**。所以这里确实与老平台不同。
两面的后果我都写下来，不替用户决定：

- 跟随（老平台）：`http → https`、路径搬迁这类**正当重定向**能正常工作；代价是重定向
  目标若是第三方，**供应商密钥会被送过去**（查询请求带着 `x-api-key` / `authorization`）。
- 不跟随（Rust）：密钥不会外流；代价是**正当重定向会让查询失败**。

#### 24.4 我考虑过「只跟随同主机」，最后**没有改**

想法是：只跟随「同一主机」的重定向——既能覆盖 `http → https`，又不会把密钥送去别的域。

**但它并不能解决执行器文档里写的那个问题**：登录页通常就在**同一个主机**上
（`/login`），所以「同主机也跟」会把那段 HTML 又跟回来，错误信息照样难以理解。

两边各能解决对方解决不了的一个场景，而**哪个更重要取决于用户实际遇到哪种中转站**——
我没有依据判断，所以**保持现状并把两种后果写清楚**，而不是按自己的猜测改。

> 这一条与前三轮不同：那三处是「移植时丢了前提」，这一处是「移植时做了另一种取舍」。
> 记下来是为了让下次遇到「模型查询莫名失败」的人能在文档里找到这个解释。

### 25 · 🔴 新线索第一击：供应商**健康与熔断**整条持久化链路没接上

我按建议去查「内存与数据库之间的同步」，第一处就撞上了。**这不是一个缺陷，是一簇。**

#### 25.1 老平台做了什么

`gateway-resilience.ts` 提供三个函数，而 `vendor-gateway.ts` 把它们放在**请求路径**上：

| 位置 | 调用 |
| --- | --- |
| 第 333 行 | `Promise.all([getGatewayVendorSnapshot(), hydrateGatewayVendorHealth()])`——**每个请求都从数据库把健康状态读回内存** |
| 第 426 / 453 / 467 行 | `recordGatewayVendorFailure(...)` |
| 第 469 行 | `recordGatewayVendorSuccess(...)` |

也就是说：**健康状态存在数据库里，每个请求前读回、每次结果写回**。熔断因此能跨重启记住，
界面上看到的也是真数据。

#### 25.2 Rust 这边

`HealthTracker` 是**纯内存**的：

| 症状 | 证据 |
| --- | --- |
| **从不 hydrate** | `HealthTracker::hydrate` 的调用点只有 `failover.rs:649`（一条单测）。生产路径一次都没调 |
| **从不写库** | `GatewayHealthRepo` 有 `upsert` / `upsert_many`，但全仓**没有任何生产调用**；只有 `load_all` 被 `gateway:get-vendor-health` 调用 |
| **「重置健康」按了没反应** | `gateway:reset-vendor-health` 只 `delete` / `delete_all` 数据库行（本来就是空的），**完全不碰活的 `HealthTracker`** |

#### 25.3 三个用户可见的后果

1. **熔断状态重启即丢**：关掉应用再打开，一台已经被熔断的供应商会被从头重试一遍。
   老平台会记得。
2. **界面上的供应商健康永远是空的**：`gateway:get-vendor-health` 读的表没有写入方，
   所以工作台那一栏**永远显示不出任何东西**——不是「都健康」，是「什么都没有」。
3. **「重置健康」是个空操作**：它清了本来就没数据的表，而真正的熔断状态在内存里纹丝不动。
   用户按了之后行为完全不变。

#### 25.4 一条佐证：这是**遗漏**，不是有意的设计

`aivc-core/src/constants.rs:102` 有：

```rust
pub const GATEWAY_HEALTH_SUCCESS_PERSIST_INTERVAL_MS: i64 = 5_000;
```

——「成功状态最多每 5 秒落库一次」这个**节流常量已经写好了**，说明设计时就打算落库，
只是接线没做。它现在是个未被使用的常量。

#### 25.5 为什么这一轮**只诊断、没有动手**

修它要同时做四件事，而且要一起成立才有意义：

1. 启动时 `hydrate`（从库读回，含「已过期的熔断要归一化」）；
2. 失败时写库（`upsert`）；
3. 成功时**按 5 秒节流**写库（常量已经在了）；
4. `reset-vendor-health` 同时重置内存里的 `HealthTracker`（否则第 3 条做完反而更糟：
   界面清了、熔断还在）。

四件里少任何一件，症状都比现在更混淆（例如只做 2 不做 4，就会出现「界面显示已重置、
实际仍被熔断」）。这是一个**需要完整做、并且要能验证**的改动，
在剩下这点预算里仓促拆一半，只会把「清楚的缺失」变成「半截的假象」。

因此：**精确记下诊断与四步方案，留作下一轮的第一件事。**

### 27 · 健康/熔断持久化：只做完了能独立成立的那一步

§25.5 的四步里，我**只做了第 4 步**（`reset-vendor-health` 真正重置内存状态）。

#### 27.1 为什么停在一步

其余三步要动 `GatewaySource` trait、`RepositorySource` 结构体与构造函数
（**5 个调用点**）、转发路径、组装根，共约 10–12 处编辑。剩余预算无法**一次做完整并验证**。

按 §25.5 自己写的理由——「四件里少任何一件，症状都比现在更混淆」——
**半截的改动比清楚的缺失更糟**，所以我没有启动它。

第 4 步不同：它**独立成立**，而且**只在消除混淆**、不制造新的。所以单独交付。

#### 27.2 改了什么

| | 改动 |
| --- | --- |
| `GatewayState` | 新增 `reset_health(Option<&VendorId>)`：重置内存里的 `HealthTracker` |
| 路由器 | `gateway:reset-vendor-health` 在清完数据库之后，**同时**重置内存那份；抽成 `reset_vendor_health` 方法（`dispatch` 因此突破 clippy 的 150 行上限，抽出后反而更清楚） |

#### 27.3 验证

`resetting_health_also_clears_the_live_circuit_state`：**走真实命令面**——
建一台指向死端口的供应商、把熔断阈值压到 1、发一次请求把熔断打开，
然后 `dispatch("gateway:reset-vendor-health", …)`，最后断言**内存快照里不再有开着的熔断**。

断言分两半：先确认熔断**确实是开着的**（否则测试可能什么都没测到），再确认重置之后关掉。

**false-green 检查过**：把路由器里那段重置短路掉，测试立刻红
（`重置之后内存里的熔断必须被清掉，否则按钮看起来没反应`）。

#### 27.4 仍然缺的三步（下一轮的第一件事）

1. 启动时 `hydrate`（含「已过期的熔断要归一化」）；
2. 失败时 `upsert` 落库；
3. 成功时**按 `GATEWAY_HEALTH_SUCCESS_PERSIST_INTERVAL_MS`（5 秒）节流**落库。

三个症状里，「熔断重启即丢」与「健康视图恒为空」**仍然存在**；
「重置按了没反应」这一条**已修**。

### 29 · 健康/熔断持久化：**补完了**（本轮）

上一轮只交付了第 4 步，这一轮把剩下三步做完。

#### 29.1 关键设计决定：用钩子，不改 `GatewaySource`

原方案是给 `GatewaySource` 加两个方法、再让 `RepositorySource` 实现它们——那要动
`RepositorySource` 的结构体与构造函数，**5 个调用点**、以及全部测试替身。

改成**钩子**（与已有的 `SwitchHook` 同一个模式）：

```rust
pub struct HealthHook {
    pub load: LoadHealth,   // 读回全部
    pub save: SaveHealth,   // 写回一条
}
```

网关仍然不认识数据库，由组装根注入；**没装钩子时就是「不持久化」**，
因此五处测试替身与既有测试**一行都不用改**。

#### 29.2 三处实现

| 步骤 | 做法 |
| --- | --- |
| 启动读回 | `GatewayState::hydrate_health()`，由 `start_gateway_if_enabled` 在 `serve` **之前**调用 |
| 失败落库 | `record_vendor_failure()`：更新内存后立刻 `upsert`。失败少，且「刚熔断」最需要跨重启记住，**不节流** |
| 成功落库 | `record_vendor_success()`：按 `GATEWAY_HEALTH_SUCCESS_PERSIST_INTERVAL_MS`（5 秒）节流。成功是高频事件，每次落库会把健康表变成写放大 |

转发路径上原本 4 处直接操作 `state.health` 的地方全部改走这两个方法——
**这一步不做，前面的就都是惰性的**。

#### 29.3 与现网的一处**有意差别**：不在每个请求前 hydrate

现网**每个请求**都 `hydrateGatewayVendorHealth()`。这里只在启动时读一次，理由是
`HealthState.failures` 是**有窗口**的：每个请求都 hydrate 会把窗口清成
「开着的熔断用一个占位时间戳」，于是「窗口内 N 次失败才熔断」的判定基本失效。

读一次、之后靠写回同步，既让熔断活过重启，又保住窗口语义。

#### 29.4 验证

`an_open_circuit_survives_a_restart` —— 用**真实数据库**走完整条路：

1. 建一台指向死端口的供应商、熔断阈值压到 1；
2. 起网关、打一次请求 → 断言内存快照里熔断**确实是开着的**；
3. 关掉；
4. **换一套全新的 `GatewayState` + `RepositorySource`（模拟重启）**、同一个数据库、
   走同一条装配路径 → `hydrate_health()` → 断言熔断**仍然是开着的**。

**false-green 检查过**：把第 4 步的 `hydrate_health()` 去掉，测试立刻红
（`熔断必须活过重启，否则用户会对着同一台坏供应商再白等一轮`）。

#### 29.5 §25.3 的三个症状现在都消了

| 症状 | 状态 |
| --- | --- |
| 熔断状态重启即丢 | ✅ 已修（本轮） |
| 界面上的供应商健康恒为空 | ✅ 已修（本轮：现在有写入方了） |
| 「重置健康」是空操作 | ✅ 已修（上一轮） |

### 31 · 把「真实 CLI → 网关」的探针扩到多平台（本轮）

探针本身早已支持 `-ProbeArgs`，所以扩起来主要是**按平台给对非交互参数**——
交互式 TUI 要用户敲一句提示才会发请求，每个平台都得找到它的非交互形式。
参数表直接写进了 `scripts/probes/gateway-e2e.js`：

| 平台 | 非交互参数 | 备注 |
| --- | --- | --- |
| codex | `exec --skip-git-repo-check hi` | 还要跳过「未被信任的目录」检查 |
| claude | `-p hi` | `--print` |
| gemini | `-p hi` | **本机未安装，无法验证** |
| qoder | —— | 不支持网关，永远不会创建路由，这个探针对它没有意义 |

#### 31.1 codex：**PASS**

```text
verdict:       "PASS：CLI 的请求到达了网关"
usage:         requestCount: 29
terminalTail:  "OpenAI Codex v0.156.1 … provider: akim_gateway … ERROR: Reconnecting… 5/5"
```

`provider: akim_gateway` 那句是 Codex 自己打出来的，证明注入的路由配置生效；
29 条请求到达网关（上游是故意打不通的端口，所以全部失败——这正是预期）。

#### 31.2 claude：**FAIL，但原因不在我们这边**

终端里拿到了完整诊断：

```text
[claude-code:unrecognized_model] {"model":"deepseek-v4.1-flash","query_source":"sdk"}
"deepseek-v4.1-flash" isn't described by this version's model catalog; update Claude Code,
or map it with behavesAs on a modelPicker row … CLAUDE_CODE_DISABLE_UNKNOWN_MODEL_WINDOW_ENFORCEMENT=1
restores the previous wait-for-the-API behavior.
```

**Claude Code 在本地模型目录里不认识这个模型名**（来自本机环境的 `ANTHROPIC_MODEL`
之类），于是在发出**任何** API 请求之前就退出了。

已经确认到位的部分：路由绑定成功、`ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN` 注入到位、
Claude Code 真的起来了（上面那句是它自己打的）。**卡在它自己的模型校验上。**

我又试了一次显式指定模型（`-p hi --model claude-sonnet-4-5`）——输出更少、仍然没有请求。
**没有继续试**：这是**本机 Claude Code 配置**的事，解决办法在客户端
（换一个它认识的模型名，或 `CLAUDE_CODE_DISABLE_UNKNOWN_MODEL_WINDOW_ENFORCEMENT=1`），
不该由网关去绕。

#### 31.3 因此这一步的结论是

**两个能装的平台里，codex 端到端通过；claude 那一条查下去发现了一个更值得修的根因**（见 31.4）。
gemini 未安装、qoder 不支持网关。

这与「四平台一致」的目标还有距离——但差距的性质在 31.4 之后变了：
**不再只是环境问题，而是我们自己注入方式的限制。**

#### 31.4 🔴 根因找到了：Claude 自己的设置**压过了进程环境**

用户把本机 Claude 配置修好之后重跑探针，结果是这样：

```text
terminalTail: "Hi! How can I help you today?"     ← Claude 真的回答了
requestCount: 0                                    ← 但网关一条请求都没收到
```

**它答得很好，只是没走我们的网关。** 查它的设置文件，一眼就清楚了：

```json
// ~/.claude/settings.json
{ "env": {
    "ANTHROPIC_AUTH_TOKEN": "<用户的密钥>",
    "ANTHROPIC_BASE_URL": "https://ooioo.work",
    … },
  "model": "sonnet" }
```

**Claude Code 自己设置里的 `env` 块优先于进程环境**，于是我们注入的
`ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN` **被完全覆盖**。

##### 为什么能确定是「被覆盖」而不是「没送到」

`command.env` 这条通道是**通的**，证据在 Codex 那一侧：Codex 的地址走的是 `-c` 命令行配置，
但它的**凭据**走的是我们注入的 `OPENAI_API_KEY`（`env_key = "OPENAI_API_KEY"`）——
而它的 29 条请求里有 3 条带着真实 `vendorId`、成功地被转发出去，
说明那一份环境变量确实到达了子进程并通过了网关鉴权。

所以：环境送到了，Claude 没用它。

##### 后果比 Codex 那个更隐蔽

| | Codex 的 `CODEX_API_KEY` 遮蔽 | Claude 的设置覆盖 |
| --- | --- | --- |
| 表现 | CLI **报错**（`invalid type` 之类）或直连官方 | **一切正常**，界面照常回答 |
| 用户怎么发现 | 看终端 | **只能从账单上发现** |

用户看到 Claude 一切正常，而请求去了 `https://ooioo.work`、**用的是他自己那把密钥**。
这正是模块文档警告的那种失败——「走网关没生效，且在花自己的额度」——
只是这次是**另一条路径**造成的。

##### 一处必须说清楚的事：这不是本次移植的回归

老平台注入的是**同样两个变量**（`terminal-sessions.ts` 里的 `ANTHROPIC_BASE_URL` /
`ANTHROPIC_AUTH_TOKEN`），因此**它在同样配置的机器上也会被覆盖**。
这不是移植引入的缺陷，而是**这套注入方式本身的限制**。

##### 修复方向（已核实可行）

`claude --help` 里有 `--settings <file-or-json>`，帮助文本明确写了它是高优先级
（"managed settings and `--settings` still apply"）。

也就是说：**命令行/`--settings` 级别的配置压得住用户的设置文件**——
Codex 那边正是这样（`-c` 命令行压得住 `config.toml`），所以 Codex 一直没这个问题。

因此正确做法是：为 Claude 生成一份路由设置 JSON，用 `--settings` 传进去，
而**不是**指望环境变量。这样网关才是权威的。

这一条**本轮只诊断、未实现**：它要给 Claude 构造器新增一条传参路径
（生成临时 JSON + 拼 `--settings`），并补一条「用户设置里有别的地址时仍然走网关」的测试。
值得单独一轮做，而且做完要能用探针证明 `requestCount > 0`。

#### 31.5 ✅ 已修并验证：Claude 的路由改用 `--settings` 注入

**改法**：`claude_route_settings()` 生成一份内联 JSON（`env.ANTHROPIC_BASE_URL` +
`env.ANTHROPIC_AUTH_TOKEN`），由 `build_command` 拼成 `--settings '<json>'`，
放在**用户参数之前**（与 Codex 的 `-c` 同一位置约定）。
环境变量**仍然照常注入**：老版本或未读 `--settings` 的场合那条路还在。

**验证（探针，真实 CLI）**：

| | 修复前 | 修复后 |
| --- | --- | --- |
| `terminalTail` | `"Hi! How can I help you today?"` | **空** |
| `requestCount` | **0** | **7** |
| 含义 | 它偷偷走了用户配置的地址，**一切看起来正常** | 请求到了我们的网关，因上游是故意打不通的端口而失败——**这正是应该发生的事** |

**四个单测**钉住形状：JSON 能被解析回两个值、排在用户参数之前、内联 JSON 不带空格、
只有 Claude 带 `--settings`（Codex / Gemini / Qoder 都不带）、WSL 形态里也在、
**没有路由时不注入**。

还确认了 POSIX 形态下整个 JSON 是**一个**单引号参数（`'{"env":{…}}'`），
因此里面的双引号既不破坏 shell、也不会被空格拆开。

#### 31.6 同一族的风险在 Gemini 上**尚未核对**

Gemini CLI 用 `GOOGLE_GEMINI_BASE_URL` / `GEMINI_API_KEY` 两个环境变量，
而它同样可能有自己的设置文件（`~/.gemini/settings.json`）**覆盖进程环境**——
那样就会重现刚修掉的这个症状。**本机没装 Gemini，无法验证。**
记在这里：修 Claude 时用的是「命令行压过配置文件」这条通则，
Gemini 需要先确认它有没有对应的命令行入口。


### 33 · 状态栏供应商选择器的组件测试（本轮）

`vendor-route-picker.tsx` 是界面这一侧**唯一没有测试的新增件**，而它承担的是
「我现在花的是谁的钱」——走网关之后供应商可能被**自动**换掉，界面不显示或显示错，
用户只能从账单上发现。

新增 `ui/src/app/vendor-route-picker.test.tsx`，7 条：

| 用例 | 钉住的行为 |
| --- | --- |
| 没有终端 | 不渲染任何东西 |
| 系统终端 | 不显示供应商（它不注入环境、不绑路由） |
| 没有绑定 | 显示**「直连」**，而不是一个空的选择器——用户需要能区分「没配」与「配了没生效」 |
| 有绑定 | 下拉框选中它；改选会请求切换，参数是 `(terminalId, vendorId)` |
| 当前供应商已停用 | **仍然列出来并标明**——不能悄悄跳到另一家 |
| 自动切换徽标 | **只有 `failure`** 显示；`candidate-pool` 是路由调整，不该当故障提示 |
| 切换失败 | 错误显示出来（不是静默成功） |

**false-green 检查过**：把徽标的门槛从 `reason === "failure"` 改成「任何切换都显示」，
正好那一条红、其余六条绿。

顺带把前端全量跑了一遍：**62 个测试 + 类型检查 + 构建全绿**，
并重建了嵌界面的二进制（`ui/dist` 比 exe 新时验证的会是旧界面）。

#### 33.1 一条**决定不加**的东西：`NVM_SYMLINK`

这个待办挂了两轮，本轮给出结论：**不加**。

`locate::fallback_dirs` 已经把 `%NVM_HOME%\<每个版本目录>` 都补进 PATH，
覆盖面**比那个符号链接更广**——符号链接只指向其中一个版本，而它指向的那个版本目录
本来就在我们的清单里。再加它属于照搬，不是改进。

### 35 · 过期熔断的归一化：补上真实数据库的用例（本轮）

熔断是有期限的。库里存着一行「**开着、但截止时间已经过去**」的记录时，如果读回后
仍然算「开着」，那台供应商就**永远不会再被尝试**——用户看到的是一台怎么都不用的供应商，
而且**没有任何报错**。

新增 `an_expired_circuit_is_normalized_when_read_back`：往真实数据库写一行
`status = open` + `circuit_until` 在 2020 年的记录，走 `wire_health` + `hydrate_health`
读回，断言它**不再是开着的**。

#### 35.1 顺手核实了一句我自己写下的话

我在测试文档里写了「归一化发生在**仓储的读取路径**上（`normalize_status`）」——
这是一句**可验证的事实断言**，而它决定了这条测试到底在测什么：如果 `upsert` 在**写入时**
就已经归一化了，那这条测试根本没碰到读取路径。

**false-green 检查过**：把 `normalize_status` 里「已过期 → `HalfOpen`」改成「保持 `Open`」，
测试立刻红。**所以那句话是真的**：救回来的是读取路径，写入时存的就是 `open`。

> 这类检查顺带能把「我写下的解释」也变成被验证过的东西，而不只是注释。

### 37 · `successCount` 的口径：查清了，**不是缺陷**

端到端跑时看到一个反常的数字：`requestCount: 7, successCount: 1, failureCount: 6`，
而那一轮的上游是**故意打不通**的端口——**哪来的一次成功？**

#### 37.1 分类表（两侧逐行核对过）

| 路径 | 老平台 | Rust | 结论 |
| --- | --- | --- | --- |
| 路由不存在（404） | `outcome = "ok"`（L301） | `mark_ok()` | 一致 |
| 外部 API 关闭 / 令牌不对（503 / 401） | `outcome = "ok"`（L307/313） | `mark_ok()` | 一致 |
| 路由令牌不对（401） | `outcome = "ok"`（L325） | `mark_ok()` | 一致 |
| **没有可用供应商（503）** | `outcome = "ok"`（**L342**） | **`error`** | **有意偏离** |

于是我看到的那个数字有了完整解释：

- **6 条失败** = 熔断打开后选不出供应商的 503——Rust 把这类算作**失败**；
- **1 条成功** = 一条**根本没转发**的请求（路由不存在或鉴权失败），走了 `mark_ok()`。

#### 37.2 那处「有意偏离」是对的

老平台把「没有可用供应商」也记成 `ok`，而 Rust 记成 `error`，注释写着：

> 没有可用供应商：记录里保留空供应商，但结果分类必须是失败，
> 否则用量报表会把这类请求算成正常流量。

**移植时在这一点上比现网更正确**——请求一条都没发出去，却记成成功，那正是会让用量报表
失真的口径。这一处**保留**。

#### 37.3 剩下的那半（`ok`）也不改，但值得知道它的代价

为什么「路由不存在 / 鉴权失败」记成 `ok` 是**可辩护**的：它们**不是供应商的错**，
是客户端的问题；把它们记成失败会污染按供应商统计的失败率与熔断判定。

代价是一个**可诊断性**问题：

```json
{ "outcome": "ok", "vendorId": "", "upstreamStatus": null }
```

**这条记录和一个真正转发成功的记录长得几乎一样**——`mark_ok()` 会清掉错误字段，
而这些路径又不设 `upstreamStatus`。于是有人查「我的 CLI 为什么失败」时，
在 `gateway:query-logs` 里看到的是一串 `ok`，而它们其实是 404。

**本轮不改**（改它要新加一个「成功但带原因」的表达方式，属于另一件事），
但记在这里：**一个「成功」的记录应当至少能被区分为「转发成功」与「没转发」。**

### 39 · 让「没转发」的记录能被看出来（本轮）

§37.3 记下的那个可诊断性问题：`outcome: ok` + 空供应商 + 空上游状态，
**与一次真正转发成功的记录几乎一样**。有人查「我的 CLI 为什么失败」时，
在 `gateway:query-logs` 里看到的是一串 `ok`，而它们其实是 404。

#### 39.1 改法与为什么这样改是安全的

新增 `RequestTracker::mark_ok_with(code)`：**保持 `outcome = ok` 不变**，只补一个原因码。

不改 `outcome` 是有理由的：那些路径**不是供应商的错**，记成失败会污染按供应商统计的
失败率与熔断判定——§37 已经查清老平台也是这么分的。

安全性先验证过再动手：

| 消费者 | 过滤/统计依据 | 受影响吗 |
| --- | --- | --- |
| 失败列表 / 故障诊断 | `outcome IN ('error','timeout')`（**不是**按 `error_code` 过滤） | 否 |
| 用量汇总 | `outcome = 'ok'` / `outcome <> 'ok'` | 否 |

因此补这个码**不会**把这些记录混进失败列表，也不会改变任何计数。

四处路径各带自己的码：`ROUTE_NOT_FOUND`（路由不存在、平台不匹配）、
`EXTERNAL_API_DISABLED`、`UNAUTHORIZED`（外部令牌缺失/不对、路由令牌不对）。
另外把 `errorCode` 加进了 `gateway:query-logs` 的返回，界面才看得到。

#### 39.2 验证

扩展了既有的 `a_rejected_request_is_still_recorded`（它本来就覆盖「路由不存在也要留记录」）：

```rust
assert_eq!(records[0].outcome, GatewayOutcome::Ok);
assert_eq!(records[0].error_code.as_deref(), Some("ROUTE_NOT_FOUND"));
```

两半都断言：**口径没变**（仍是 `ok`）**且原因可辨**（`ROUTE_NOT_FOUND`）。

### 41 · 回到界面层：先盘点，再补两处缺口（本轮）

按 §40 的建议做了一次「核心能力在界面上看得到多少」的盘点。结论比预想的好，
但**有两处明显的接线缺口**。

#### 41.1 盘点结果

界面**已经接了**：`getVendorHealth`（健康）、`queryGatewayLogs`（日志）、
`getUsageSummary`（用量）、`listFailoverRules` / `saveFailoverRule`（故障规则）、
`getExternalApiStatus`、`getGatewayPort`、`switchVendorRoute` / `setVendorRouteMode`
（状态栏选择器）、`CHANNEL_GATEWAY_VENDOR_SWITCHED`（自动切换通知）。

**缺的是两处**：

| 缺口 | 性质 |
| --- | --- |
| `gateway:request-recorded` **没有任何订阅方** | 后端**每转发一个请求**都推一条，而界面一处都没接 |
| `gateway:reset-vendor-health` **没有入口** | 命令实现了、也刚修好，但界面上按不到 |

第一处特别典型：后端 `lib.rs` 的注释写着「工作台靠这个事件实时更新，**不轮询**」——
**那一半从来没写**。于是「实时」从未成立，用户只能关掉面板再打开。

#### 41.2 改动

1. `commands.ts`：补 `CHANNEL_GATEWAY_REQUEST_RECORDED` 常量与 `resetVendorHealth()`；
2. `config-overlay.tsx` 的 `GatewayInsights`：订阅该事件并刷新，
   **刷新做合并**；健康区补一个「重置全部健康」按钮。

**为什么必须合并刷新**：一次刷新要拉三个接口，而一个终端在重试循环里一秒能发好几条——
不合并的话一次重试就能把数据库打满，而面板上的数字并不会因此更准。

#### 41.3 唯一有逻辑的部分抽出来测了

`app/refresh-coalescer.ts` + 5 条测试。这段逻辑的价值全在**边界**上，而两个方向都能写错：

- 把重复请求「**重新计时**」→ 持续流量会让刷新永远排不上，**面板反而不更新**；
- 不合并 → 每条事件拉三个接口。

因此最关键的一条断言是：**20 条事件横跨 200ms、窗口 100ms → 正好刷 2 次**
（每窗口一次，不是 20 次、也不是 0 次）。写这条时我自己先写错了期望值（写成 1 次），
是测试把它纠正过来的。

#### 41.4 如实说明：新接的这两处**没有自动化测试**

`refresh-coalescer` 有测试，但**订阅接线本身与那个按钮没有**——
它们在一个 1380 行的组件里，渲染它需要搭整套界面上下文。
目前只做到：类型检查通过、前端 67 个测试全过、构建通过、后端门禁全过。

**这是缺口，不是「已经验过了」。** 要补它得先把 `GatewayInsights` 从大文件里拆出来。

### 43 · 把网关面板拆出来，并给它补上测试（本轮）

§41.4 留下的缺口是自己造成的，所以这一轮先把它补掉。

#### 43.1 拆分

`GatewayInsights`（约 200 行）从 1397 行的 `config-overlay.tsx` 搬到
`app/gateway-insights.tsx`。**它是那 10 个面板里唯一有跨组件接线的一个**
（订阅后端事件、合并刷新、调命令），留在大文件里就只能靠肉眼看界面验证。
主文件因此从 1397 行降到 1191 行。

#### 43.2 五条组件测试

| 用例 | 钉住的行为 |
| --- | --- |
| 订阅了「请求已记录」事件 | 不订阅就谈不上「实时」 |
| 收到事件之后会重新取数 | 事件真的触发了刷新（要等合并窗口过去） |
| 一串事件只触发一次刷新 | 5 条事件 → 刷新**恰好 +1**，不是 +5 |
| 健康表为空时说明原因 | 「还没有请求记录」≠「都健康」 |
| 重置按钮走命令 | 调 `gateway:reset-vendor-health` **并且**重新取数——只在界面上清空表格是假的重置 |

**false-green 检查过**：把订阅那一段短路掉，**正好那三条接线相关的红**，
另外两条绿的。

#### 43.3 ⚠️ 过程中的一次事故（如实记）

拆分时我用 PowerShell 做了行切片：

```powershell
$lines = Get-Content $path            # ← 没有 -Encoding utf8
Set-Content -Path $path -Encoding utf8
```

结果**整个文件的中文变成乱码**，而且行号切错了、组件没删掉。
`aivc-rs` 目前**未提交**（`?? aivc-rs/`），没有 git 兜底。

**救回来靠的是上一次 vite 构建的 sourcemap**——`dist/assets/*.js.map` 的
`sourcesContent` 里存着原文。用 Node 取回后按 utf8 原样写回，中文与行数都复原了，
再用**唯一字符串边界 + 断言**（而不是行号）删块。

三条教训，都已落到做法上：

1. **同一个坑我踩了第二次**：本项目早先就记录过「`Get-Content` 不带 `-Encoding utf8`
   会毁掉中文」，这次我把它用在了最大的那个源文件上；
2. **不要用 PowerShell 做文本往返**——改文件用能保证字节精确的方式
   （write 工具 / Node 按 utf8），PowerShell 只用来跑命令和读信息；
3. **未提交的代码没有安全网**。顺带的收获是发现 sourcemap 可以当恢复手段——
   但那只是运气好（构建产物恰好是最新的）。

### 45 · 给 `aivc-rs` 建了独立仓库（本轮）

上一轮的事故（§43.3）暴露的问题：**未提交的代码没有安全网**。
按你的选择，在 `aivc-rs/` **内部**建了独立仓库，与旧平台完全隔离。

#### 45.1 做法与结果

| | |
| --- | --- |
| 位置 | `D:\code\ai\aivc-rs\.git`（`git rev-parse --show-toplevel` 确认就是它自己） |
| 远端 | 无 |
| 被跟踪 | 217 个文件，`.git` 1.1 MB |
| 提交 | 3 条：初始提交 → CI → 移除误提交的临时文件 |
| 旧平台 | `src/` 改动 **0**；`aivc-rs/` 在它眼里仍是 `?? aivc-rs/`（未改动旧仓库的任何设置） |

沿用工程里现成的 `.gitignore`（已排除 `.toolchain/` 2.4 GB、`target/` 80 GB、
`node_modules/`、`ui/dist/`、`/logs/`、`/data/`、`/shots/`、`*.db*`）。
提交前做了路径自检，217 个文件里没有一条命中这些目录。

**没有**在旧平台仓库里执行任何提交——`git config` 也只动了仓库级。

#### 45.2 顺手修掉一处「独立之后就失效」的假设

旧平台仓库里有一份 `.github/workflows/aivc-rs-ci.yml`，是**为「本工程是它的子目录」**
写的（`working-directory: aivc-rs`、路径过滤 `aivc-rs/**`）。独立成仓库后那个假设不再成立。

于是在新仓库里放了 `.github/workflows/ci.yml`：去掉全部 `aivc-rs/` 前缀与
`working-directory`，并补了一个前端 job。旧平台那份**原处不动**（我不改它的文件），
但要清楚：**它覆盖不到这个独立仓库**。

CI 里有两条注释是踩过的坑，特意留着：

1. **不要把 `scripts/verify.ps1` 当 CI 入口**——它把 `CARGO_HOME` / `RUSTUP_HOME`
   重定向到工作区内的 `.toolchain/`，CI 里没有那个目录；
2. **Windows 上检查 `scripts/*.ps1` 的 UTF-8 BOM**——没有 BOM 时 PowerShell 按 ANSI 读，
   中文乱码并报出与真实原因无关的解析错误（§43.3 正是这个坑的极端版本）。

提交前逐条核对过 CI 的假设：`ui/package-lock.json` 在、`deny.toml` 在、
五个 `scripts/*.ps1` 全都有 BOM。

#### 45.3 过程里的一次小失误

`git add -A` 把临时写提交信息用的 `ci-msg.txt` 一起提交了（因为我是先建文件、
后 `add`）。已经用一条 `git rm --cached` 的提交修掉，最终 217 个文件、工作区干净。

**教训**：临时文件要么放在被忽略的路径下，要么用 `git commit -F` 之外的方式传信息。

### 47 · 「持久化语义」这条线索收尾：路由**本来就该是即时的**

§40 起我挂着一条待办：「路由仍是纯内存的（重启即丢），先确认老平台是否也如此」。
本轮查完了，**结论是我当初的担心是误报**。

#### 47.1 老平台也是纯内存

```ts
// vendor-gateway.ts:52-53
const routes = new Map<string, MutableVendorRoute>();
const externalRoutes = new Map<AiProviderId, MutableVendorRoute>();
```

全文件唯一的持久化是**请求日志**（`persistGatewayRequest`）。路由没有任何落盘。
生命周期只有三处：`:153 set`（终端启动时建）、`:203 delete`（终端停止）、`:208 clear`（网关停止）。

#### 47.2 而且这样是**对的**，不是在偷懒

一条路由与**一个正在运行的终端**绑在一起：终端的环境变量（`ANTHROPIC_BASE_URL`
之类）在启动时就写进子进程了，进程一没，那条路由就没有任何意义。

而终端**不跨进程重启存活**——所以「重启即丢」不是遗漏，是**定义**：
重启之后本来就该是零条路由，等新终端起来再建。

#### 47.3 唯一一处差异，以及为什么它**观测不到**

老平台 `stopVendorGateway()` 会 `routes.clear()`。Rust 这边没有对应的清空动作
（`GatewayHandle::shutdown` 只关服务器；`RouteRegistry` 没有 `clear`）。

关键在于**有没有「进程内停止/重启网关」这条路径**：

| | 老平台 | Rust |
| --- | --- | --- |
| 进程内停止网关 | 有（`stopVendorGateway`） | **没有**：`start_gateway_if_enabled` 只在进程启动时调一次（`main.rs:77`），全仓没有 `stop_gateway` |
| 因此「停止后残留旧路由」的窗口 | 存在，所以要清 | **不存在** |

所以这处差异**今天观测不到**：进程退出时整个状态一起消失，清不清都一样。

**但要记下来**：如果将来加了「改设置后热重启网关」，就**必须**同时加清空——
否则旧路由（带着旧令牌、指向旧端口）会留在表里。若新端口恰好复用，
那些本该失效的令牌会重新变得可用，那是**安全相关**的，不只是整洁问题。

#### 47.4 这条线索的总结

| 东西 | 老平台 | Rust | 结论 |
| --- | --- | --- | --- |
| 请求日志 | 落库 | 落库 | 一致 |
| 供应商健康 / 熔断 | 落库，且**每个请求前读回** | 落库（本轮接上），**启动时读回一次** | 一致（读回时机有意不同，见 §29.3） |
| 故障规则 | 落库 | 落库 | 一致 |
| **路由** | **纯内存** | **纯内存** | **一致，且正确** |
| 网关停止时清路由 | 清 | 不清（无此路径） | 无差异（观测不到），但已记条件 |

**「持久化语义」到此收尾**：三处该落库的都落了，路由不用落，一处差异有条件地记下。

### 48 · 「已知缺口」收敛成一页清单（本轮）

这份文档已经 1800 余行、48 节。它记录的是**诊断过程与证据**，但「还剩什么没做」
散落在 §22.3、§24.4、§31.6、§37.3、§47.3 这些地方——需要一张只回答
「现在还剩哪些已知缺口」的索引。

新增 [`31-open-items.md`](./31-open-items.md)，按**性质**分组（而不是按发现时间）：

| 组 | 内容 |
| --- | --- |
| A | 阻塞交付的缺口 —— **没有** |
| B | **有意的行为差异**（7 条）—— 列出来是为了不再被反复提起 |
| C | 验证缺口：实现看着对、但**没有证据**（4 条，其中 1 条本轮关闭） |
| D | 需要外部条件，当前无法验证（3 条，含 Gemini） |
| E | 早期记录里标着「未完成」的条目 —— **本轮逐条核对** |
| F | 工程整洁（3 条，低优先） |
| G | 交付前必须处理的**非代码**事项：轮换那把密钥 |

每一项都给了**复核方法**，因此这张清单不依赖我对旧记录的印象。

#### 48.1 核对 E 组的结果：两条是**假待办**

写清单时发现早期 §6.7 有三条「未完成」，本轮核对了前两条：

- **WSL 运行时手工验证** —— **其实早就做完了**：本会话用真实 Codex 在 WSL 里跑通了
  整条链路。文档里那条是**过时的**。
- **`candidate-pool` 从来不产生** —— **其实是错的**：`server.rs:655` 确实会产生它，
  `:2319` 还有测试断言「按正文匹配的失败不转移，但候选池变化要写回」。而且界面
  **有意**不为它显示徽标（`:646` 的注释写明只对 `failure` 显示），与现网一致。
- 剩下的「工作台按平台开终端的入口」范围缩小到一处调用点，已写进清单。

> 这两条的价值不在「省了活」，而在**暴露了文档本身的问题**：
> 我把「待办」写进正文之后就没再回头勾掉，于是它们会一直看起来像缺口。
> 这也是为什么这张清单要**独立成页**——它需要被勾掉，而正文不需要。

### 50 · 收清单上的三条低成本项（本轮）

按 [`31-open-items.md`](./31-open-items.md) 的排序，做掉 C3、E2、F1。

#### 50.1 C3：订阅卸载后要真的解绑 —— **✅ 关闭**

`gateway-insights.test.tsx` 加了「卸载之后解绑，不再订阅后端事件」。
不解绑的后果不是抽象的：组件没了还在往它上面写状态，而且**每卸载重开一次就多一个
幽灵订阅**，各自触发一次取数。

**false-green 检查过**：去掉 `unlisten?.()`，正好那一条红、其余五条绿。

#### 50.2 E2：界面**不能独立选平台** —— **🔴 确认是缺口**

原来记的是「界面入口只覆盖『选中目标』这一条路」。本轮读代码把它确定下来：

```ts
// ui/src/app/use-terminals.ts:306
providerId: target?.provider ?? "codex",
```

- 界面里唯一的开终端路径是 `openTerminal(target, cwd)`，**供应商来自目标**；
- 而**后端支持**只给 `providerId`、不给 `targetId`（`terminal:start` 的 `targetId` 是可选参数）；
- **后果**：没有配置对应平台的**目标**时，开不出那个平台的终端——
  想开一个 Claude 终端，得先建一个 Claude 目标。这是**产品能力的缺失**，不是接线问题。

修法很轻（`open` 已经接受 `providerId`，只差给调用方一个入口），因此**没有在本轮动手**：
它是一次界面交互设计（放哪里、怎么选），不该顺手塞进去。

#### 50.3 F1：`.gitattributes` —— **✅ 完成**

`text=auto` + 二进制标注 + **`*.ps1 -text`**。

最后一条是有意的：那几个脚本靠 **UTF-8 BOM** 让 PowerShell 正确按 UTF-8 读，
应当保持逐字节原样，别让换行或编码被 Git 改。

**加之前先确认了它不会造成大面积改动**：加完 `git status` 只有两项
（新文件本身 + 本轮的测试改动）。没有为了消掉 CRLF 警告去强制 `eol=lf`——
那会改写一批文件的换行，代价大于收益。

### 51 · 下一轮的建议顺序

1. **E2：给界面补一个「选平台开终端」的入口** —— 这是清单上**唯一一条产品能力缺失**，
   其余不是「需要环境」就是「工程整洁」。它需要先定交互（放在工作台还是新建流程里）。
2. **G：轮换那把密钥** —— 功能已接通，这是当初约定的时机。
3. **D1：Gemini 的同类风险** —— 等有环境时再安排。

#### 一条与本项目无关、但必须提的操作提醒

排查过程中我读了 `~/.claude/settings.json`，它的内容（含一把明文 API Key）**打进了本次会话的
输出**。我没有把它写进任何文件或文档，但用户已表示会在功能接通后轮换该密钥。
这是我的疏忽——当时只需看键名即可。

