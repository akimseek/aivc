# 界面基线集：每个视图各截一张，落在 `shots/baseline/`。
#
# # 为什么需要它
#
# `shots/` 里那些是**过程图**——每一次修 bug 留下的证据，编号到 29。它们能回答
# 「当时为什么这么改」，但回答不了「这次改动有没有把界面弄坏」。
#
# 基线集相反：**每个视图固定一张**，同一组步骤、同一个初始状态。改动之后重跑一遍，
# 逐张 diff 就能看出界面回归——包括那些门禁和单元测试都看不见的（排版错位、元素被挡、
# 文案换了位置）。
#
# # 与 `ui-check.ps1` 的关系
#
# 那个脚本负责「启动一次、拍一张」，每次都从干净状态重来。这里复用它的启动方式，
# 但**只启动一次**，然后多次附着同一个调试端口——每一步都是一次新的 `cdp-shot`，
# 状态在步骤之间累积，这正是「浏览一遍各个视图」需要的行为。
#
# # 只差一步的检查不可省
#
# `cdp-shot.cjs` 在 CDP 连接断开时**会静默退出 0**（Node 事件循环空了就退出，
# `main()` 的 promise 永远不会 settle）。所以这里**必须检查 PNG 真的生成了**，
# 而不是看退出码——这个坑踩过一次，代价是以为截到了、其实什么都没有。

