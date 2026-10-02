# 20 · `message_count` 语义迁移方案

> 承接 [19](19-ui-verification-and-two-bugs.md) 的第 ② 个问题。本文只做**方案与站点清单**，
> 不含代码改动 —— 原因见文末「为什么这一轮没有直接改」。

---

## 1. 问题

列表模式下 `message_count` 是**头部窗口内的条数**，界面却当总数显示。实测：

```
界面显示「2 条」    实际渲染 80 条消息，且 hasMore = true
```

界面写「2 条」，用户理解成「这个会话有 2 条消息」—— 这是**错误的陈述**，不是格式问题。

### 现状（已做的缓解）

`ui/src/app/app.tsx` 里列表行与会话头的条数显示**已移除**。这是「不再说错话」，
不是「把数字修对」——代价是列表里没有条数信息了。

## 2. 项目里已有的约定（重要）

分页那条路径**早就想清楚了同一件事**，只是列表路径没跟上：

```rust
// crates/aivc-provider/src/paging.rs:228
/// `session_message_count` 来自会话摘要；`has_more` 时不得把已读条数当成总数

// crates/aivc-provider/src/paging.rs:659
assert_eq!(outcome.message_count, 0, "has_more 时不得声称已知总数");
```

**因此正确修法不是发明新概念，而是让列表路径遵守分页已确立的规则。**

## 3. 设计选择

| 方案 | 优点 | 缺点 |
| --- | --- | --- |
| **A. `message_count: Option<u64>`** | 类型层面就没有「假装知道」的可能；能区分「空会话」与「未知」 | 约 20 个构造点全部要改；DB 要允许 NULL |
| **B. 加 `message_count_is_total: bool`** | 构造点只多一行 | 两个字段可以互相矛盾，靠约定维持 |
| **C. 用 `0` 表示未知**（沿用 paging 的写法） | **零类型改动**；已是项目既有约定 | **无法区分真实空会话**，两者都显示「—」 |

## 2.1 实施结果（**已按 C 落地**）

最初我把 C 评为「不可接受」，理由是「空会话也是 0」。**那个判断是错的** ——
我把它当成「新发明的约定」来评估了，而实际上 `paging.rs` 早就在用 `0` 表示
「不得声称已知总数」。既然是既有约定，复用它就比另起一套安全。

实际改动**只有两处**（不是原估的 107 处）：

| 位置 | 改动 |
| --- | --- |
| `crates/aivc-provider/src/codex/listing.rs` | 列表模式把 `message_count` 置 `0` |
| `ui/src/app/app.tsx` | 新增 `messageCountLabel`：`0` → 「—」 |

保留的取舍：**真实空会话也显示「—」**。判断是这可以接受 ——
把空会话显示成「—」不是误导，而把 80 条显示成「2 条」是。

要彻底区分空与未知，仍需执行下面的 A 方案。

## 4. 站点清单

改动顺序按「编译器能一次报全」来排：**先改类型，让编译器把站点全部炸出来**，再逐个消掉。

### 第一步：类型

| 文件 | 位置 |
| --- | --- |
| `crates/aivc-core/src/session.rs` | `Session.message_count: u64` → `Option<u64>`（加 `skip_serializing_if`） |
| `crates/aivc-core/src/session.rs` | `SessionMetadata.message_count: u64`（第 547 行附近）需一并确认 |
| `crates/aivc-ipc/src/paging.rs` | `message_count: u64` 与 `anchor_message_count` |
| `crates/aivc-infra/src/db/repos/message_index.rs` | `message_count: u64` |

### 第二步：4 个 parser（各自 5–8 处）

| 文件 | 要点 |
| --- | --- |
| `codex/parser.rs:82,111,189,257` | List 模式给 `None`，Full 模式给 `Some(n)` |
| `qoder/parser.rs:266,286,349,363,394` | 同上 |
| `gemini/parser.rs:354,375,466,576` | 同上 |
| `claude/parser.rs:308,328,418,488` | 同上 |

**四处必须一起改完**，否则会出现「codex 有条数、gemini 没有」的不一致。

### 第三步：provider 与派生逻辑

| 文件 | 位置 |
| --- | --- |
| `codex/provider.rs:317,324` | `folder.finish(session.message_count)` |
| `qoder/provider.rs:358,364` | 同上 |
| `gemini/provider.rs:367,373` | 同上 |
| `claude/provider.rs:356,362` | 同上 |
| `export.rs:28,39,487,492` | 导出时把计数改成过滤后的条数（**导出是全量的，应当是 `Some`**） |
| `codex/branch.rs:376`、`common.rs:357`、`metadata.rs:310`、`text.rs:349` | 构造点 |

### 第四步：缓存与数据库

