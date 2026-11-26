param([Parameter(Mandatory=$true)][string]$PackageFolder,[Parameter(Mandatory=$true)][string]$OutputFolder)
$ErrorActionPreference='Stop'
$package=(Resolve-Path -LiteralPath $PackageFolder).Path
$output=[IO.Path]::GetFullPath($OutputFolder)
if (Test-Path -LiteralPath $output) {throw 'Choose a new package-test output directory.'}
New-Item -ItemType Directory -Path $output | Out-Null
$setup=Join-Path $package 'PelagicSetup.exe'
$installed=Join-Path $output 'Installed'
function Run-Setup([string]$program,[string[]]$arguments,[string]$report,[bool]$expected=$true) {
    $quoted=@($arguments | ForEach-Object {'"'+$_+'"'})
    $quoted+=@('--quiet','--report',('"'+$report+'"'))
    $process=Start-Process -FilePath $program -ArgumentList $quoted -WindowStyle Hidden -Wait -PassThru
    $result=Get-Content -LiteralPath $report -Raw | ConvertFrom-Json
    if ($result.ok -ne $expected -or (($process.ExitCode -eq 0) -ne $expected)) {throw ('Installer result mismatch: '+$result.message)}
}
Run-Setup $setup @('--verify',$package) (Join-Path $output 'verify.json')
Run-Setup $setup @('--destination',$installed) (Join-Path $output 'install.json')
$first=Get-Content -LiteralPath (Join-Path $installed 'current.json') -Raw | ConvertFrom-Json
$launchReport=Join-Path $output 'launch-project.json'
$launcher=Join-Path $installed 'Pelagic.exe'
Start-Process -FilePath $launcher -ArgumentList @('--check-project',('"'+(Join-Path $package 'Examples\v5-robotics.pelagic')+'"'),('"'+$launchReport+'"')) -WindowStyle Hidden -Wait
for($i=0;$i -lt 50 -and -not (Test-Path -LiteralPath $launchReport);$i++){Start-Sleep -Milliseconds 100}
if (-not (Get-Content -LiteralPath $launchReport -Raw | ConvertFrom-Json).valid) {throw 'Stable launcher project check failed.'}
$alternate=Join-Path $output 'UpdateSource'
Copy-Item -LiteralPath $package -Destination $alternate -Recurse
Add-Content -LiteralPath (Join-Path $alternate 'README.txt') -Value 'Installation update smoke fixture.'
$manifest=Get-Content -LiteralPath (Join-Path $alternate 'SHA256.json') -Raw | ConvertFrom-Json
$readme=$manifest | Where-Object path -eq 'README.txt'
$readme.sha256=(Get-FileHash -LiteralPath (Join-Path $alternate 'README.txt')).Hash
$readme.bytes=(Get-Item -LiteralPath (Join-Path $alternate 'README.txt')).Length
$manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $alternate 'SHA256.json') -Encoding UTF8
Run-Setup (Join-Path $alternate 'PelagicSetup.exe') @('--destination',$installed) (Join-Path $output 'update.json')
$second=Get-Content -LiteralPath (Join-Path $installed 'current.json') -Raw | ConvertFrom-Json
if ($first.release -eq $second.release) {throw 'Update did not select a new verified payload.'}
Run-Setup $launcher @('--rollback') (Join-Path $output 'rollback.json')
$rolled=Get-Content -LiteralPath (Join-Path $installed 'current.json') -Raw | ConvertFrom-Json
if ($rolled.release -ne $first.release) {throw 'Rollback did not restore the previous payload.'}
Add-Content -LiteralPath (Join-Path $alternate 'README.txt') -Value 'Deliberate corruption; manifest unchanged.'
Run-Setup (Join-Path $alternate 'PelagicSetup.exe') @('--destination',$installed) (Join-Path $output 'corrupt.json') $false
if ((Get-Content -LiteralPath (Join-Path $installed 'current.json') -Raw | ConvertFrom-Json).release -ne $first.release) {throw 'Corrupted payload changed the selected release.'}
@{pass=$true;verified_install=$true;stable_launch=$true;update=$true;rollback=$true;corruption_rejected=$true;release=$first.release} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $output 'summary.json') -Encoding UTF8
Write-Output ('Package install/update/rollback/corruption checks passed: '+$output)
