param([switch]$SkipTests)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Initialize-Toolchain.ps1')
$project = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $project
try {
    & cargo +stable-x86_64-pc-windows-gnu clippy --workspace --all-targets --offline -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'Strict Clippy check failed.' }
    & (Join-Path $PSScriptRoot 'Build-Desktop.ps1') -SkipTests:$SkipTests
    & cargo +stable-x86_64-pc-windows-gnu build --release --workspace --bins --examples --offline --jobs 1
    if ($LASTEXITCODE -ne 0) { throw 'SDK build failed.' }
    $version=((& cargo +stable-x86_64-pc-windows-gnu metadata --offline --no-deps --format-version 1 | ConvertFrom-Json).packages | Where-Object name -eq 'pelagic').version
    $stage = Join-Path $project ('dist\Pelagic-'+$version+'-' + (Get-Date).ToUniversalTime().ToString('yyyyMMdd-HHmmssfff'))
    New-Item -ItemType Directory -Path $stage | Out-Null
    foreach ($name in @('SDK','Examples','Data','Docs','Source')) { New-Item -ItemType Directory -Path (Join-Path $stage $name) | Out-Null }
    $source=Join-Path $stage 'Source'
    foreach ($file in @('Cargo.toml','Cargo.lock','build.rs','LICENSE','NOTICE','README.md','ROADMAP.md','CHANGELOG.md','CONTRIBUTING.md','SECURITY.md','.gitignore','.gitattributes')) {Copy-Item -LiteralPath (Join-Path $project $file) -Destination $source}
    foreach ($folder in @('src','crates','apps','examples','scripts','tests','include','third-party','.cargo','.github','docs')) {Copy-Item -LiteralPath (Join-Path $project $folder) -Destination (Join-Path $source $folder) -Recurse}
    Copy-Item -LiteralPath (Join-Path $project 'Pelagic.exe') -Destination $stage
    Copy-Item -LiteralPath (Join-Path $project 'target\release\examples\distribution.exe') -Destination (Join-Path $stage 'PelagicSetup.exe')
    Copy-Item -LiteralPath (Join-Path $project 'target\release\pelagic.dll') -Destination (Join-Path $stage 'SDK')
    Copy-Item -LiteralPath (Join-Path $project 'include\pelagic.h') -Destination (Join-Path $stage 'SDK')
    foreach ($example in @('engine_host','mavlink','mavlink_loopback','experiment','acceptance')) {
        Copy-Item -LiteralPath (Join-Path $project ('target\release\examples\'+$example+'.exe')) -Destination (Join-Path $stage 'SDK')
        Copy-Item -LiteralPath (Join-Path $project ('examples\'+$example+'.rs')) -Destination (Join-Path $stage 'SDK')
    }
    Copy-Item -Path (Join-Path $project 'examples\scenes\v5-*.pelagic') -Destination (Join-Path $stage 'Examples')
    Copy-Item -LiteralPath (Join-Path $project 'examples\runs\v5-reference.pelagic-run') -Destination (Join-Path $stage 'Examples')
    Copy-Item -Path (Join-Path $project 'examples\data\*') -Destination (Join-Path $stage 'Data')
    Copy-Item -Path (Join-Path $project 'docs\*.md') -Destination (Join-Path $stage 'Docs')
    Copy-Item -LiteralPath (Join-Path $project 'README.md') -Destination (Join-Path $stage 'Docs\SOURCE-README.md')
    Copy-Item -LiteralPath (Join-Path $project 'CHANGELOG.md') -Destination (Join-Path $stage 'Docs')
    Copy-Item -LiteralPath (Join-Path $project 'CONTRIBUTING.md') -Destination (Join-Path $stage 'Docs')
    Copy-Item -Path (Join-Path $project 'tests\*.rs') -Destination (Join-Path $stage 'SDK')
    & (Join-Path $PSScriptRoot 'Generate-Notices.ps1') -Destination (Join-Path $stage 'ThirdParty')
    $evidence=Join-Path $project 'docs\evidence'
    if (Test-Path -LiteralPath $evidence) {Copy-Item -LiteralPath $evidence -Destination (Join-Path $stage 'Docs\evidence') -Recurse}
    if (Test-Path -LiteralPath (Join-Path $project 'LICENSE')) { Copy-Item -LiteralPath (Join-Path $project 'LICENSE') -Destination $stage }
    Copy-Item -LiteralPath (Join-Path $project 'NOTICE') -Destination $stage
    Copy-Item -LiteralPath (Join-Path $project 'apps\desktop\assets\OFL-Geist.txt') -Destination (Join-Path $stage 'Docs\Geist-OFL.txt')
    Copy-Item -LiteralPath (Join-Path $project 'apps\desktop\assets\OFL-Manrope.txt') -Destination (Join-Path $stage 'Docs\Manrope-OFL.txt')
    @'
Pelagic — native Rust desktop and SDK

Run Pelagic.exe. Choose Try reference scene, or Open a project in Examples.
Run PelagicSetup.exe for verified per-user installation and a stable launcher.
The app contains its font, logo, startup audio and CPU renderer.
No Python, JavaScript, browser or Rust installation is required to run it.

SDK examples from this folder in PowerShell:
  .\SDK\engine_host.exe .\SDK\pelagic.dll
  .\SDK\mavlink_loopback.exe
  .\SDK\experiment.exe .\Examples\v5-robotics.pelagic .\MyDataset 5

The full Rust source stays in the authoring repository. Docs describe model
limits, data attribution, supported budgets and release measurements.
This build is unsigned; SHA256.json detects corruption, not publisher identity.
The supplied brand logo remains the project owner's artwork.
'@ | Set-Content -LiteralPath (Join-Path $stage 'README.txt') -Encoding UTF8
    $manifest = Get-ChildItem -LiteralPath $stage -File -Recurse | ForEach-Object {
        [pscustomobject]@{ path=$_.FullName.Substring($stage.Length+1); sha256=(Get-FileHash -LiteralPath $_.FullName).Hash; bytes=$_.Length }
    }
    $manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $stage 'SHA256.json') -Encoding UTF8
    $archive = $stage + '.zip'
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $archive
    ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash+'  '+[IO.Path]::GetFileName($archive)) | Set-Content -LiteralPath ($archive+'.sha256') -Encoding ASCII
    Write-Output ('Portable package: ' + $archive)
} finally { Pop-Location }
