# 09 · 目标列表：与原平台的逐条差异

> 本文回答一个具体问题：**「选了平台，目标列表不完整」**。
>
> 结论先说：这是我这边的**功能回归**，不是原平台的问题。原平台对每个 WSL 发行版
> **无条件**产出一个目标；我加了两道 `continue` 把「探测不理想」的发行版过滤掉了。
>
> 同时本文列出**无法复制**与**有意不复制**的差异，以及仍未解决的问题。

---

## 1. 根因

原平台 `src/electron/providers/codex/codex-targets.ts` 的 `probeAllTargets()`：

```js
const distros = await listWslDistros();
const wslTargets = distros.map(async (distro) => {
  const overrideCodexHome = await getWslCodexHomeOverride(distro);
  const probe = overrideCodexHome ? null : await probeWslDistro(distro);
  return {
    id: buildWslProviderTargetId("codex", distro),
    ...
    codexHome: overrideCodexHome || probe?.codexHome,
    available: true,                                    // ← 恒为 true
    detail: overrideCodexHome
      ? `使用手动路径 ${overrideCodexHome}`
      : probe?.detail || "可进入 WSL，未找到 Codex 目录"
  };
});
// 没有 filter，没有 continue：每个发行版都成为一个目标
```

我的实现（`crates/aivc-provider/src/codex/provider.rs`，**改前**）有两道原平台没有的跳过：

```rust
let home = ctx.wsl().resolve_codex_home(&distro).await;
if home.is_empty() { continue; }                       // 跳过 1
if !ctx.wsl().exists(&distro, &sessions_root).await {
    continue;                                          // 跳过 2
}
```

**实机后果**（`wsl -l -q` → `docker-desktop / Ubuntu-22.04 / Ubuntu-24.04`）：

| 发行版 | `~/.codex/sessions` | 原平台 | 我（改前） |
| --- | --- | --- | --- |
| Ubuntu-24.04 | 有 | 显示 | 显示 |
| Ubuntu-22.04 | **无** | 显示 | **被跳过 2 拦掉** |
| 本机 | 有 | 显示 | 显示 |

所以用户看到的目标比原平台**少一个**，而界面上没有任何解释。

### 为什么「跳过」比「不显示」更糟

被跳过的发行版连带失去一条**恢复路径**：WSL 内的 Codex 装在非标准位置时，用户要靠
`codex:set-wsl-codex-home`（「手动指定 Codex 目录」）把它接进来，而那个入口只对
**已经出现在列表里**的目标开放。

**目标被跳过 = 永远无法配置。**

---

## 2. 改了什么

| # | 改动 | 位置 |
| --- | --- | --- |
| 1 | 删除两道 `continue`，改为**无条件**产出目标 | `codex/provider.rs` `wsl_targets()` |
| 2 | 把「构造目标」抽成没有 `Option`/`Result` 返回路径的纯函数 | 同上 `wsl_target()` |
| 3 | 新增 `probe_codex_home() -> Result<String, String>`，区分「进不去」与「没找到目录」 | `aivc-infra/src/wsl/adapter.rs` |
| 4 | **删除** `resolve_codex_home() -> String`（它把失败折叠成空串，是撒谎的源头） | 同上 |
| 5 | 空 `codex_home` 增加 guard，避免退化成相对路径查询 | `codex/provider.rs` `list_files()` |
| 6 | 本机目标判据从「`sessions` 目录存在」放宽为「配置目录存在」 | `codex/provider.rs` `local_target()` |

第 2 条是**结构性**的：`wsl_target()` 的签名里根本没有「不构造」这条返回路径，
因此「因为探测不理想所以不显示」这个曾经出错的判断**在代码里已不存在**。

第 5 条是第 1 条**激活**的：此前只有解析出配置目录的发行版才会成为目标，空 `home`
不可达；现在不可达了，必须兜住——否则 `find` 会在发行版当前目录下按相对路径乱找。

第 6 条是同一类错误的另一处：要求 `sessions` 必须存在，意味着**全新安装**（CLI 装好、
还没跑过任何会话）的用户看不到本机目标。「列表为空」与「根本没有这个目标」是两件事。

---

## 3. 有意**不复制**：原平台在这里是错的

