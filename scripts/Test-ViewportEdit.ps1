param([Parameter(Mandatory=$true)][string]$Executable,[Parameter(Mandatory=$true)][string]$Project,[Parameter(Mandatory=$true)][string]$Report,[switch]$GpuPreview)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
Add-Type @'
using System;using System.Runtime.InteropServices;
public static class TransformTestWindow {
 [StructLayout(LayoutKind.Sequential)] public struct Point {public int X,Y;}
 [DllImport("user32.dll")] public static extern bool ScreenToClient(IntPtr h,ref Point p);
 [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h,uint m,UIntPtr w,IntPtr l);
 [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h,uint m,UIntPtr w,IntPtr l);
 [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h,IntPtr after,int x,int y,int w,int height,uint flags);
}
'@
# Only this isolated test app's HWND/UIA tree receives synthetic input. No desktop capture.
$exePath=(Resolve-Path -LiteralPath $Executable).Path
$reportPath=[IO.Path]::GetFullPath($Report)
$isolated=Join-Path ([IO.Path]::GetDirectoryName($reportPath)) ('transform-state-'+[Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $isolated | Out-Null
$fixture=Join-Path $isolated 'editing.pelagic'
$scene=Get-Content -LiteralPath $Project -Raw | ConvertFrom-Json
$scene.camera.position=@(-4.0,-6.0,-9.0);$scene.camera.target=@(0.0,0.0,-12.0)
[IO.File]::WriteAllText($fixture,($scene | ConvertTo-Json -Depth 50),(New-Object Text.UTF8Encoding $false))
$handleFile=Join-Path $isolated 'app.hwnd'
$arguments='--open "'+$fixture+'" "--test-storage='+$isolated+'" "--automation-handle='+$handleFile+'" --dark'
if($GpuPreview){$arguments+=' --gpu'}
$window=[IntPtr]::Zero;$app=$null
function Paint { [TransformTestWindow]::SendMessage($script:window,15,[UIntPtr]::Zero,[IntPtr]::Zero) | Out-Null }
function Start-TestApp {
 $script:app=Start-Process -FilePath $exePath -ArgumentList $arguments -WindowStyle Hidden -PassThru
 Start-Sleep -Seconds 6
 $script:app.Refresh();if($script:app.HasExited){throw 'Transform test app exited.'}
 $script:window=[IntPtr][Int64](Get-Content -LiteralPath $handleFile)
 $dpi=[TransformTestWindow]::GetDpiForWindow($script:window)/96.0
 [TransformTestWindow]::SetWindowPos($script:window,[IntPtr]::Zero,0,0,[int](1280*$dpi),[int](900*$dpi),22) | Out-Null
 Paint
 $script:root=[Windows.Automation.AutomationElement]::FromHandle($script:window)
}
function Find([string]$name) {
 $c=New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty,$name)
 $e=$script:root.FindFirst([Windows.Automation.TreeScope]::Descendants,$c)
 if($null -eq $e){throw ('Control missing: '+$name)};return $e
}
function Value([string]$name){return [double](Find $name).GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).Current.Value}
function Invoke([string]$name){(Find $name).GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke();Start-Sleep -Milliseconds 120;Paint}
function Point([string]$name){$b=(Find $name).Current.BoundingRectangle;$p=New-Object TransformTestWindow+Point;$p.X=[int]($b.X+$b.Width/2);$p.Y=[int]($b.Y+$b.Height/2);[TransformTestWindow]::ScreenToClient($script:window,[ref]$p) | Out-Null;return $p}
function Mouse([uint32]$message,[int]$x,[int]$y){$packed=($x -band 65535) -bor (($y -band 65535) -shl 16);$buttons=if($message -eq 514){0}else{1};[TransformTestWindow]::SendMessage($script:window,$message,[UIntPtr]::new([uint32]$buttons),[IntPtr]$packed) | Out-Null}
function Key([int]$key){[TransformTestWindow]::SendMessage($script:window,256,[UIntPtr]::new([uint32]$key),[IntPtr]::Zero) | Out-Null;[TransformTestWindow]::SendMessage($script:window,257,[UIntPtr]::new([uint32]$key),[IntPtr]::Zero) | Out-Null;Paint}
function Drag([string]$name,[int]$dx,[int]$dy,[switch]$Cancel,[switch]$LoseCapture){
 $p=Point $name;Mouse 513 $p.X $p.Y
 for($i=1;$i -le 6;$i++){Mouse 512 ($p.X+[int]($dx*$i/6)) ($p.Y+[int]($dy*$i/6));Paint}
 if($Cancel){Key 27}
 if($LoseCapture){[TransformTestWindow]::SendMessage($script:window,533,[UIntPtr]::Zero,[IntPtr]::Zero) | Out-Null;Paint}
 Mouse 514 ($p.X+$dx) ($p.Y+$dy);Paint
}
function Assert-Close([double]$actual,[double]$expected,[string]$message){if([Math]::Abs($actual-$expected) -gt 0.00001){throw "$message : $actual versus $expected"}}
try {
 Start-TestApp
 if($GpuPreview){$null=Find 'GPU preview'}
 $p=Point 'Select in viewport: Reference vehicle';Mouse 513 $p.X $p.Y;Mouse 514 $p.X $p.Y;Paint
 $original=Value 'Position X (m)'
 Drag 'Move X axis' 36 0
 $moved=Value 'Position X (m)';if([Math]::Abs($moved-$original) -lt 0.1){throw 'Axis drag did not move the selected actor.'}
 Invoke 'Undo';Assert-Close (Value 'Position X (m)') $original 'One undo must restore the entire drag'
 Invoke 'Redo';Assert-Close (Value 'Position X (m)') $moved 'Redo must restore the entire drag'
 Drag 'Move X axis' 25 0 -Cancel;Assert-Close (Value 'Position X (m)') $moved 'Escape must cancel preview'
 Drag 'Move X axis' 25 0 -LoseCapture;Assert-Close (Value 'Position X (m)') $moved 'Capture loss must cancel preview'
 Key 50
 $beforeHeading=Value 'Heading (degrees)';Drag 'Rotate heading' 15 -18
 $heading=Value 'Heading (degrees)';if([Math]::Abs($heading-$beforeHeading) -lt 0.5){throw 'Heading ring drag did not rotate actor.'}
 Invoke 'Undo';Assert-Close (Value 'Heading (degrees)') $beforeHeading 'Rotation undo'
 Invoke 'Redo';Assert-Close (Value 'Heading (degrees)') $heading 'Rotation redo'
 Key 51
 $beforeSize=@((Value 'Body size X (m)'),(Value 'Body size Y (m)'),(Value 'Body size Z (m)'))
 Drag 'Resize uniformly' 24 -24
 $size=@((Value 'Body size X (m)'),(Value 'Body size Y (m)'),(Value 'Body size Z (m)'))
 $ratio=$size[0]/$beforeSize[0];if($ratio -lt 1.05){throw 'Uniform resize did not enlarge actor.'}
 Assert-Close ($size[1]/$beforeSize[1]) $ratio 'Y dimension must scale uniformly'
 Assert-Close ($size[2]/$beforeSize[2]) $ratio 'Z dimension must scale uniformly'
 Invoke 'Save'
 $saved=Get-Content -LiteralPath $fixture -Raw | ConvertFrom-Json
 Assert-Close $saved.reference.mass $scene.reference.mass 'Resize must preserve authored mass'
 Assert-Close $saved.reference.volume $scene.reference.volume 'Resize must preserve authored volume'
 Invoke 'Play';Start-Sleep -Milliseconds 150;Paint
 $c=New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty,'Resize uniformly')
 if($null -ne $root.FindFirst([Windows.Automation.TreeScope]::Descendants,$c)){throw 'Transform handles remained active during simulation.'}
 Invoke 'Pause';Invoke 'Reset';$null=Find 'Resize uniformly'
 if($GpuPreview){$null=Find 'GPU preview'}
 [TransformTestWindow]::PostMessage($window,16,[UIntPtr]::Zero,[IntPtr]::Zero) | Out-Null
 if(-not $app.WaitForExit(5000)){throw 'Edited fixture did not close cleanly.'}
 $window=[IntPtr]::Zero;Start-TestApp
 Invoke 'Select in viewport: Reference vehicle';Assert-Close (Value 'Position X (m)') $moved 'Saved position reload';Assert-Close (Value 'Heading (degrees)') $heading 'Saved heading reload'
 Key 51;Assert-Close (Value 'Body size X (m)') $size[0] 'Saved dimensions reload'
 Key 49;(Find 'Move X axis').SetFocus();Start-Sleep -Milliseconds 100;Key 39
 Assert-Close (Value 'Position X (m)') ($moved+0.1) 'Keyboard axis nudge'
 Invoke 'Undo';Assert-Close (Value 'Position X (m)') $moved 'Keyboard nudge undo'
 @{pass=$true;gpu_preview=[bool]$GpuPreview;viewport_click_selected=$true;axis_before=$original;axis_after=$moved;one_undo_per_drag=$true;escape_cancelled=$true;capture_loss_cancelled=$true;keyboard_nudge=$true;heading=$heading;uniform_ratio=$ratio;saved_reloaded=$true;live_edit_guard=$true;mass_volume_preserved=$true;storage=$isolated;screen_capture=$false} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $reportPath -Encoding UTF8
}finally{if($null -ne $app -and -not $app.HasExited -and $window -ne [IntPtr]::Zero){[TransformTestWindow]::PostMessage($window,16,[UIntPtr]::Zero,[IntPtr]::Zero) | Out-Null}}
