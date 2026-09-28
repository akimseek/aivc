<#
.SYNOPSIS
  启动应用（带 CDP）、跑一段页面探针、打印结果与日志，然后收尾。

.DESCRIPTION
  把「验证真实运行时行为」这件事收敛成一条命令。它做四件手工时容易漏掉的事：

  1. **在隔离的运行根里启动**：应用在开发构建下把 `data/` 与 `logs/` 放在**当前工作目录**
     下（`resolve_runtime_storage_root`），因此探针把工作目录指到一个临时目录，
     于是**永远碰不到用户的真实数据**；
  2. **等 CDP 真的可用**再跑探针（固定 sleep 要么太短要么白等）；
  3. **总是收尾**：进程、临时目录，无论探针成功还是抛异常；
  4. **顺手打印日志里命中的行**——很多结论要靠后端日志才能下定论。

  探针文件由 `scripts/probe-helpers.js`（定义 `window.__p`）与探针本身拼成。
  因此探针必须是**最后一个表达式**，它的值就是打印出来的结果。

  # 为什么改成临时运行根（这是一次事故换来的）

  原先这个脚本每次都 `Remove-Item logs, data`。它的本意是「保证从干净状态启动」，
  但**应用的数据目录就是工作目录下的那两份**——也就是说，只要探针的工作目录
  恰好与用户启动应用时相同，它删的就是**用户自己的供应商配置与历史**。

  实测中用户就是因此在终端里看到 `INSUFFICIENT_BALANCE`：探针删掉了供应商，
  路由无法建立，CLI 于是直连了他自己配置里的第三方地址。

  现在的做法把两件事分开：**干净状态**由「每次一个全新的临时目录」保证，
  而**用户的数据**根本不在这条路径上，因此不需要（也不允许）删除任何东西。

.EXAMPLE
  .\scripts\app-probe.ps1 -Probe probes\gateway-e2e.js

.EXAMPLE
  # 保留临时运行根（日志、数据、stderr）以便排查
  .\scripts\app-probe.ps1 -Probe probes\gateway-e2e.js -Keep

.EXAMPLE
  .\scripts\app-probe.ps1 -Probe probes\terminal-ui.js -Shot shots\my-check.png
#>
param(
  # 探针文件（相对 scripts\probes\ 或任意路径）。
  [Parameter(Mandatory = $true)][string] $Probe,
  # 截图落点；留空则用 shots\probe-<探针名>.png。
  [string] $Shot = "",
  # 只打印日志里匹配这个正则的行（不区分大小写）。
  [string] $Log = "",
  # 最多等多少秒让 CDP 就绪。
  [int] $Ready = 40,
  # 跑完保留临时运行根（日志 / 数据 / stderr），并把路径打出来。
  [switch] $Keep,
  # 注入到页面的探针参数，作为 `window.__pArgs`。
  [string] $ProbeArgs = "",
  # 用**已存在的**运行根（例如应用真实数据目录 `target\debug`），而不是新建临时目录。
  #
  # 用途只有一个：**排查用户实际遇到的现象**——那种情况必须先看到用户自己的数据。
  # 给了这个参数时脚本**不会删除任何东西**，而且会先把运行根打印出来让人确认。
  [string] $RunRoot = ""
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root "target\debug\aivc-app.exe"
if (-not (Test-Path $exe)) {
  throw "找不到 $exe；先跑 .\scripts\cargo.ps1 build -p aivc-app"
}

# Tauri 在**编译时**把 ui/dist 嵌进二进制。只跑 vite build 是看不到前端改动的——
# 验证的会是旧资源，而且一切看起来都「正常」。这个守卫是踩过一次之后加的。
$dist = Join-Path $root "ui\dist\index.html"
if ((Test-Path $dist) -and ((Get-Item $dist).LastWriteTimeUtc -gt (Get-Item $exe).LastWriteTimeUtc)) {
  throw "ui/dist 比 aivc-app.exe 新，二进制里嵌的还是旧界面。先跑 .\scripts\cargo.ps1 build -p aivc-app"
}

# 探针可以写成 `probes\x.js`、`x.js` 或绝对路径；按顺序找，报错时把找过的都列出来。
$candidates = @(
  $Probe,
  (Join-Path $PSScriptRoot $Probe),
  (Join-Path (Join-Path $PSScriptRoot "probes") $Probe)
)
$probePath = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $probePath) { throw "找不到探针：$Probe`n找过：$($candidates -join '; ')" }

$helpers = Join-Path $PSScriptRoot "probe-helpers.js"
if (-not (Test-Path $helpers)) { throw "找不到 $helpers" }

if (-not $Shot) {
  $name = [System.IO.Path]::GetFileNameWithoutExtension($probePath)
  $Shot = Join-Path $root "shots\probe-$name.png"
}
New-Item -ItemType Directory -Path (Split-Path -Parent $Shot) -Force | Out-Null

