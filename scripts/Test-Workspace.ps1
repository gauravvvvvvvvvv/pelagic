param([Parameter(Mandatory=$true)][string]$Executable,[Parameter(Mandatory=$true)][string]$Project,[Parameter(Mandatory=$true)][string]$Report,[switch]$GpuPreview)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class WorkspaceTestWindow {
 [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; }
 [DllImport("user32.dll")] public static extern bool ScreenToClient(IntPtr h, ref Point p);
 [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h,uint m,UIntPtr w,IntPtr l);
 [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h,uint m,UIntPtr w,IntPtr l);
 [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h,IntPtr after,int x,int y,int w,int height,uint flags);
}
'@
# All input, geometry and UIA queries target only the isolated app started here.
# No mouse-pointer movement, desktop-root query or screen capture is performed.
$exePath=(Resolve-Path -LiteralPath $Executable).Path
$projectPath=(Resolve-Path -LiteralPath $Project).Path
$reportPath=[IO.Path]::GetFullPath($Report)
$isolated=Join-Path ([IO.Path]::GetDirectoryName($reportPath)) ('workspace-state-'+[Guid]::NewGuid().ToString('N'))
$handleFile=$isolated+'.hwnd'
$arguments='--open "'+$projectPath+'" "--test-storage='+$isolated+'" "--automation-handle='+$handleFile+'"'
if($GpuPreview){$arguments+=' --gpu'}
$window=[IntPtr]::Zero
$app=$null
$checks=@()
function Start-TestApp {
 $script:app=Start-Process -FilePath $exePath -ArgumentList $arguments -WindowStyle Hidden -PassThru
 Start-Sleep -Seconds 6
 $script:app.Refresh()
 if($script:app.HasExited){throw 'Workspace test app exited.'}
 $script:window=[IntPtr][Int64](Get-Content -LiteralPath $handleFile)
 $dpi=[WorkspaceTestWindow]::GetDpiForWindow($script:window)/96.0
 [WorkspaceTestWindow]::SetWindowPos($script:window,[IntPtr]::Zero,0,0,[int](1280*$dpi),[int](900*$dpi),22) | Out-Null
 Repaint
 $script:root=[Windows.Automation.AutomationElement]::FromHandle($script:window)
}
function Find-Control([string]$name){
 $condition=New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty,$name)
 $element=$script:root.FindFirst([Windows.Automation.TreeScope]::Descendants,$condition)
 if($null -eq $element){throw ('Control missing: '+$name)}
 return $element
}
function Range([string]$name){return (Find-Control $name).GetCurrentPattern([Windows.Automation.RangeValuePattern]::Pattern)}
function Repaint { [WorkspaceTestWindow]::SendMessage($script:window,15,[UIntPtr]::Zero,[IntPtr]::Zero) | Out-Null }
function Key([int]$key){
 [WorkspaceTestWindow]::SendMessage($script:window,256,[UIntPtr]::new([uint32]$key),[IntPtr]::Zero) | Out-Null
 [WorkspaceTestWindow]::SendMessage($script:window,257,[UIntPtr]::new([uint32]$key),[IntPtr]::Zero) | Out-Null
 Repaint
}
function Mouse([uint32]$message,[int]$x,[int]$y){
 $packed=($x -band 65535) -bor (($y -band 65535) -shl 16)
 $buttons=if($message -eq 513 -or $message -eq 512){1}else{0}
 [WorkspaceTestWindow]::SendMessage($script:window,$message,[UIntPtr]::new([uint32]$buttons),[IntPtr]$packed) | Out-Null
}
function Divider-Point([string]$name){
 $bounds=(Find-Control $name).Current.BoundingRectangle
 $p=New-Object WorkspaceTestWindow+Point
 $p.X=[int]($bounds.X+$bounds.Width/2);$p.Y=[int]($bounds.Y+$bounds.Height/2)
 [WorkspaceTestWindow]::ScreenToClient($script:window,[ref]$p) | Out-Null
 return $p
}
try {
 Start-TestApp
 if($GpuPreview){$null=Find-Control 'GPU preview'}
 foreach($name in @('Sidebar width','Outliner height','Content Browser height')){
  $range=Range $name
  $value=$range.Current.Value
  $wanted=[Math]::Min($range.Current.Maximum,$value+30)
  if($wanted -le $value+5){throw "Test window has no resizing space for $name"}
  $range.SetValue($wanted)
  Start-Sleep -Milliseconds 150
  Repaint
  $actual=(Range $name).Current.Value
  if([Math]::Abs($actual-$wanted) -gt 1){throw "Range resize failed for $name : $actual vs $wanted"}
  $point=Divider-Point $name
  Mouse 513 $point.X $point.Y
  $endX=$point.X;$endY=$point.Y
  if($name -eq 'Sidebar width'){$endX-=20}elseif($name -eq 'Content Browser height'){$endY-=20}else{$endY+=20}
  Mouse 512 $endX $endY
  Mouse 514 $endX $endY
  Repaint
  $after=(Range $name).Current.Value
  if($after -le $actual+5){throw "Mouse drag failed for $name"}
  $mouseValue=$after
  (Find-Control $name).SetFocus()
  Start-Sleep -Milliseconds 100
  $growKey=if($name -eq 'Sidebar width'){37}elseif($name -eq 'Outliner height'){40}else{38}
  Key $growKey
  $after=(Range $name).Current.Value
  if($after -le $mouseValue+10){throw "Keyboard resize failed for $name"}
  # Escape restores the in-memory size and does not persist a partial drag.
  $point=Divider-Point $name
  Mouse 513 $point.X $point.Y
  Mouse 512 ($point.X-13) ($point.Y+13)
  Key 27
  Mouse 514 ($point.X-13) ($point.Y+13)
  Repaint
  if([Math]::Abs((Range $name).Current.Value-$after) -gt 1){throw "Escape failed for $name"}
  $checks+=@{divider=$name;before=$value;range_value=$actual;mouse_value=$mouseValue;keyboard_value=$after;drag_value=$after;escape_restored=$true}
 }
 # Selection and numeric properties must survive both maximize and restore.
 $search=(Find-Control 'Search actors').GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern)
 $search.SetValue('Surface vessel')
 Start-Sleep -Milliseconds 150
 (Find-Control 'Surface vessel 1').GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
 Start-Sleep -Milliseconds 150
 $position=(Find-Control 'Position X (m)').GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).Current.Value
 Key 122
 $null=Find-Control 'Restore'
 $condition=New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty,'Search actors')
 if($null -ne $root.FindFirst([Windows.Automation.TreeScope]::Descendants,$condition)){throw 'Outliner remained exposed in maximized view.'}
 Key 122
 $null=Find-Control 'Maximize'
 if((Find-Control 'Position X (m)').GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).Current.Value -ne $position){throw 'Selection/property changed while maximizing.'}
 if($GpuPreview){$null=Find-Control 'GPU preview'}
 $saved=Get-Content -LiteralPath (Join-Path $isolated 'preferences.json') -Raw | ConvertFrom-Json
 [WorkspaceTestWindow]::PostMessage($window,16,[UIntPtr]::Zero,[IntPtr]::Zero) | Out-Null
 if(-not $app.WaitForExit(5000)){throw 'Workspace test app did not close cleanly.'}
 $window=[IntPtr]::Zero
 Start-TestApp
 foreach($check in $checks){
  if([Math]::Abs((Range $check.divider).Current.Value-$check.drag_value) -gt 1){throw ('Restart did not restore '+$check.divider)}
 }
 $null=Find-Control 'Maximize'
 @{pass=$true;gpu_preview=[bool]$GpuPreview;checks=$checks;restart_restored=$true;maximize_restored_selection=$true;preferences=$saved.workspace;storage=$isolated;screen_capture=$false} | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $reportPath -Encoding UTF8
} finally {
 if($null -ne $app -and -not $app.HasExited -and $window -ne [IntPtr]::Zero){[WorkspaceTestWindow]::PostMessage($window,16,[UIntPtr]::Zero,[IntPtr]::Zero) | Out-Null}
}
