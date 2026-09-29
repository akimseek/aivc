// 让这个脚本安静一点：`node:sqlite` 会往 stderr 打 ExperimentalWarning，
// 而 PowerShell 把 stderr 输出当成错误（exit code 1），会误导使用者以为脚本失败了。
process.removeAllListeners("warning");
process.on("warning", () => undefined);

/**
 * 往一个运行根的库**造一段真实形状的网关请求记录**。
 *
 * # 它解决什么问题
 *
 * 用量 / 异常 / 日志三个面板的渲染有**两条代码路径**：空状态与有数据。
 * 只跑过空状态就等于只验证了一半——表头、分组、金额与耗时的格式化、
 * 错误代码列全都没被执行过。
 *
 * 要让面板里真的有数据，只有两条路：
 *
 * 1. 让网关**真的服务一次请求**——那要配一家供应商，而且会花用户的钱；
 * 2. 往运行根的库里写几条**符合真实 schema** 的记录。
 *
 * 这里走第二条。它与「伪造验证结果」是两回事：写进去的是**数据**，
 * 而面板要从这份数据里算出什么、有没有算对，是应用自己算的。
 * 所以它能证明渲染路径，**证明不了「网关记账逻辑对不对」**——
 * 后者要看 Rust 侧的单测与真实请求。
 *
 * # 为什么要求库已经存在
 *
 * 表结构由应用自己的迁移建立。手写一份 `CREATE TABLE` 等于把 schema 抄了第二遍，
 * 抄错一处以后就再也对不上。因此这个脚本**拒绝在库不存在时自行建表**，
 * 而是让你先跑一次空探针让应用把库建出来。
 *
 * 用法：
 *   node scripts/seed-gateway-logs.cjs <运行根>
 */
const fs = require("fs");
const path = require("path");
const { DatabaseSync } = require("node:sqlite");

const runRoot = process.argv[2];
if (!runRoot) {
  console.error("用法：node scripts/seed-gateway-logs.cjs <运行根>");
  process.exit(1);
}

const dbPath = path.join(runRoot, "data", "aivc.db");
if (!fs.existsSync(dbPath)) {
  console.error(`库不存在：${dbPath}`);
  console.error(
    "先跑一次应用让它建库：.\\scripts\\app-probe.ps1 -Probe probes\\_boot-only.js -RunRoot <运行根>"
  );
  process.exit(1);
}

/** 时间戳格式必须与后端的 `Timestamp::format_unix_millis` 逐字一致（UTC，毫秒，`Z` 结尾）。 */
function stamp(offsetMs) {
  return new Date(Date.now() + offsetMs).toISOString().replace(/(\.\d{3})\d*Z$/, "$1Z");
}

/**
 * 四条记录，各自覆盖一种**面板上不同的渲染分支**：
 *
 * | 记录 | 覆盖什么 |
 * | --- | --- |
 * | 成功 | 成功数、token 分项与总数 |
 * | 供应商侧失败（429） | 异常面板、错误代码列、重试与切换数 |
 * | 超时 | 超时筛选、30 秒耗时不被截断 |
 * | `outcome=ok` 但有错误代码 | **最要紧的一条**：路由不存在/鉴权失败就是这个形状 |
 */
