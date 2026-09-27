<#
.SYNOPSIS
  启动应用（带 CDP）、跑一段页面探针、打印结果与日志，然后收尾。

.DESCRIPTION
  把「验证真实运行时行为」这件事收敛成一条命令。它做四件手工时容易漏掉的事：

  1. **每次都以干净状态启动**：残留的 logs / data 会让「首次运行」的路径测不到；
  2. **等 CDP 真的可用**再跑探针（固定 sleep 要么太短要么白等）；
  3. **总是收尾**：进程、日志、数据目录，无论探针成功还是抛异常；
  4. **顺手打印日志里命中的行**——很多结论要靠后端日志才能下定论。

  探针文件由 `scripts/probe-helpers.js`（定义 `window.__p`）与探针本身拼成。
  因此探针必须是**最后一个表达式**，它的值就是打印出来的结果。

.EXAMPLE
  .\scripts\app-probe.ps1 -Probe probes\gateway-e2e.js

.EXAMPLE
  # 保留现场（日志、数据目录、stderr）以便排查
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
  # 跑完保留 logs / data / stderr，便于排查。
  [switch] $Keep,
  # 注入到页面的探针参数，作为 `window.__pArgs`。
  [string] $ProbeArgs = ""
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

# 每次都以干净状态启动。
foreach ($dir in @("logs", "data")) {
  Remove-Item (Join-Path $root $dir) -Recurse -Force -ErrorAction SilentlyContinue
}
$errFile = Join-Path $root "shots\probe-stderr.txt"
Remove-Item $errFile -Force -ErrorAction SilentlyContinue

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

$process = Start-Process -FilePath $exe -WorkingDirectory $root -PassThru `
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
    Write-Host "`n（-Keep：保留了 logs\、data\ 与 shots\probe-stderr.txt）"
  } else {
    foreach ($dir in @("logs", "data")) {
      Remove-Item (Join-Path $root $dir) -Recurse -Force -ErrorAction SilentlyContinue
    }
    Remove-Item $errFile -Force -ErrorAction SilentlyContinue
  }
}

if ($failed) { throw "探针失败（cdp-shot.cjs 非零退出）" }