# **两种运行根，两条规则，绝不混用。**
#
# - 不给 `-RunRoot`（默认）：新建临时目录，**保证干净**，且绝不碰工程目录；
# - 给了 `-RunRoot`：**只读地**用那个目录，用来排查「用户实际遇到的」现象
#   （那种情况必须先看到用户自己的数据）。这条路径**不删任何东西**。
if ($RunRoot) {
  if (-not (Test-Path $RunRoot)) { throw "运行根不存在：$RunRoot" }
  $runRoot = (Resolve-Path $RunRoot).Path
  $ephemeral = $false
  Write-Host "**注意：使用已有运行根，不会删除任何内容**：$runRoot"
} else {
  $runRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("aivc-probe-" + [System.Guid]::NewGuid().ToString("N").Substring(0, 8))
  New-Item -ItemType Directory -Path $runRoot -Force | Out-Null
  $ephemeral = $true
}

# **守卫：临时运行根绝不许落在工程目录里。**
#
# 这一条是把事故钉死的地方。原先的工作目录就是工程根，而开发构建把
# `data/`（供应商、会话、设置）与 `logs/` 都写在工作目录下——于是任何
# 「先删掉 data 与 logs 再从干净状态启动」的写法，删的都是**工程目录里的那份**，
# 而它可能正是用户在用的数据。实测中用户就是这么在终端里撞上第三方
# `INSUFFICIENT_BALANCE` 的。
#
# 现在默认运行根是临时目录，这条断言保证**将来有人改回工程目录时立刻失败**，
# 而不是又一次悄悄删掉用户的数据。显式 `-RunRoot` 是唯一的例外，且它不删东西。
if ($ephemeral) {
  $projectRoot = (Resolve-Path $root).Path
  $resolvedRunRoot = (Resolve-Path $runRoot).Path
  if ($resolvedRunRoot.StartsWith($projectRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "临时运行根不能落在工程目录内（会把工程目录当成应用数据目录）：$resolvedRunRoot"
  }
}

$errFile = Join-Path $runRoot "stderr.txt"

# **必须显式指定 UTF-8**：Windows PowerShell 的 `Get-Content -Raw` 默认按 ANSI 读，
# 而这两个文件里有中文（注释与字符串字面量）。读错会把源码变成乱码，
# 表现为探针「抛异常」而看不出原因。
$expression = (Get-Content $helpers -Raw -Encoding utf8) + "`n"
if ($ProbeArgs) { $expression += "window.__pArgs = $ProbeArgs;`n" }
$expression += (Get-Content $probePath -Raw -Encoding utf8)
$env:CDP_PROBE = $expression
# 页面要加载数据，给一轮渲染时间；太短会读到空白。
$env:CDP_PROBE_WAIT = "2500"
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=9222"

Write-Host "运行根：$runRoot"

$process = Start-Process -FilePath $exe -WorkingDirectory $runRoot -PassThru `
  -RedirectStandardError $errFile
$failed = $false
try {
  # 等 CDP 端点真的应答，而不是猜一个固定的等待时间。
  $deadline = (Get-Date).AddSeconds($Ready)
  $ready = $false
  while ((Get-Date) -lt $deadline) {
    try {
      $null = Invoke-WebRequest -Uri "http://127.0.0.1:9222/json/version" `
        -TimeoutSec 2 -UseBasicParsing
      $ready = $true
      break
    } catch {
      Start-Sleep -Milliseconds 400
    }
  }
  if (-not $ready) { throw "等了 $Ready 秒，CDP 端点仍未就绪" }

  & node (Join-Path $PSScriptRoot "cdp-shot.cjs") $Shot
  if ($LASTEXITCODE -ne 0) { $failed = $true }
} finally {
  if (-not $process.HasExited) { $process.Kill() }
  Start-Sleep -Seconds 1

  if ($Log -and (Test-Path $errFile)) {
    Write-Host "`n--- 日志中匹配 '$Log' 的行 ---"
    Get-Content $errFile -Encoding utf8 |
      Select-String -Pattern $Log |
      ForEach-Object { Write-Host $_.Line }
  }

  if ($Keep) {
    Write-Host "`n（-Keep：保留了运行根 $runRoot）"
  } elseif ($ephemeral) {
    # **只删自己刚建的那个临时目录。**
    # 用 `-RunRoot` 指定时（排查用户实际现象）绝不删任何东西。
    Remove-Item $runRoot -Recurse -Force -ErrorAction SilentlyContinue
  } else {
    Write-Host "`n（使用了已有运行根，未删除任何内容）"
  }
}

if ($failed) { throw "探针失败（cdp-shot.cjs 非零退出）" }
