# 本地质量门禁：格式、编译、Lint、测试、依赖分层。
#
# 用法：
#   .\scripts\verify.ps1              # 全部检查
#   .\scripts\verify.ps1 -Quick       # 跳过测试（快速反馈）
#
# 与 CI 保持一致；任何一步失败即以非零码退出。

[CmdletBinding()]
param(
    [switch]$Quick
)

$ErrorActionPreference = 'Continue'

$scriptDir = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
$repoRoot = Split-Path -Parent $scriptDir
Set-Location $repoRoot

# 定位 cargo：优先使用工作区内隔离的工具链，否则回退全局安装。
$toolchainRoot = Join-Path $repoRoot '.toolchain'
$cargoExe = Join-Path $toolchainRoot 'cargo\bin\cargo.exe'
if (Test-Path $cargoExe) {
    $env:CARGO_HOME = Join-Path $toolchainRoot 'cargo'
    $env:RUSTUP_HOME = Join-Path $toolchainRoot 'rustup'
    $env:RUSTUP_TOOLCHAIN = 'stable-x86_64-pc-windows-msvc'
    $env:PATH = "$(Join-Path $toolchainRoot 'cargo\bin');$env:PATH"
}
else {
    $cargoExe = 'cargo'
}

$failures = @()

# 所有 .ps1 必须带 UTF-8 BOM。
#
# Windows PowerShell 的解析器与 `Get-Content` 会按 ANSI 读没有 BOM 的文件，于是中文注释
# 变成乱码，而且**常常在字符串终止符上炸开**，报的却是一句看不懂的语法错误。
# 这一条踩过两次：一次是新写的脚本，一次是**用会去掉 BOM 的编辑器改过之后**。
# 放进来自动查，比靠记性可靠。
$missingBom = @()
foreach ($file in Get-ChildItem (Join-Path $PSScriptRoot "*.ps1")) {
    $bytes = [System.IO.File]::ReadAllBytes($file.FullName)
    $hasBom = $bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF
    if (-not $hasBom) { $missingBom += $file.Name }
}
if ($missingBom.Count -gt 0) {
    Write-Host "=== 脚本编码 (UTF-8 BOM) ===" -ForegroundColor Cyan
    Write-Host "缺少 BOM：$($missingBom -join ', ')" -ForegroundColor Red
    $failures += 'bom'
}

# 探针选择器守卫：脚本（探针 / 基线）里引用的类名必须真的存在于 `ui/src`。
#
# 与上面那条同一个思路：这类腐坏**不会自己报错**——探针只会静默地查不到元素，
# 打印一个 `NO-ROW` 或空对象，截图依旧是一张 PNG，看的人以为界面里就是没有。
# 规则、排除项与白名单的理由都写在 `scripts/check-probe-selectors.cjs` 的文件头。
Write-Host "=== 探针选择器 (check-probe-selectors) ===" -ForegroundColor Cyan
node (Join-Path $PSScriptRoot "check-probe-selectors.cjs")
if ($LASTEXITCODE -ne 0) {
    Write-Host "失败（exit $LASTEXITCODE）：脚本引用了 ui/src 里不存在的类名" -ForegroundColor Red
    $failures += 'probe-selectors'
}
else {
    Write-Host "通过" -ForegroundColor Green
}

