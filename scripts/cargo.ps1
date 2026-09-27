# 使用工作区内隔离的 Rust 工具链运行 cargo。
#
# 本机未安装全局 Rust；工具链位于 aivc-rs/.toolchain/，通过重定向 CARGO_HOME /
# RUSTUP_HOME 使用，不修改 PATH 与用户 profile。删除 .toolchain/ 即可完全回退。
#
# 用法：
#   .\scripts\cargo.ps1 build --workspace
#   .\scripts\cargo.ps1 test --workspace
#   .\scripts\cargo.ps1 clippy --all-targets -- -D warnings
#
# 若已自行安装全局 rustup，可直接用 `cargo`，行为一致。
#
# 注意：刻意不设置 $ErrorActionPreference = 'Stop'。cargo 会把进度信息写到
# stderr，而 Windows PowerShell 5.1 会把原生命令的 stderr 当作 NativeCommandError
# 抛错并中断；此处以 $LASTEXITCODE 为准。

$scriptDir = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
$repoRoot = Split-Path -Parent $scriptDir
$toolchainRoot = Join-Path $repoRoot '.toolchain'
$cargoExe = Join-Path $toolchainRoot 'cargo\bin\cargo.exe'

if (Test-Path $cargoExe) {
    $env:CARGO_HOME = Join-Path $toolchainRoot 'cargo'
    $env:RUSTUP_HOME = Join-Path $toolchainRoot 'rustup'
    $env:RUSTUP_TOOLCHAIN = 'stable-x86_64-pc-windows-msvc'
    # 把工具链 bin 前置到 PATH，便于 cargo 调用 rustc / clippy-driver / rustfmt。
    $env:PATH = "$(Join-Path $toolchainRoot 'cargo\bin');$env:PATH"
    & $cargoExe @args
}
else {
    # 回退到全局 cargo（协作者与 CI 使用标准 rustup 安装）。
    & cargo @args
}

exit $LASTEXITCODE
