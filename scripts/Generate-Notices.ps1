param([Parameter(Mandatory=$true)][string]$Destination)
$ErrorActionPreference='Stop'
$metadata=& cargo +stable-x86_64-pc-windows-gnu metadata --locked --offline --format-version 1 --filter-platform x86_64-pc-windows-gnu
if ($LASTEXITCODE -ne 0) {throw 'Dependency metadata failed.'}
$metadata=$metadata | ConvertFrom-Json
$folder=[IO.Path]::GetFullPath($Destination)
New-Item -ItemType Directory -Force -Path $folder | Out-Null
# Crates whose published archives omit their licence files; the exact
# upstream texts are vendored under third-party/ (see each SOURCE.md).
$vendored=@{
    'accesskit'='AccessKit'; 'accesskit_consumer'='AccessKit'; 'accesskit_windows'='AccessKit'; 'accesskit_winit'='AccessKit'
    'ecolor'='egui'; 'egui'='egui'; 'egui-wgpu'='egui'; 'egui-winit'='egui'; 'emath'='egui'; 'epaint'='egui'
    'clipboard-win'='clipboard-win'; 'profiling'='profiling'; 'spirv'='rspirv'
    'zune-core'='zune-core'; 'zune-jpeg'='zune-jpeg'
}
$inventory=@()
$missing=@()
foreach ($package in ($metadata.packages | Where-Object source | Sort-Object name,version)) {
    $target=Join-Path $folder ($package.name+'-'+$package.version)
    New-Item -ItemType Directory -Force -Path $target | Out-Null
    $source=Split-Path -Parent $package.manifest_path
    $files=@(Get-ChildItem -LiteralPath $source -File | Where-Object {$_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)'})
    if ($vendored.ContainsKey($package.name)) {
        $files+=Get-ChildItem -LiteralPath (Join-Path (Join-Path (Split-Path -Parent $PSScriptRoot) 'third-party') $vendored[$package.name]) -File -Filter 'LICENSE*'
    }
    if ($package.license_file) {$files+=Get-Item -LiteralPath (Join-Path $source $package.license_file)}
    $copied=@()
    foreach ($file in ($files | Sort-Object FullName -Unique)) {Copy-Item -LiteralPath $file.FullName -Destination $target -Force; $copied+=$file.Name}
    if ($copied.Count -eq 0) {$missing+=($package.name+' '+$package.version+' ('+$package.license+')'); continue}
    $inventory+=[pscustomobject]@{name=$package.name;version=$package.version;license=$package.license;source=$package.source;texts=$copied}
}
if ($missing.Count) {throw ('No licence text found for: '+($missing -join '; '))}
$inventory | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $folder 'inventory.json') -Encoding UTF8
Write-Output ('Collected '+$inventory.Count+' locked dependency licence inventories.')
