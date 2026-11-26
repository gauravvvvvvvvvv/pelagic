$ErrorActionPreference = 'Stop'
$rustRoot = (& rustup run stable-x86_64-pc-windows-gnu rustc --print sysroot).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Install the stable-x86_64-pc-windows-gnu Rust toolchain.' }
$env:PATH = (Join-Path $rustRoot 'lib\rustlib\x86_64-pc-windows-gnu\bin') + ';' + $env:PATH
# windows-rs raw-dylib import generation needs dlltool in addition to Rust's
# bundled linker. This changes this build process only, never the user's PATH.
if (-not (Get-Command dlltool.exe -ErrorAction SilentlyContinue)) {
    $localTool = Join-Path $env:USERPROFILE '.codex\tools\llvm-mingw-20261104'
    if (Test-Path -LiteralPath (Join-Path $localTool 'dlltool.exe')) {
        $env:PATH = $localTool + ';' + $env:PATH
    } else {
        throw 'Install LLVM-MinGW or MinGW binutils, add its bin folder to this shell PATH, then build again. See docs/BUILDING.md.'
    }
}