const rows = [
  {
    request_id: "seed-ok-1",
    route_id: "term-seed-1",
    provider_id: "codex",
    vendor_id: "seed-vendor-a",
    method: "POST",
    path: "/v1/responses",
    model: "gpt-5-codex",
    upstream_status: 200,
    outcome: "ok",
    duration_ms: 1840,
    retry_count: 0,
    switched: 0,
    error_code: null,
    error_message: null,
    usage_json: JSON.stringify({
      // **字段名必须是 camelCase。** `GatewayUsage` 带
      // `#[serde(rename_all = "camelCase")]`，写成 snake_case 不会报错——
      // `parse_stored_usage` 会解析成功、但每个字段都是 `None`，
      // 于是面板上 token 显示成「—」。静默出错，只有真的看一眼才发现。
      model: "gpt-5-codex",
      inputTokens: 1200,
      cachedInputTokens: 300,
      outputTokens: 480,
      totalTokens: 1680,
      costUsd: 0.0123
    }),
    created_at: stamp(-40 * 60 * 1000)
  },
  {
    request_id: "seed-fail-1",
    route_id: "term-seed-1",
    provider_id: "codex",
    vendor_id: "seed-vendor-a",
    method: "POST",
    path: "/v1/responses",
    model: "gpt-5-codex",
    upstream_status: 429,
    outcome: "error",
    duration_ms: 320,
    retry_count: 2,
    switched: 1,
    error_code: "rate_limited",
    error_message: "upstream returned 429",
    usage_json: null,
    created_at: stamp(-30 * 60 * 1000)
  },
  {
    request_id: "seed-timeout-1",
    route_id: "term-seed-2",
    provider_id: "codex",
    vendor_id: "seed-vendor-b",
    method: "POST",
    path: "/v1/responses",
    model: "gpt-5-codex",
    upstream_status: null,
    outcome: "timeout",
    duration_ms: 30000,
    retry_count: 0,
    switched: 0,
    error_code: "upstream_timeout",
    error_message: null,
    usage_json: null,
    created_at: stamp(-20 * 60 * 1000)
  },
  {
    request_id: "seed-noroute-1",
    route_id: "term-seed-3",
    provider_id: "claude",
    vendor_id: "(none)",
    method: "POST",
    path: "/v1/messages",
    model: null,
    upstream_status: null,
    outcome: "ok",
    duration_ms: 3,
    retry_count: 0,
    switched: 0,
    error_code: "route-missing",
    error_message: "没有可用的供应商",
    usage_json: null,
    created_at: stamp(-10 * 60 * 1000)
  }
];

const db = new DatabaseSync(dbPath);
const insert = db.prepare(
  `INSERT OR REPLACE INTO gateway_request_logs
   (request_id, route_id, provider_id, vendor_id, method, path, model, upstream_status,
    outcome, duration_ms, bytes_in, bytes_out, retry_count, switched, error_code,
    error_message, usage_json, created_at)
   VALUES (?,?,?,?,?,?,?,?,?,?,0,0,?,?,?,?,?,?)`
);
for (const row of rows) {
  insert.run(
    row.request_id,
    row.route_id,
    row.provider_id,
    row.vendor_id,
    row.method,
    row.path,
    row.model,
    row.upstream_status,
    row.outcome,
    row.duration_ms,
    row.retry_count,
    row.switched,
    row.error_code,
    row.error_message,
    row.usage_json,
    row.created_at
  );
}

/**
 * 同时把这两家供应商**登记进去**。
 *
 * # 为什么这一条不能省
 *
 * 不登记的话，用量报表里那三行的供应商名会是 `已删除供应商`（后端的
 * `DELETED_VENDOR_LABEL`）——那是个**正确的降级**，但它让「运行根里有真实数据」
 * 这件事不成立：真实运行根里供应商是存在的。第一版探针就是这么误报的：
 * 它断言表里能看到 `seed-vendor-a`，而那一列按设计只显示展示名。
 *
 * 登记之后表里显示的是「供应商甲 / 供应商乙」，与用户的真实处境一致。
 */
const upsertVendor = db.prepare(
  `INSERT OR REPLACE INTO api_vendors
   (id, provider_id, name, name_norm, api_key, api_base_url, sort, enabled, created_at, updated_at)
   VALUES (?,?,?,?,?,?,?,1,?,?)`
);
const now = stamp(0);
upsertVendor.run(
  "seed-vendor-a",
  "codex",
  "供应商甲",
  "供应商甲",
  "sk-seed-a",
  "https://a.example.com/v1",
  0,
  now,
  now
);
upsertVendor.run(
  "seed-vendor-b",
  "codex",
  "供应商乙",
  "供应商乙",
  "sk-seed-b",
  "https://b.example.com/v1",
  1,
  now,
  now
);

const total = db.prepare("SELECT COUNT(*) AS c FROM gateway_request_logs").get().c;
db.close();
console.log(`已写入 ${rows.length} 条请求记录与 2 家供应商，库内共 ${total} 条记录。`);
console.log(`库：${dbPath}`);