原平台对**每一个** WSL 目标都写 `available: true`——包括**根本进不去**的发行版。

实测存在这种发行版：注册在册，`wsl.exe -d Ubuntu-22.04` 却返回

```
Wsl/Service/CreateInstance/MountDisk/HCS/E_ACCESSDENIED
```

（`D:\wsl-os\ai\ext4.vhdx` 挂载被拒）。原平台会把它与可用的发行版显示得**一模一样**，
用户点下去只会收到失败。

我的实现如实标注：

```
available: false
detail: "无法进入 WSL：Wsl/Service/CreateInstance/MountDisk/HCS/E_ACCESSDENIED"
```

**照样列出**（用户要能看见它、要能对它做「手动指定」），但不可用。
附带好处：前端「自动选中第一个可用目标」不会选中它。

因此 `detail` 现在有**四态**，逐字互不相同：

| 状态 | `available` | `detail` |
| --- | --- | --- |
| 找到 `sessions` | `true` | `找到 Codex 会话` |
| 有配置目录、无 `sessions` | `true` | `找到 Codex 目录` |
| 进得去、无 Codex | `true` | `可进入 WSL，未找到 Codex 目录` |
| **进不去** | **`false`** | `无法进入 WSL：<原因>` |

前三种沿用原平台措辞（`可进入 WSL，…` 读起来像**邀请**而不是报错）。第四种是本工程新增。

---

## 4. 差异总表

| 项 | 原平台 | 我（改前） | 我（改后） |
| --- | --- | --- | --- |
| WSL 发行版过滤 | 无 | 两道 `continue` | 无 |
| WSL `available` | 恒 `true` | 恒 `true`（但常被跳过） | 可达 `true` / 不可达 `false` |
| WSL `detail` | 3 态 | 仅路径 | 4 态 |
| 内部发行版过滤 | `docker-desktop`、`docker-desktop-data` | 同 | 同 |
| 本机判据 | 配置目录存在 **或** CLI 存在 | `sessions` 目录必须存在 | 配置目录存在 |
| 本机 `detail` | 3 态、不带路径 | 仅路径 | 2 态、**带路径** |
| 本机 `id` | `local` | `codex:local:<home>` | 不变 |
| 排序（win32） | WSL 在前、**本机最后** | 本机在前 | 本机在前（有意保留） |
| 标签 | `本机（win32）`、`WSL：X` | `本机 Codex`、`WSL · X` | 不变 |
| 内存 TTL 缓存 | 30 秒 | 无 | 无（见 §6） |
| 并发探测合流 | `targetsInFlight` | 无 | 无（见 §6） |

---

## 5. 无法复制：本机 CLI 存在性检查

原平台本机目标的判据是 `hasCodexHome || codexFound`，其中

```js
const codexFound = await commandExists(process.platform === "win32" ? "codex.cmd" : "codex");
```

`detail` 里也有一态是 `找到 Codex 命令`。

**我无法在 provider 层实现这一条，这是分层约束导致的，不是遗漏：**

- `aivc-provider` 的依赖只有 `aivc-core`（层 0）与 `aivc-infra`（层 1）；
- 宿主命令探测能力在 `aivc-tooling::cli_env`（层 2，与 `aivc-provider` 是**兄弟模块**）；
- 兄弟之间互相依赖会破坏 `cargo xtask check-layers` 的分层校验。

**行为差异**：`~/.codex` 不存在、但 CLI 已安装时——

| | 表现 |
| --- | --- |
| 原平台 | 显示 `本机（win32）`，`available: false`，`detail: 找到 Codex 命令` |
| 我 | **不显示本机目标** |

**正确做法**是把它接到 `cli_env` 子系统——那是架构里回答「CLI 是否安装」的**唯一**位置
（`checkCliEnvironment` / `installCli` 已经在那里）。在 provider 里再写一份
`where codex.cmd` 会制造第二份真相并破坏分层。

**我没有擅自加依赖，也没有复制一份探测。** 这一条属于「已知未实现」，不是「已实现」。

影响面小：`available: false` 的本机目标做不了任何会话操作。

---

## 6. 尚未实现的性能语义

原平台有两处我还没做：

