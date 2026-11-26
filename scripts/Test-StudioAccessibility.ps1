param([Parameter(Mandatory=$true)][string]$Executable,[Parameter(Mandatory=$true)][string]$Report)
# UI Automation test for the GPU editor shell (egui + AccessKit).
# This client inspects only the process it starts, through that process's own
# window handle. It never queries the desktop root or captures the screen, and
# all editor state goes to an isolated --test-storage folder.
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
Add-Type 'using System; using System.Runtime.InteropServices; public static class PelagicStudioTestWindow { [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd); [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hwnd, int command); [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int width, int height, uint flags); }'
$exePath=(Resolve-Path -LiteralPath $Executable).Path
$reportPath=[IO.Path]::GetFullPath($Report)
$isolated=Join-Path ([IO.Path]::GetDirectoryName($reportPath)) ('studio-a11y-state-'+[Guid]::NewGuid().ToString('N'))
$handleFile=$isolated+'.hwnd'
$fixture=Join-Path (Split-Path $PSScriptRoot -Parent) 'examples/scenes/asset-library.pelagic'
$app=Start-Process -FilePath $exePath -ArgumentList ('--open "'+$fixture+'" "--test-storage='+$isolated+'" "--automation-handle='+$handleFile+'"') -WindowStyle Hidden -PassThru
$checks=[ordered]@{}
try {
    for($i=0;$i -lt 150 -and -not (Test-Path -LiteralPath $handleFile);$i++){Start-Sleep -Milliseconds 100}
    if(-not (Test-Path -LiteralPath $handleFile)){throw 'Editor did not report its window handle.'}
    $hwnd=[IntPtr][Int64](Get-Content -LiteralPath $handleFile)
    $root=[Windows.Automation.AutomationElement]::FromHandle($hwnd)
    function Find-Control([string]$name,[int]$tries=40) {
        $condition=New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty,$name)
        for($i=0;$i -lt $tries;$i++){
            $element=$root.FindFirst([Windows.Automation.TreeScope]::Descendants,$condition)
            if($null -ne $element){return $element}
            Start-Sleep -Milliseconds 100
        }
        throw ('Accessible control missing: '+$name)
    }
    function Invoke-Control([string]$name) {
        (Find-Control $name).GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
        Start-Sleep -Milliseconds 250
    }
    # Plain names: icons and padding are not read aloud.
    $expected=@('File','Edit','Window','Help','Save','Open','Import','Undo','Redo','Move','Rotate','Scale','Snap','Play','Step','Reset','Graphics settings','Viewport','Earth','Outliner','Details','Content Browser','Output Log','3D viewport','Search actors','Navigate','Environment')
    foreach($name in $expected){$null=Find-Control $name}
    $checks.names=$expected.Count
    $all=$root.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)
    $decorated=@($all | Where-Object {$_.Current.Name -match '^[\s\u2190-\u2bff]|^\s|\s$'} | ForEach-Object {$_.Current.Name})
    if($decorated.Count){throw ('Names still decorated: '+($decorated -join ' | '))}
    $buttons=@($all | Where-Object {$_.Current.ControlType -eq [Windows.Automation.ControlType]::Button -and [string]::IsNullOrWhiteSpace($_.Current.Name)})
    if($buttons.Count){throw ($buttons.Count.ToString()+' unnamed buttons')}
    $checks.elements=$all.Count
    # Resize only this test window. Catch wrapped button text and overlapping
    # toolbar controls at laptop width, using the app's accessibility bounds.
    $scale=[PelagicStudioTestWindow]::GetDpiForWindow($hwnd)/96.0
    [PelagicStudioTestWindow]::ShowWindow($hwnd,9) | Out-Null
    if(-not [PelagicStudioTestWindow]::SetWindowPos($hwnd,[IntPtr]::Zero,0,0,[int](1000*$scale),[int](720*$scale),6)){throw 'Could not resize the isolated editor'}
    $compactOk=$false
    for($i=0;$i -lt 40;$i++){
        Start-Sleep -Milliseconds 100
        $graphicsBounds=(Find-Control 'Graphics settings').Current.BoundingRectangle
        $resetBounds=(Find-Control 'Reset').Current.BoundingRectangle
        if($graphicsBounds.Width -ge 90*$scale -and $graphicsBounds.Height -le 50*$scale -and -not $graphicsBounds.IntersectsWith($resetBounds)){$compactOk=$true;break}
    }
    if(-not $compactOk){throw ('Compact toolbar text wraps or overlaps Reset: '+$graphicsBounds.ToString())}
    $checks.compact_toolbar='ok'
    [PelagicStudioTestWindow]::ShowWindow($hwnd,3) | Out-Null
    Start-Sleep -Milliseconds 250
    Invoke-Control 'Navigate'
    $null=Find-Control 'Camera speed'
    $null=Find-Control 'Go to land'
    Invoke-Control 'Navigate'
    Invoke-Control 'Environment'
    $null=Find-Control 'Live conditions'
    $null=Find-Control 'Water clarity'
    Invoke-Control 'Environment'
    $checks.viewport_menus='ok'
    # Actions: start/stop the simulation, select an actor, switch tabs.
    Invoke-Control 'Play'
    $null=Find-Control 'Pause'
    Invoke-Control 'Pause'
    $null=Find-Control 'Play'
    $checks.play_pause='ok'
    $hint=New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty,'Select an actor to view its details.')
    if($null -eq $root.FindFirst([Windows.Automation.TreeScope]::Descendants,$hint)){throw 'Details placeholder missing before selection.'}
    # Outliner rows are selectable items (UIA Toggle pattern).
    (Find-Control 'Ocean environment').GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    for($i=0;$i -lt 30 -and $null -ne $root.FindFirst([Windows.Automation.TreeScope]::Descendants,$hint);$i++){Start-Sleep -Milliseconds 100}
    if($null -ne $root.FindFirst([Windows.Automation.TreeScope]::Descendants,$hint)){throw 'Selecting an actor through UI Automation did not open its details.'}
    $checks.select='ok'
    # Typing into the Outliner search filters the actor list.
    (Find-Control 'Search actors').GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue('Seabed')
    $other=New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty,'Reference vehicle')
    for($i=0;$i -lt 30 -and $null -ne $root.FindFirst([Windows.Automation.TreeScope]::Descendants,$other);$i++){Start-Sleep -Milliseconds 100}
    if($null -ne $root.FindFirst([Windows.Automation.TreeScope]::Descendants,$other)){throw 'Search typed through UI Automation did not filter the Outliner.'}
    $null=Find-Control 'Seabed'
    (Find-Control 'Search actors').GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue('')
    $checks.search='ok'
    $tab=Find-Control 'Earth'
    $tab.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    $null=Find-Control 'Earth globe'
    $null=Find-Control 'Map layer'
    foreach($name in @('Live waves','Sea forecast','Live sea in new scenes','Map layers','Go to latitude, longitude or undersea feature')){$null=Find-Control $name}
    $create=Find-Control 'Create ocean scene here'
    if(-not $create.Current.IsEnabled){throw 'Create ocean scene here is disabled'}
    $checks.create_scene_button='ok'
    $checks.earth_tab='ok'
    (Find-Control 'Experiments').GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    $ellipsis=[char]0x2026
    foreach($name in @('Record','Batch duration','Run batch',"Open recording$ellipsis","Export mounted camera$ellipsis",'Run campaign','Campaign runs','Max nav error limit')){$null=Find-Control $name}
    $checks.experiments_tab='ok'
    (Find-Control 'Sensor Monitor').GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    $null=Find-Control 'View from mounted camera'
    $checks.sensor_tab='ok'
    # Trials workspace and the fin/rudder editor for a selected vehicle. A
    # real place has no vehicle until a tool needs one: the Trials example
    # adds the reference vehicle and selects it.
    (Find-Control 'Trials').GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    Invoke-Control 'Show me an example'
    $null=Find-Control 'Reference vehicle' 80
    try { foreach($name in @('Add rudder','Add hydroplanes','Add fixed fin')){$null=Find-Control $name 20} }
    catch {
        (Find-Control 'Reference vehicle').GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
        foreach($name in @('Add rudder','Add hydroplanes','Add fixed fin')){$null=Find-Control $name}
    }
    $checks.fins='ok'
    foreach($name in @('Run trials','Approach thrust','Rudder angle','Wave height','Operating depth','Min top speed limit')){$null=Find-Control $name}
    $checks.trials_tab='ok'
    (Find-Control 'Seakeeping').GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    foreach($name in @('Compute seakeeping','Transit thrust','Motion criteria','Roll RMS limit','Pick a place on Earth')){$null=Find-Control $name}
    $checks.seakeeping_tab='ok'
    (Find-Control 'Search & Rescue').GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    foreach($name in @('Plan search','Search object','Last known latitude','Last known longitude','Hours in the water','Search speed','Sweep width','Time on scene')){$null=Find-Control $name}
    $checks.sar_tab='ok'
    (Find-Control 'Survey').GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    foreach($name in @('Plan survey','Run survey with vehicle physics','IHO S-44 order','Swath angle','Survey speed','Area length','Line heading','Overlap')){$null=Find-Control $name}
    $checks.survey_tab='ok'
    (Find-Control 'Ship Handling').GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    foreach($name in @('Start encounter','Run all encounters with autopilot','Encounter','Safe passing distance')){$null=Find-Control $name}
    $checks.ship_tab='ok'
    (Find-Control 'Inspection').GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    foreach($name in @('Plan inspection','Fly inspection with vehicle physics','Inspection target','Required sharpness')){$null=Find-Control $name}
    $checks.inspection_tab='ok'
    (Find-Control 'Spill').GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    foreach($name in @('Drift spill','Oil','Spill volume','Spill latitude','Spill duration')){$null=Find-Control $name}
    $checks.spill_tab='ok'
    $checks.result='passed'
} catch {
    $checks.result='failed'
    $checks.error=$_.Exception.Message
} finally {
    # Only the process this script started, with isolated storage.
    if(-not $app.HasExited){Stop-Process -Id $app.Id -Force}
    Remove-Item -LiteralPath $handleFile -ErrorAction SilentlyContinue
}
$checks | ConvertTo-Json | Set-Content -LiteralPath $reportPath -Encoding UTF8
$checks | ConvertTo-Json
if($checks.result -ne 'passed'){exit 1}
