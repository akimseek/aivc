# 界面验证：启动应用、截图、可选地跑一段 DOM 探测。
#
# # 为什么需要它
#
# `cargo test`、类型检查、门禁都只能证明「类型对、逻辑对、能跑」。它们**看不见界面**。
# 而这个应用的问题恰恰有一类只在界面上才暴露：标题不可辨认、数字与实际不符、
# 排班错位、元素被遮挡。这类问题此前一直靠人肉发现。
#
# 本脚本把「看一眼」变成可重复的动作：应用以 WebView2 远程调试端口启动，用 CDP
# 离屏截图。**离屏**意味着不依赖窗口可见、不会被其它窗口遮挡。
#
# # 用法
#
#   pwsh -File scripts/ui-check.ps1                       # 只截启动画面
#   pwsh -File scripts/ui-check.ps1 -Probe "document.title"
#   pwsh -File scripts/ui-check.ps1 -Name 02-配置面板
#
# # 限制
#
# **原生对话框拍不到**：文件选择框、系统通知是操作系统的真窗口，不在 WebView 里。
# 涉及它们的路径（导出「另存到…」、技能导入）仍然只能手动验。
#
# 调试端口是**验证专用**，绝不能写进发布配置。

[CmdletBinding()]
param(
    # 截图文件名（不含扩展名），落在 <运行根>/shots/ 下。
    [string] $Name = "ui",
    # 可选：截图前在页面里执行的 JS。结果会打印出来。
    [string] $Probe = "",
    # 页面加载后等待的秒数。数据要经过几轮 IPC，等太短会拍到空白。
    [int] $Wait = 10
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root "target\debug\aivc-app.exe"

if (-not (Test-Path $exe)) {
    throw "找不到 $exe；先跑一次 cargo build -p aivc-app。"
}

# Tauri 在**编译时**把 frontendDist（ui/dist）嵌进二进制。因此只跑 vite build 是
# **看不到**前端改动的——验证的会是旧资源，而且一切看起来都「正常」。
# 这个检查是踩过一次之后加的：那次改了标题显示，进程、日志、探测都没报错，
# 只是显示的还是旧界面。
$dist = Join-Path $root "ui\dist\index.html"
if (Test-Path $dist) {
    $distTime = (Get-Item $dist).LastWriteTimeUtc
    $exeTime = (Get-Item $exe).LastWriteTimeUtc
    if ($distTime -gt $exeTime) {
        $hint = "ui/dist 比 aivc-app.exe 新，二进制里嵌的还是旧界面。先跑 cargo build -p aivc-app。"
        throw $hint
    }
}

$shots = Join-Path $root "shots"
New-Item -ItemType Directory -Path $shots -Force | Out-Null
$out = Join-Path $shots "$Name.png"

# 每次都以干净状态启动：残留的 data/ 会让「首次运行」的路径测不到。
foreach ($dir in @("logs", "data")) {
    Remove-Item (Join-Path $root $dir) -Recurse -Force -ErrorAction SilentlyContinue
}

$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=9222"
if ($Probe) { $env:CDP_PROBE = $Probe }  # 未传时保留继承值：见 cdp-shot.cjs 的用法说明

$process = Start-Process -FilePath $exe -WorkingDirectory $root -PassThru
try {
    Start-Sleep -Seconds $Wait
    & node (Join-Path $PSScriptRoot "cdp-shot.cjs") $out
    if ($LASTEXITCODE -ne 0) { throw "截图失败（exit $LASTEXITCODE）" }
}
finally {
    Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
    # 运行期数据属于这一次运行，不留在工作区里污染下一次。
    Start-Sleep -Seconds 1
    foreach ($dir in @("logs", "data")) {
        Remove-Item (Join-Path $root $dir) -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Write-Host "截图：$out"
