param([switch]$SkipTests, [switch]$Launch)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Initialize-Toolchain.ps1')
$project = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $project
try {
    & cargo +stable-x86_64-pc-windows-gnu fmt --all --check
    if ($LASTEXITCODE -ne 0) { throw 'Formatting check failed.' }
    if (-not $SkipTests) {
        & cargo +stable-x86_64-pc-windows-gnu test --workspace --all-targets --release --offline --jobs 1
        if ($LASTEXITCODE -ne 0) { throw 'Tests failed.' }
    }
    & cargo +stable-x86_64-pc-windows-gnu build --release -p pelagic-desktop --offline --jobs 1
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    $built = Join-Path $project 'target\release\pelagic-desktop.exe'
    $packaged = Join-Path $project 'Pelagic.exe'
    try { Copy-Item -LiteralPath $built -Destination $packaged -Force -ErrorAction Stop }
    catch { throw 'Close Pelagic.exe and rerun packaging; your existing app was left in place.' }
    if ((Get-FileHash -LiteralPath $built).Hash -ne (Get-FileHash -LiteralPath $packaged).Hash) { throw 'Packaged binary verification failed.' }
    Write-Output "Packaged $packaged"
    if ($Launch) { Start-Process -FilePath $packaged -WorkingDirectory $project }
} finally { Pop-Location }