- `session_cache.rs` / `session_metadata.rs`：缓存条目读写
- `migrations.rs:214`：`message_count INTEGER NOT NULL` → 需允许 NULL（或明确用 `-1`）
- `message_index.rs:81,107,135,142,158,184,205`：索引表的读写

### 第五步：测试

- 4 个 parser 里的 `assert_eq!(session.message_count, N)` → `Some(N)`（约 15 处）
- **`codex/parser.rs:649` 的 `assert_eq!(session.message_count, 20, "计数必须是全量")` 要改**：
  它喂的是完整内容，所以仍应是 `Some(20)`；但那个测试名会让人误以为 List 模式也有全量计数，
  **注释要写清楚「这里喂的是全量内容，不代表列表路径」**
- `parser_parity.rs:68,122,214,271`：List 与 Full 的对比断言要按新语义重写

### 第六步：前端

| 文件 | 改动 |
| --- | --- |
| `ui/src/types.ts` | `messageCount: number` → `messageCount?: number` |
| `ui/src/app/app.tsx` | 有值才显示；`undefined` 时显示「—」而不是隐藏 |

## 5. 验证

1. `scripts/verify.ps1` 七项全绿
2. `cargo build -p aivc-app`（**前端改动必须重建**，否则验证的是旧资源）
3. `scripts/ui-check.ps1 -Probe '...'` 确认列表里条数显示为「—」而不是「2 条」
4. 打开一个长会话，确认详情页的条数是**全量**（这条路径本来就是 Full 模式）

## 6. 为什么这一轮没有直接改

`Session` 加/改字段会让**所有构造点同时编译失败**，这不是可以分步提交的改动：

- 约 **20 个结构体字面量**要一次性补齐
- **4 个 parser 必须一起改完**，否则语义不一致
- 还要动数据库 schema

需要一次连续的、跑得完门禁与截图验证的执行窗口。在余量不足时硬上，留下的会是
「编译不过」或「4 个 provider 行为不一致」的中间态 —— 比不改更糟。

**本文的作用**：把下一步从「探索」变成「机械执行」。上表每一行都是具体位置，
照着做不需要再分析。

---

## 7. 复核（第二轮）：实测规模、真实收益，以及一个更好的方案

在动手之前重新量了一遍。**结论变了**：不建议执行 A 方案，理由如下。

### 7.1 实测规模：真实构造点约 17 个，不是 113

`grep message_count` 共 **113 处**，但构成是：

| 类别 | 数量 | 是否要改 |
| --- | --- | --- |
| **`Session` 结构体字面量** | 约 17（4 个 parser × 2、`export.rs:39`、约 8 处测试） | 要 |
| `assert_eq!(session.message_count, N)` | 约 25 | 要（→ `Some(N)`） |
| **`MessageIndexEntry.message_count`**（`infra/message_index.rs`） | 约 12 | **不要**——另一个结构体，语义是「索引到的条数」，永远已知 |
| `paging.rs` 内部（`PageOutcome` / `PageFolder`） | 约 15 | 视 `finish` 签名而定，牵连面待定 |
| 其余（读点、`keep_message_count` 形参等） | 约 44 | 不要 |

所以不是 107 处，而是 **约 50 处 + 数据库 schema + 若干轮编译-测试循环**。

### 7.2 真实收益只有一个边角

`listing.rs:209-211` 是**无条件**置 `0` 的：

```rust
if lightweight {
    entry.session.message_count = 0;
}
```

也就是说列表模式下**每一条**都是 `0` → 界面全部显示「—」。既然已经不再有「把 80 条
显示成 2 条」这种**错误陈述**，那么 A 方案唯一能买到的就是：

> 一个**真实空会话**在列表里显示「0 条」，而不是「—」。

而第 2.1 节自己已经判定这个取舍可以接受。**用约 50 处改动 + schema 迁移换这一个
边角，性价比是负的。**

### 7.3 最终采用的方案：A′（「下界」进数据）——**已落地**

第 7.2 节漏掉了一件事：列表模式数出来的条数是**真实的下界**——它读了头部窗口，
数到多少就是多少。**「≥2 条」是一句真话**，而「2 条」是假话，「—」是放弃表达。

我先把「≥」做在渲染层（3 处改动），**被 `end_to_end.rs` 挡了回来**：

```
assertion `left == right` failed: 列表路径不该声称已知总数
```

**它挡得对。** 那样一来线上传的是一个**光看数字分不出是下界还是总数**的值，而「它是
下界」只写在界面的一行注释里。本项目里每一个「跨层约定只存在于人的记忆里」的地方都
出过问题（参见 `router.rs` 里 `HANDLED_BY_PLATFORM` 的说明）。

**所以「下界」先进数据，再进渲染层。** 实际改动：