[CmdletBinding()]
param(
    # 基线目录名，落在 `shots/<Name>/` 下。
    [string] $Name = "baseline",
    # 应用启动后等待的秒数。太短会拍到空白。
    [int] $Wait = 12,
    # 每一步之后等待的毫秒数（给界面时间完成跳转）。
    [int] $StepWait = 2500
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root "target\debug\aivc-app.exe"
$dist = Join-Path $root "ui\dist\index.html"

if (-not (Test-Path $exe)) {
    throw "找不到 $exe；先跑一次 cargo build -p aivc-app。"
}
# Tauri 在**编译时**把前端嵌进二进制。只跑 vite build 会拍到旧界面。
if ((Test-Path $dist) -and ((Get-Item $dist).LastWriteTimeUtc -gt (Get-Item $exe).LastWriteTimeUtc)) {
    throw "ui/dist 比 aivc-app.exe 新，二进制里嵌的还是旧界面。先跑 cargo build -p aivc-app。"
}

$shots = Join-Path $root "shots\$Name"
New-Item -ItemType Directory -Path $shots -Force | Out-Null

# 每一步：名字 + 到那一步为止要执行的 JS。probe 的返回值会打印出来，方便核对
# 「页面当时到底处于什么状态」——只看图有时分不清「没这个元素」和「图拍早了」。
$steps = @(
    @{
        # **不叫「会话列表」**：应用**启动后什么都不选**（平台/目标/会话都空），
        # 所以这一张拍到的其实是「默认启动状态」而不是一张列表。
        # 名字要如实反映拍到了什么——叫错了会让后来看基线的人对着图找不存在的差异。
        #
        # 批次三：列表行从 `.rows .row` 换成了卡片（`features/sessions/session-card.tsx`），
        # 正文头部从 `.session-head` 换成了 `.history-head`（`features/workspace/history-view.tsx`）。
        Name  = "01-启动默认"
        Probe = 'JSON.stringify({cards:document.querySelectorAll(".sidebar .rows .session-card").length,detailOpen:!!document.querySelector(".history-head"),sideHint:(document.querySelector(".sidebar .rows")?.textContent??"").trim().slice(0,40)})'
    },
    @{
        # 批次三：主列表的行是 `.session-card`，单击它打开的是**历史正文**
        # （双击才是恢复终端）。这一步因此先按用户的真实路径选平台与目标，
        # 列表才有行可点——否则拍到的是空列表 + `NO-CARD`。
        Name  = "02-会话详情"
        Probe = '(async()=>{const set=(el,v)=>{Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,"value").set.call(el,v);el.dispatchEvent(new Event("change",{bubbles:true}));};const sels=()=>[...document.querySelectorAll(".sidebar .pickers select")];const p0=[...((sels()[0]||{}).options||[])].find(o=>o.value==="codex");if(p0)set(sels()[0],"codex");await new Promise(t=>setTimeout(t,2500));const s1=sels();const t=[...((s1[1]||{}).options||[])].find(o=>o.value!=="");if(t)set(s1[1],t.value);for(let i=0;i<24&&!document.querySelector(".sidebar .rows .session-card");i++)await new Promise(r=>setTimeout(r,500));const c=document.querySelector(".sidebar .rows .session-card");if(!c)return "NO-CARD";c.click();await new Promise(t=>setTimeout(t,' + $StepWait + '));return JSON.stringify({head:(document.querySelector(".history-head .meta")?.textContent??"").trim()||null,messages:document.querySelectorAll(".history-message").length});})()'
    },
    @{
        Name  = "03-终端"
        # 「+ Shell」整体移除后又跟着批次一换了入口：旧侧栏的「+ Codex」不存在了，
        # 现在唯一的新会话入口是**标签栏右侧动作区**的按钮
        # （`features/workspace/tab-bar.tsx`，`aria-label="新建会话"`），
        # 弹的是共享 `Dialog`（附加类名 `.new-session-dialog`，`app/new-session-dialog.tsx`）。
        # 验的东西没变：终端能不能起、xterm 有没有挂上。
        Probe = '(async()=>{const b=[...document.querySelectorAll(".tab-bar-action")].find(x=>(x.getAttribute("aria-label")||"")==="新建会话");if(!b)return "NO-BUTTON";b.click();await new Promise(t=>setTimeout(t,1200));const c=[...document.querySelectorAll(".new-session-dialog button")].find(x=>x.textContent.trim()==="打开");if(c)c.click();for(let i=0;i<40;i++){await new Promise(t=>setTimeout(t,250));if(document.querySelector(".xterm-helper-textarea"))break;}return JSON.stringify({term:!!document.querySelector(".xterm"),dialog:!!document.querySelector(".new-session-dialog")});})()'
    },
    @{
        Name  = "04-配置"
        # 入口从菜单栏的「配置」改成活动栏底部的设置按钮：批次二移除了菜单栏，
        # 设置中心现在只有这一个入口（`ui/src/shell/activity-bar.tsx`）。
        # 面板与页签的类名也跟着换了：`.overlay-panel` → `.settings-panel`、
        # `.overlay-tabs button` → `.settings-nav-item`。
        Probe = '(async()=>{const b=document.querySelector(".activity-bottom button");if(!b)return "NO-BUTTON";b.click();await new Promise(t=>setTimeout(t,' + $StepWait + '));return JSON.stringify({panel:!!document.querySelector(".settings-panel"),tabs:[...document.querySelectorAll(".settings-nav-item")].map(x=>x.textContent.trim())});})()'
    },
    @{
        Name  = "05-退出确认"
        # 退出确认框批次二换成共享 `Dialog`：外壳与页脚的类名是
        # `.dialog-panel` / `.dialog-footer button`，不再是 `.overlay-panel-confirm` / `.overlay-foot`。
        Probe = '(async()=>{const inv=(c,a)=>window.__TAURI__.core.invoke("aivc_invoke",{command:c,args:a??null});await inv("app:command",{command:"quit"});await new Promise(t=>setTimeout(t,' + $StepWait + '));return JSON.stringify({dialog:!!document.querySelector(".dialog-panel"),buttons:[...document.querySelectorAll(".dialog-footer button")].map(x=>x.textContent.trim())});})()'
    }
)

foreach ($dir in @("logs", "data")) {
    Remove-Item (Join-Path $root $dir) -Recurse -Force -ErrorAction SilentlyContinue
}

$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=9222"
$process = Start-Process -FilePath $exe -WorkingDirectory $root -PassThru
$failed = @()
try {
    Start-Sleep -Seconds $Wait
    foreach ($step in $steps) {
        $env:CDP_PROBE = $step.Probe
        $env:CDP_PROBE_WAIT = "1500"
        $out = Join-Path $shots "$($step.Name).png"
        Remove-Item $out -Force -ErrorAction SilentlyContinue

        & node (Join-Path $PSScriptRoot "cdp-shot.cjs") $out

        # **看文件，不看退出码**：见文件头的说明。
        if (-not (Test-Path $out)) {
            $failed += $step.Name
            Write-Host "[失败] $($step.Name)：没有生成 PNG"
        } else {
            Write-Host "[完成] $($step.Name)"
        }
        Start-Sleep -Milliseconds $StepWait
    }
}
finally {
    Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 1
    foreach ($dir in @("logs", "data")) {
        Remove-Item (Join-Path $root $dir) -Recurse -Force -ErrorAction SilentlyContinue
    }
}

if ($failed.Count -gt 0) {
    throw "这些视图没有截到：$($failed -join '、')"
}
Write-Host "基线集：$shots（$($steps.Count) 张）"
