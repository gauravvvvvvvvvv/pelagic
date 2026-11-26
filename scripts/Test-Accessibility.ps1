param([Parameter(Mandatory=$true)][string]$Executable,[Parameter(Mandatory=$true)][string]$Project,[Parameter(Mandatory=$true)][string]$Report,[switch]$GpuPreview,[switch]$EditorLayout,[switch]$AutomaticCpuFallback)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
$exePath=(Resolve-Path -LiteralPath $Executable).Path
$scenePath=(Resolve-Path -LiteralPath $Project).Path
$reportPath=[IO.Path]::GetFullPath($Report)
$isolated=Join-Path ([IO.Path]::GetDirectoryName($reportPath)) ('accessibility-state-'+[Guid]::NewGuid().ToString('N'))
$handleFile=$isolated+'.hwnd'
Add-Type 'using System; using System.Runtime.InteropServices; public static class PelagicTestWindow { [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr window, uint message, UIntPtr w, IntPtr l); [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr window, uint message, UIntPtr w, IntPtr l); [DllImport("user32.dll")] public static extern uint GetGuiResources(IntPtr process, uint flags); }'
# This client inspects only the process it starts and its native accessibility
# tree. It never queries the desktop root or captures the user's screen.
$gpuArgument=if($GpuPreview){' --legacy-ui --gpu'}elseif($AutomaticCpuFallback){''}else{' --legacy-ui'}
$app=Start-Process -FilePath $exePath -ArgumentList ('--open "'+$scenePath+'" "--test-storage='+$isolated+'" "--automation-handle='+$handleFile+'"'+$gpuArgument) -WindowStyle Hidden -PassThru
$window=[IntPtr]::Zero
try {
    Start-Sleep -Seconds 6
    $app.Refresh()
    if ($app.HasExited) {throw 'App exited before UI Automation test.'}
    $window=[IntPtr][Int64](Get-Content -LiteralPath $handleFile)
    $root=[Windows.Automation.AutomationElement]::FromHandle($window)
    if($AutomaticCpuFallback){
        $startup=Get-Content -LiteralPath (Join-Path $isolated 'startup.json') -Raw | ConvertFrom-Json
        if($startup.backend -ne 'cpu' -or $startup.reason -notmatch 'PELAGIC_DISABLE_GPU') {throw 'Expected automatic CPU fallback was not recorded.'}
    }
    function Find-Control([string]$name) {
        $condition=New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty,$name)
        $element=$null
        for($attempt=0;$attempt -lt 40;$attempt++){
            $element=$root.FindFirst([Windows.Automation.TreeScope]::Descendants,$condition)
            if($null -ne $element){break}
            Start-Sleep -Milliseconds 50
        }
        if ($null -eq $element) {throw ('Accessible control missing: '+$name)}
        return $element
    }
    if($GpuPreview){
        $ready=$false
        for($i=0;$i -lt 50;$i++){
            $condition=New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty,'GPU preview')
            if($null -ne $root.FindFirst([Windows.Automation.TreeScope]::Descendants,$condition)){$ready=$true;break}
            Start-Sleep -Milliseconds 100
        }
        if(-not $ready){throw 'Native GPU preview did not become ready.'}
    }
    if($EditorLayout){
        foreach($name in @('File','Edit','Window','Tools','Build','Select','Actor','Help')){$null=Find-Control $name}
        (Find-Control 'Output Log').GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
        Start-Sleep -Milliseconds 150
        (Find-Control 'Content Browser').GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
        Start-Sleep -Milliseconds 150
        $search=(Find-Control 'Search actors').GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern)
        $search.SetValue('Surface vessel')
        Start-Sleep -Milliseconds 150
        (Find-Control 'Surface vessel 1').GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
        Start-Sleep -Milliseconds 150
        $null=Find-Control 'Position X (m)'
        (Find-Control 'Search actors').GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue('')
        Start-Sleep -Milliseconds 150
        (Find-Control 'Inspect asset: Mounted sensors').GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
        Start-Sleep -Milliseconds 150
        $null=Find-Control 'Max range (m)'
    }
    $life=Find-Control 'Marine life'
    $invoke=$life.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern)
    $invoke.Invoke()
    Start-Sleep -Milliseconds 350
    $all=$root.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)
    $names=@($all | ForEach-Object {$_.Current.Name})
    $fish=$all | Where-Object {$_.Current.Name -like 'Fish count*'} | Select-Object -First 1
    if ($null -eq $fish) {throw 'Fish count edit not exposed.'}
    $value=$fish.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern)
    $old=$value.Current.Value
    $fishName=$fish.Current.Name
    $latency=[Diagnostics.Stopwatch]::StartNew()
    $value.SetValue('12')
    for($i=0;$i -lt 200;$i++){
        $current=(Find-Control $fishName).GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern)
        if($current.Current.Value -eq '12'){break}
        Start-Sleep -Milliseconds 10
    }
    $latency.Stop()
    $fish=Find-Control $fish.Current.Name
    $value=$fish.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern)
    if ($value.Current.Value -ne '12') {throw 'UI Automation SetValue failed.'}
    $value.SetValue($old)
    Start-Sleep -Milliseconds 200
    if($GpuPreview){$null=Find-Control 'GPU preview'}
    $play=Find-Control 'Play'
    $play.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    $null=Find-Control 'Pause'
    # Exercise the native paint/COM provider path without capturing any screen.
    # SendMessage targets only the HWND returned by this isolated test app.
    for($i=0;$i -lt 100;$i++){[PelagicTestWindow]::SendMessage($window,15,[UIntPtr]::Zero,[IntPtr]::Zero) | Out-Null}
    $app.Refresh();$privateBefore=$app.PrivateMemorySize64
    $gdiBefore=[PelagicTestWindow]::GetGuiResources($app.Handle,0)
    for($i=0;$i -lt 1000;$i++){[PelagicTestWindow]::SendMessage($window,15,[UIntPtr]::Zero,[IntPtr]::Zero) | Out-Null}
    $app.Refresh();$growth=$app.PrivateMemorySize64-$privateBefore
    $gdiGrowth=[Int64][PelagicTestWindow]::GetGuiResources($app.Handle,0)-[Int64]$gdiBefore
    if ($growth -gt 16777216 -or $gdiGrowth -gt 2){throw ('Native paint/provider growth exceeded budget: '+$growth+' bytes, '+$gdiGrowth+' GDI handles')}
    $pause=Find-Control 'Pause'
    $pause.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    $null=Find-Control 'Play'
    Start-Sleep -Milliseconds 900
    $app.Refresh();$cpu=$app.TotalProcessorTime.TotalMilliseconds
    Start-Sleep -Seconds 2
    $app.Refresh();$idle=$app.TotalProcessorTime.TotalMilliseconds-$cpu
    if ($idle -gt 50) {throw ('Idle CPU budget exceeded: '+$idle+' ms / 2 s')}
    @{pass=$true;gpu_preview=[bool]$GpuPreview;editor_layout=[bool]$EditorLayout;process_id=$app.Id;controls=$names;invoke=$true;set_value=$true;restored_value=$old;edit_feedback_ms=$latency.Elapsed.TotalMilliseconds;native_paint_soak=1000;private_growth_bytes=$growth;gdi_growth=$gdiGrowth;idle_cpu_ms_per_2s=$idle;working_set_bytes=$app.WorkingSet64;private_bytes=$app.PrivateMemorySize64;screen_capture=$false} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $reportPath -Encoding UTF8
} finally {
    if (-not $app.HasExited -and $window -ne [IntPtr]::Zero) {[PelagicTestWindow]::PostMessage($window,16,[UIntPtr]::Zero,[IntPtr]::Zero) | Out-Null}
    # A modified synthetic fixture may display an unsaved-changes dialog.
    # Tear down only the process this test launched; never leave it locking
    # the build executable or ask the user's real editor to close.
    if(-not $app.WaitForExit(2000)){Stop-Process -Id $app.Id -Force}
}
