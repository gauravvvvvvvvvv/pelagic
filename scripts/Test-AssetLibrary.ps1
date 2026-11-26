param([Parameter(Mandatory=$true)][string]$Executable,[Parameter(Mandatory=$true)][string]$Project,[Parameter(Mandatory=$true)][string]$Report,[switch]$GpuPreview)
# Native asset-library workflow: select library cards, assign a preset material,
# place a mesh instance, undo/redo, refuse removing a used mesh, save, restart.
# Only this isolated test app's HWND/UIA tree receives input. No desktop capture.
$ErrorActionPreference='Stop'
trap { Write-Host ('Failed at line '+$_.InvocationInfo.ScriptLineNumber+': '+$_.InvocationInfo.Line.Trim()); break }
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
Add-Type @'
using System;using System.Runtime.InteropServices;
public static class LibraryTestWindow {
 [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h,uint m,UIntPtr w,IntPtr l);
 [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h,uint m,UIntPtr w,IntPtr l);
 [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h,IntPtr after,int x,int y,int w,int height,uint flags);
}
'@
$exePath=(Resolve-Path -LiteralPath $Executable).Path
$reportPath=[IO.Path]::GetFullPath($Report)
$isolated=Join-Path ([IO.Path]::GetDirectoryName($reportPath)) ('library-state-'+[Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $isolated | Out-Null
$fixture=Join-Path $isolated 'library.pelagic'
Copy-Item -LiteralPath $Project -Destination $fixture
$original=Get-Content -LiteralPath $fixture -Raw | ConvertFrom-Json
$handleFile=Join-Path $isolated 'app.hwnd'
$arguments='--open "'+$fixture+'" "--test-storage='+$isolated+'" "--automation-handle='+$handleFile+'" --dark'
if($GpuPreview){$arguments+=' --gpu'}
$window=[IntPtr]::Zero;$app=$null
function Paint { [LibraryTestWindow]::SendMessage($script:window,15,[UIntPtr]::Zero,[IntPtr]::Zero) | Out-Null }
function Start-TestApp {
 if(Test-Path -LiteralPath $handleFile){Remove-Item -LiteralPath $handleFile}
 $script:app=Start-Process -FilePath $exePath -ArgumentList $arguments -WindowStyle Hidden -PassThru
 Start-Sleep -Seconds 6
 $script:app.Refresh();if($script:app.HasExited){throw 'Library test app exited.'}
 $script:window=[IntPtr][Int64](Get-Content -LiteralPath $handleFile)
 $dpi=[LibraryTestWindow]::GetDpiForWindow($script:window)/96.0
 [LibraryTestWindow]::SetWindowPos($script:window,[IntPtr]::Zero,0,0,[int](1280*$dpi),[int](900*$dpi),22) | Out-Null
 Paint
 $script:root=[Windows.Automation.AutomationElement]::FromHandle($script:window)
}
function Find([string]$name) {
 $c=New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty,$name)
 $e=$script:root.FindFirst([Windows.Automation.TreeScope]::Descendants,$c)
 if($null -eq $e){throw ('Control missing: '+$name)};return $e
}
function Invoke([string]$name){$e=Find $name;$p=$null;if(-not $e.TryGetCurrentPattern([Windows.Automation.InvokePattern]::Pattern,[ref]$p)){throw ('Not invokable: '+$name+' ('+$e.Current.ControlType.ProgrammaticName+')')};$p.Invoke();Start-Sleep -Milliseconds 150;Paint}
try {
 Start-TestApp
 if($GpuPreview){$null=Find 'GPU preview'}
 $entities=$original.entities.Count
 # Assign a built-in preset to the AUV.
 (Find 'Search actors').GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue('Survey AUV');Start-Sleep -Milliseconds 150;Paint
 Invoke 'Survey AUV hull'
 (Find 'Search actors').GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue('');Start-Sleep -Milliseconds 150;Paint
 Invoke 'Materials'
 Invoke 'Select library item: Weathered copper'
 Invoke 'Assign'
 # Place a second lander instance, then check one undo/redo entry.
 Invoke 'Meshes'
 Invoke 'Select library item: Seabed lander'
 Invoke 'Place'
 Invoke 'Save';$after=(Get-Content -LiteralPath $fixture -Raw | ConvertFrom-Json).entities.Count
 if($after -ne $entities+1){throw "Place did not add one body ($entities -> $after)."}
 Invoke 'Undo';Invoke 'Save';$undone=(Get-Content -LiteralPath $fixture -Raw | ConvertFrom-Json).entities.Count
 if($undone -ne $entities){throw "One undo did not remove the placed instance ($after -> $undone)."}
 Invoke 'Redo'
 # A used mesh cannot be removed.
 Invoke 'Select library item: Mooring buoy'
 Invoke 'Remove'
 $null=Find 'Select library item: Mooring buoy'
 Invoke 'Save'
 $saved=Get-Content -LiteralPath $fixture -Raw | ConvertFrom-Json
 if($saved.version -ne 5){throw "Saved schema $($saved.version), expected 5."}
 if($saved.entities.Count -ne $entities+1){throw "Expected $($entities+1) entities, saved $($saved.entities.Count)."}
 if($saved.library.meshes.Count -ne 3){throw 'Used mesh was removed.'}
 $copper=$saved.library.materials | Where-Object {$_.name -eq 'Weathered copper'}
 if($null -eq $copper){throw 'Preset was not copied into the project library.'}
 $auv=$saved.entities | Where-Object {$_.name -eq 'Survey AUV hull'}
 if($auv.material -ne $copper.id){throw 'AUV material handle was not saved.'}
 $placed=$saved.entities[-1]
 $lander=$saved.library.meshes | Where-Object {$_.name -eq 'Seabed lander'}
 if($placed.asset -ne $lander.id){throw 'Placed instance does not reference the lander mesh.'}
 [LibraryTestWindow]::PostMessage($window,16,[UIntPtr]::Zero,[IntPtr]::Zero) | Out-Null
 if(-not $app.WaitForExit(5000)){throw 'Saved fixture did not close cleanly.'}
 $window=[IntPtr]::Zero;Start-TestApp
 Invoke 'Materials'
 $null=Find 'Select library item: Weathered copper'
 Invoke 'Meshes'
 $null=Find 'Select library item: Seabed lander'
 @{pass=$true;gpu_preview=[bool]$GpuPreview;preset_assigned=$true;material_id=$copper.id;placed_asset=$placed.asset;one_undo_per_place=$true;used_mesh_remove_refused=$true;saved_schema=5;reopened=$true;storage=$isolated;screen_capture=$false} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $reportPath -Encoding UTF8
}finally{if($null -ne $app -and -not $app.HasExited){if($window -ne [IntPtr]::Zero){[LibraryTestWindow]::PostMessage($window,16,[UIntPtr]::Zero,[IntPtr]::Zero) | Out-Null};if(-not $app.WaitForExit(5000)){Stop-Process -Id $app.Id -Force}}}