1. **内存 TTL 缓存**（`TARGETS_CACHE_TTL_MS = 30_000`）——30 秒内重复调用直接返回上次结果。
2. **并发合流**（`targetsInFlight`）——两个并发请求若同时穿透 TTL，合流为一次探测。

我只有**持久化**缓存（供前端首屏渲染）。因此短时间内重复调用 `list-targets` 会**重复探测**
（每个发行版再跑一次 `wsl.exe`）。

直接影响：前端 `loadInitialTargets` 的策略是「先渲染缓存 → **1500ms 后**再次刷新」，
第二次调用在我这边**不会短路**。

未做原因：它需要一份跨请求共享的可变状态，而当前上下文是每命令构造的。这是架构决定，
不该顺手加——加错地方会比不加更糟（本次的 `continue` 就是「顺手优化」的产物）。

### 顺带记录：30 秒的根因假设（**未验证**）

`listing_wsl_sessions_is_fast` 仍是 `#[ignore]`：列出 WSL 会话耗时 30.5 秒，
过于接近 30 秒超时。

原平台与我的调用次数相同（每个会话文件一次 `head`），差别在于**传输层**：

| | 原平台 | 我 |
| --- | --- | --- |
| 简单探测 | `wsl.exe -d X -- head -c 65536 <path>`（**直接执行**） | `wsl.exe -d X -- bash -lc "printf <b64> \| base64 -d \| bash"`（**登录 shell**） |

我的每次调用都多一次登录 shell 启动 + base64 解码。若单次登录 shell 在此环境约 1 秒，
30 次即 30 秒——**与观测值吻合**。

我尝试在本机测量两种方式的耗时，但沙箱拒绝了 `wsl.exe`（见 §7），测得的数字无效，
因此**这只是假设，没有证据**。下一步应当：给简单、无注入风险的操作用「直接执行」路径
（`spawn_program` 已具备），只对真需要脚本的操作保留 base64 + 登录 shell。

---

## 7. 验证环境的重要限制

**我的沙箱拒绝 `wsl.exe`。** 默认模式下：

```
wsl.exe -l -q  →  退出码 -1
Wsl/EnumerateDistros/Service/E_ACCESSDENIED
```

因此：

- 新的守门集成测试第一次跑起来时**在 0.06 秒内「通过」了——它什么都没验证**。
  返回类型已从 `Option` 改为 `Result<_, String>`，跳过时**必须打印原因**：
  `⚠ 跳过：<原因>——本用例没有验证任何东西。`
  **假绿比红灯危险得多**：红灯会有人去修，假绿只会让人以为已经测过了。
- 上述实机数据都是在**放宽权限后**取得的，用户的实际运行环境不受此限制。

**我无法在沙箱内复现用户的确切环境，最终确认需要用户运行一次。**

---

## 8. 证据

### 单元测试（`crates/aivc-provider/src/codex/provider.rs`）

- `wsl_target_is_produced_even_when_no_codex_home_resolves`
- `an_unreachable_distro_is_listed_but_marked_unavailable`
- `wsl_target_detail_distinguishes_the_four_states`
- `every_wsl_distro_yields_a_target_regardless_of_its_state`

### 守门集成测试（`crates/aivc-app/tests/wsl_targets_probe.rs`）

以 `wsl.exe -l -q` 的**真实输出**为基准，断言每个非内部发行版都出现在目标列表里：

```
wsl.exe 报告 ["Ubuntu-22.04", "Ubuntu-24.04"]；目标列表给出 ["Ubuntu-22.04", "Ubuntu-24.04"]
test result: ok. 1 passed; 0 failed
```

**Ubuntu-22.04 回来了。**

### 门禁

`scripts/verify.ps1` 七项全部通过：格式、分层校验、`check`、`check --all-features`、
`clippy -D warnings`、`clippy --all-features`、`test`。

---

## 9. 仍未解决

| 问题 | 状态 |
| --- | --- |
| WSL 会话列表 30 秒 | **未修**，`#[ignore]`，根因假设见 §6 |
| 常驻 worker | **仍 `use_worker: false`**，未修复，只是被绕过 |
| 本机 CLI 存在性检查 | 未实现，受分层约束，见 §5 |
| 目标探测 TTL 缓存与并发合流 | 未实现，见 §6 |
| 用户环境下的最终确认 | **需要用户运行一次** |