# 命令名守卫：前端 `invoke` 出的「域:动作」必须登记在后端 `COMMANDS` 表里。
#
# 与上一条同一个病：命令名写错不会自己报错——两边都编译得过，点下去才收到「未知命令」
# （B1 就是这么炸的：前端写 `codex:open-session-folder`，后端注册的却是 `shell:`）。
# 先跑 `--self-test` 证明「改坏一个名字真的会红」，再跑真实比对（失败级：前端有后端无；
# 后端多出来的名字只提示）。规则与排除项写在 `scripts/check-command-names.cjs` 的文件头。
Write-Host "=== IPC 命令名 (check-command-names) ===" -ForegroundColor Cyan
$commandNamesOk = $true
node (Join-Path $PSScriptRoot "check-command-names.cjs") --self-test
if ($LASTEXITCODE -ne 0) {
    Write-Host "失败（exit $LASTEXITCODE）：守卫自检未通过（注入的坏名字没有被报出来）" -ForegroundColor Red
    $commandNamesOk = $false
}
node (Join-Path $PSScriptRoot "check-command-names.cjs")
if ($LASTEXITCODE -ne 0) {
    Write-Host "失败（exit $LASTEXITCODE）：前端 invoke 了后端 COMMANDS 表里没有的命令名" -ForegroundColor Red
    $commandNamesOk = $false
}
if ($commandNamesOk) {
    Write-Host "通过" -ForegroundColor Green
}
else {
    $failures += 'command-names'
}

function Invoke-Step {
    param(
        [string]$Name,
        [string[]]$Arguments
    )

    Write-Host ""
    Write-Host "=== $Name ===" -ForegroundColor Cyan
    # 注意：clippy 需要 `--` 分隔符。PowerShell 会在字面量命令行中吞掉 `--`，
    # 因此这里必须用数组 splatting 传参，不能用字面量拼接。
    $output = & $cargoExe @Arguments 2>&1 | Out-String
    $exitCode = $LASTEXITCODE

    if ($exitCode -eq 0) {
        Write-Host "通过" -ForegroundColor Green
        return $true
    }

    Write-Host "失败（exit $exitCode）" -ForegroundColor Red
    # 回显诊断行，避免刷屏；**但不能只认 `error`/`warning`**：
    # Rust 测试失败的详情以 `failures:` / `---- <用例名> stdout ----` 开头，
    # 而**断言消息**在 `panicked at` 那一行。
    #
    # 只认前两者会把「哪个用例挂了、断言说了什么」整段吞掉——排查时只看到一个光秃秃的
    # `exit 101`，等于逼人再跑一遍；而偶发失败未必跑得出来。
    ($output -split "`n" |
        Select-String -Pattern '^(error|warning|failures:|---- |test result: FAILED)|panicked at' -Context 0, 4) |
        Select-Object -First 60 |
        Out-String -Width 200 |
        Write-Host
    return $false
}

if (-not (Invoke-Step -Name '格式检查 (cargo fmt --check)' -Arguments @('fmt', '--all', '--check'))) {
    $failures += 'fmt'
}

if (-not (Invoke-Step -Name '依赖分层校验 (xtask check-layers)' -Arguments @('run', '--quiet', '--offline', '--package', 'xtask', '--', 'check-layers'))) {
    $failures += 'check-layers'
}

if (-not (Invoke-Step -Name '编译检查 (cargo check)' -Arguments @('check', '--workspace', '--all-targets', '--offline'))) {
    $failures += 'check'
}

# 带全部可选特性再检查一遍：默认关闭的特性最容易腐坏，因为主流程不会碰它们。
if (-not (Invoke-Step -Name '编译检查（全部特性）' -Arguments @('check', '--workspace', '--all-targets', '--all-features', '--offline'))) {
    $failures += 'check-all-features'
}

if (-not (Invoke-Step -Name 'Lint (cargo clippy -D warnings)' -Arguments @('clippy', '--workspace', '--all-targets', '--offline', '--', '-D', 'warnings'))) {
    $failures += 'clippy'
}

if (-not (Invoke-Step -Name 'Lint（全部特性）' -Arguments @('clippy', '--workspace', '--all-targets', '--all-features', '--offline', '--', '-D', 'warnings'))) {
    $failures += 'clippy-all-features'
}

if (-not $Quick) {
    if (-not (Invoke-Step -Name '测试 (cargo test)' -Arguments @('test', '--workspace', '--offline'))) {
        $failures += 'test'
    }
}

Write-Host ""
if ($failures.Count -eq 0) {
    Write-Host "全部检查通过。" -ForegroundColor Green
    exit 0
}

Write-Host "以下检查未通过：$($failures -join ', ')" -ForegroundColor Red
exit 1
