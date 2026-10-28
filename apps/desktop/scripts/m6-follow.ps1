# M6 C1–C7 helper (Windows, UI Automation): start the release app with a show,
# press 開始 (follow with the saved terminal settings), and record the text the
# window shows (status tiles and diagnostics) every -IntervalMs as JSONL until
# the stop file appears. Live is operated by hand meanwhile. Nothing here
# talks to Live or MIDI ports; the app does, with the saved terminal settings.
#
#   pwsh apps/desktop/scripts/m6-follow.ps1 -Show artifacts\m6\m6-live.show.json -Out artifacts\m6\win-m6-app.jsonl
#   New-Item <Out>.stop   # stops following, takes a screenshot, closes the app
#   New-Item <Out>.shot   # takes a screenshot now (<Out>-<n>.png) and goes on
# Soak (NFR07): -SampleSeconds 300 -DurationSeconds 7200 logs the text and the
# working set of omoiiro.exe with all descendant processes (WebView2) at 10 s
# and every 300 s instead of every -IntervalMs, and stops after 2 hours.
param(
  [Parameter(Mandatory)] [string]$Show,
  [Parameter(Mandatory)] [string]$Out,
  [int]$IntervalMs = 500,
  [int]$SampleSeconds = 0,
  [int]$DurationSeconds = 0,
  [string]$Exe = ""
)
$ErrorActionPreference = "Stop"
$Repo = (Resolve-Path "$PSScriptRoot\..\..\..").Path
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Repo "target" }
if (-not $Exe) { $Exe = Join-Path $TargetDir "release\omoiiro.exe" }
$Show = (Resolve-Path $Show).Path
$Out = [System.IO.Path]::GetFullPath($Out)
$Stop = "$Out.stop"; $ShotReq = "$Out.shot"
Remove-Item $Stop, $ShotReq -ErrorAction SilentlyContinue

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class M6W {
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT r, int s);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
$AE = [System.Windows.Automation.AutomationElement]
$TS = [System.Windows.Automation.TreeScope]
$CT = [System.Windows.Automation.ControlType]

$app = Start-Process -FilePath $Exe -ArgumentList @("`"$Show`"") -PassThru
$win = $null
for ($i = 0; $i -lt 100 -and -not $win; $i++) {
  Start-Sleep -Milliseconds 200
  $c = New-Object System.Windows.Automation.AndCondition(
    (New-Object System.Windows.Automation.PropertyCondition($AE::ProcessIdProperty, $app.Id)),
    (New-Object System.Windows.Automation.PropertyCondition($AE::NameProperty, "OMOI-IRO")))
  $win = $AE::RootElement.FindFirst($TS::Children, $c)
}
if (-not $win) { throw "OMOI-IRO window not found" }

function Press($name) {
  $cond = New-Object System.Windows.Automation.AndCondition(
    (New-Object System.Windows.Automation.PropertyCondition($AE::ControlTypeProperty, $CT::Button)),
    (New-Object System.Windows.Automation.PropertyCondition($AE::NameProperty, $name)))
  for ($i = 0; $i -lt 50; $i++) {
    $b = $win.FindFirst($TS::Descendants, $cond)
    if ($b -and $b.Current.IsEnabled) { $b.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke(); return }
    Start-Sleep -Milliseconds 200
  }
  throw "button '$name' not found or disabled"
}
function Texts { $win.FindAll($TS::Descendants, (New-Object System.Windows.Automation.PropertyCondition($AE::ControlTypeProperty, $CT::Text))) | ForEach-Object { $_.Current.Name } }
$shotN = 0
function Shot {
  $script:shotN++
  $h = [IntPtr]$win.Current.NativeWindowHandle
  $r = New-Object M6W+RECT
  [M6W]::DwmGetWindowAttribute($h, 9, [ref]$r, 16) | Out-Null
  $bmp = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size); $g.Dispose()
  $file = ($Out -replace '\.jsonl$', '') + "-$script:shotN.png"
  $bmp.Save($file, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
  return $file
}
function Log($kind, $data) {
  $line = [ordered]@{ t = (Get-Date).ToString("HH:mm:ss.fff"); kind = $kind; data = $data } | ConvertTo-Json -Compress -Depth 4
  Add-Content -Path $Out -Value $line -Encoding utf8
}

function Tree-Memory {
  $all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId
  $ids = New-Object System.Collections.Generic.List[int]
  $ids.Add($app.Id)
  for ($k = 0; $k -lt $ids.Count; $k++) {
    foreach ($p in $all) { if ($p.ParentProcessId -eq $ids[$k] -and -not $ids.Contains([int]$p.ProcessId)) { $ids.Add([int]$p.ProcessId) } }
  }
  $procs = $ids | ForEach-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue }
  return [ordered]@{ app_mb = [math]::Round((Get-Process -Id $app.Id).WorkingSet64 / 1MB, 1); tree_mb = [math]::Round(($procs | Measure-Object WorkingSet64 -Sum).Sum / 1MB, 1); processes = @($procs).Count }
}

# Paths in the record are relative to the repository (`<repo>\...`), never personal absolute paths.
function RepoPath([string]$p) { if ($p.StartsWith($Repo, [StringComparison]::OrdinalIgnoreCase)) { "<repo>" + $p.Substring($Repo.Length) } else { [System.IO.Path]::GetFileName($p) } }
Log "start" @{ exe = (RepoPath $Exe); show = (RepoPath $Show); pid = $app.Id; sample_s = $SampleSeconds; duration_s = $DurationSeconds }
Press "開始"
$t0 = Get-Date
$next = 10
try {
  while (-not (Test-Path $Stop) -and -not $app.HasExited) {
    $el = ((Get-Date) - $t0).TotalSeconds
    if ($DurationSeconds -gt 0 -and $el -ge $DurationSeconds) { break }
    if ($SampleSeconds -le 0) {
      Log "texts" @(Texts)
    } elseif ($el -ge $next) {
      Log "sample" ([ordered]@{ t_s = [int]$el; memory = (Tree-Memory); texts = @(Texts) })
      $next = if ($next -eq 10) { $SampleSeconds } else { $next + $SampleSeconds }
    }
    if (Test-Path $ShotReq) { Remove-Item $ShotReq; Log "shot" (RepoPath (Shot)) }
    Start-Sleep -Milliseconds $IntervalMs
  }
  if ($SampleSeconds -gt 0 -and -not $app.HasExited) { Log "sample" ([ordered]@{ t_s = [int]((Get-Date) - $t0).TotalSeconds; memory = (Tree-Memory); texts = @(Texts) }) }
  if (-not $app.HasExited) {
    Log "texts" @(Texts)
    Log "shot" (RepoPath (Shot))
    Press "停止"; Start-Sleep -Milliseconds 600
    Log "stopped" @(Texts)
  }
} finally {
  if (-not $app.HasExited) { $app.CloseMainWindow() | Out-Null; if (-not $app.WaitForExit(10000)) { $app.Kill() } }
  Log "end" @{ exit = $app.ExitCode }
}