| 位置 | 改动 |
| --- | --- |
| `core/session.rs` | 新增 `message_count_is_lower_bound: bool`（`#[serde(default, skip_serializing_if = "is_false")]`） |
| 4 个 parser 的 `finish()` | 一律 `false`——**parser 不知道自己拿到的是不是整个文件**，那由调用方决定 |
| `codex/listing.rs` | 保留数出来的条数，并置 `true`（**唯一**读了头部窗口的地方） |
| `codex/export.rs` | 显式清成 `false`——导出文件列出的是全部内容，不能靠 `..session` 继承 |
| `ui/types.ts` + `session-display.ts` | 新增可选字段；`messageCountLabel(count, lowerBound)` 输出 `≥N 条` |
| `end_to_end.rs` | 断言改成**更强**的契约：`messageCount == 2` **且** `messageCountIsLowerBound == true` |

**关键设计判断**：置位归**读了头部窗口的调用方**，不归 parser。三个 parser
（qoder / gemini / claude）的「列表模式」表达为 `max_messages`——那是**预览上限**，
不是「只读了文件头」。只有调用方知道 `read_head` 读了 64 KB。

**为什么比原计划便宜**：不需要动数据库 schema，不需要改 4 个 parser 的解析逻辑，
`Session` 的构造点只有 14 处。

**为什么比原方案强**：那条端到端断言从「**不**声称知道总数」（`== 0`，把信息丢掉，
而且 `0` 本身也是个数——把它当总数的消费方会说「0 条」这句假话）变成了「**明确声明**
这是下界」。

### 7.4 两件前置事实（都已查清 ✅）

**其一：列表路径不经过 `paging.rs`。** `listing.rs` 只 import 了 `parse_content` 与
`SessionCacheEntry`，**完全没有** `paging`（grep 已确认）。`folder.finish(...)` 在
`{codex,qoder,gemini,claude}/provider.rs` 里，位于 **`read_page`（分页读取）** 路径上。

**其二：那个下界不会经缓存漏进详情页。** 详情走
`get_session_summary` → `find_file` → **`load_full`**，而 `load_full` 是
`store.read_text(...)` + `parse_content(..., ParseMode::Full)`——**全量解析，不查缓存**
（`codex/provider.rs:334,549,567`）。所以详情页拿到的永远是精确总数。

**顺带发现，现已修掉**：会话头（`app.tsx`）读的是 `console.selectedSession`，而它是
`sessions.find(...)`（`use-console.ts:671`）——**来自列表数据**，所以那里拿到的永远是
下界。

修法：让**总数随每一页分页响应回来**。

| 位置 | 改动 |
| --- | --- |
| `core/session.rs` | `MessagePage` 新增 `message_count`（`#[serde(default)]`，`0` = 本次无从得知） |
| `provider/paging.rs` | `PageOutcome::finish` 里算出的那个值**同时放进页里**，两处同源 |
| `ui/types.ts` / `use-console.ts` | 记下 `messageTotal`（`0` → `null`） |
| `ui/app.tsx` | 优先用精确总数，未到时回退到列表条数**并如实标注是下界** |

**实测**：同一个会话，列表说「至少 2 条」，会话头现在显示 **`2091 条`**。
那个数字本身就是「下界」这套设计存在的理由——列表路径**永远**不可能知道它。

### 7.5 收尾时抓到的一处**自己引入的**不一致

置位做在 `codex/listing.rs` 之后，**另外三个 provider 的列表路径忘了置**——它们的
列表模式会保留数出来的条数、却把标志留在 `false`，也就是**把下界当成精确值报出去**。

这正是原计划里那句「**四处必须一起改完**，否则会出现『codex 有条数、gemini 没有』的
不一致」警告的形状。区别是：这次它在**提交前**被自己的清单抓到了，而不是被用户发现。

四处置位点：

| provider | 位置 |
| --- | --- |
| codex | `codex/listing.rs` |
| qoder | `qoder/provider.rs`（`read_head` 之后） |
| gemini | `gemini/provider.rs`（同上） |
| claude | `claude/provider.rs`（同上） |

**每个 provider 的列表测试里都补了一条断言**：

```rust
assert!(sessions[0].message_count_is_lower_bound, "列表路径必须标注条数是下界");
```

**假绿检查**：临时去掉 qoder 的置位 → 那条断言如期报红。

> 为什么必须是四条而不是一条：漏掉一个 provider 时，其余三个是对的，界面上看起来
> 「大多数会话都对」，最难查。

### 7.6 为什么 `parser_parity.rs` 不是这条约定的守卫点

它直接驱动 **parser**，而 parser 一律返回 `false`（它不知道自己拿到的是不是整个文件，
见 7.3）。这条约定属于**列表路径**，所以守卫必须落在各 provider 的 `list_sessions` 上。

### 7.7 与 A 方案的关系

A 方案并没有错，它只是**贵**。如果将来出现「必须精确区分空会话与未知」的需求
（例如按条数排序、按条数筛选），那时再执行 A，而且那时会有明确的需求支撑它。

