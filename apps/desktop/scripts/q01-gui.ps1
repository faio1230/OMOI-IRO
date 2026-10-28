# Q01 GUI checks for OMOI-IRO (Windows, UI Automation). Synthetic only: the
# bridge is live-bridge/sim/fake_bridge.py, LEDs go to the in-process virtual
# Launchpad X, no MIDI port is opened, nothing is sent to Live or a device.
#
# Usage (repository root, after `npx tauri build --debug --no-bundle` and
# `cargo run -p omoiiro-probe -- q01`, which writes the sample show):
#   pwsh apps/desktop/scripts/q01-gui.ps1 -Scenario all
#   pwsh apps/desktop/scripts/q01-gui.ps1 -Scenario soak -SoakSeconds 600
#   2-hour test (M6, NFR07) on real Live / devices with the saved terminal settings:
#   pwsh apps/desktop/scripts/q01-gui.ps1 -Scenario soak -SoakSeconds 7200 -SampleSeconds 300 -BaselineSeconds 1800 -Show <file> -NoFakeBridge
# Scenarios: new-save, reopen, preview, follow, close, soak, all (= all but soak).
# The app's terminal settings directory (%APPDATA%\jp.faio.omoiiro) is backed
# up before and restored after each scenario.
param(
  [ValidateSet("new-save", "reopen", "preview", "follow", "close", "soak", "all")] [string]$Scenario = "all",
  [string]$Repo = (Resolve-Path "$PSScriptRoot\..\..\..").Path,
  [string]$OutDir = "",
  [int]$SoakSeconds = 600,
  # Memory sampling for the soak: interval, and the sample used as the baseline.
  [int]$SampleSeconds = 300,
  [int]$BaselineSeconds = 1800,
  # Show for the soak (default: the Q01 sample) and no synthetic bridge (real Live).
  [string]$Show = "",
  [switch]$NoFakeBridge
)
$ErrorActionPreference = "Stop"
if (-not $OutDir) { $OutDir = Join-Path $Repo "apps\desktop\docs\q01-files" }
New-Item -ItemType Directory -Force $OutDir | Out-Null
$Shots = Join-Path $Repo "apps\desktop\screenshots"
# Build output: CARGO_TARGET_DIR when set (BUILD.md "ディスク容量"), else <repo>	arget.
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Repo "target" }
$Exe = Join-Path $TargetDir "debug\omoiiro.exe"
$Probe = Join-Path $TargetDir "debug\omoiiro-probe.exe"
$Sample = Join-Path $OutDir "q01-sample.show.json"
$Results = Join-Path $OutDir "q01-gui-result.jsonl"

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class Q01W {
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT r, int s);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
$AE = [System.Windows.Automation.AutomationElement]
$TS = [System.Windows.Automation.TreeScope]
$CT = [System.Windows.Automation.ControlType]

function Record($id, $title, $expect, [bool]$ok, $result) {
  $line = [ordered]@{ id = $id; title = $title; expect = $expect; pass = $ok; result = "$result"; t = (Get-Date).ToString("s") } | ConvertTo-Json -Compress
  Add-Content -Path $Results -Value $line -Encoding utf8
  "[{0}] {1} {2}: {3}" -f ($(if ($ok) { "PASS" } else { "FAIL" }), $id, $title, $result)
}

$SettingsDir = Join-Path $env:APPDATA "jp.faio.omoiiro"
function Backup-Settings {
  $script:SettingsExisted = Test-Path $SettingsDir
  $script:SettingsBackup = $null
  if ($script:SettingsExisted) {
    $script:SettingsBackup = Join-Path $env:TEMP "omoiiro-settings-backup-$PID"
    Copy-Item -Recurse -Force $SettingsDir $script:SettingsBackup
  }
}
function Restore-Settings {
  if (Test-Path $SettingsDir) { Remove-Item -Recurse -Force $SettingsDir }
  if ($script:SettingsExisted -and $script:SettingsBackup) { Copy-Item -Recurse -Force $script:SettingsBackup $SettingsDir; Remove-Item -Recurse -Force $script:SettingsBackup }
}

function Start-App([string]$ShowArg) {
  $p = if ($ShowArg) { Start-Process -FilePath $Exe -ArgumentList @("`"$ShowArg`"") -PassThru } else { Start-Process -FilePath $Exe -PassThru }
  $w = $null
  for ($i = 0; $i -lt 100 -and -not $w; $i++) {
    Start-Sleep -Milliseconds 200
    $c = New-Object System.Windows.Automation.AndCondition(
      (New-Object System.Windows.Automation.PropertyCondition($AE::ProcessIdProperty, $p.Id)),
      (New-Object System.Windows.Automation.PropertyCondition($AE::NameProperty, "OMOI-IRO")))
    $w = $AE::RootElement.FindFirst($TS::Children, $c)
  }
  if (-not $w) { throw "OMOI-IRO window not found" }
  $script:App = $p; $script:Win = $w
}
function Stop-App {
  if ($script:App -and -not $script:App.HasExited) {
    $script:App.CloseMainWindow() | Out-Null
    if (-not $script:App.WaitForExit(10000)) { $script:App.Kill() }
  }
}
function Find-Named($name, $type, [int]$tries = 50) {
  $cond = New-Object System.Windows.Automation.AndCondition(
    (New-Object System.Windows.Automation.PropertyCondition($AE::ControlTypeProperty, $type)),
    (New-Object System.Windows.Automation.PropertyCondition($AE::NameProperty, $name)))
  for ($i = 0; $i -lt $tries; $i++) {
    $e = $script:Win.FindFirst($TS::Descendants, $cond)
    if ($e) { return $e }
    Start-Sleep -Milliseconds 200
  }
  return $null
}
function Press($name) {
  for ($i = 0; $i -lt 50; $i++) {
    $b = Find-Named $name $CT::Button 1
    if ($b -and $b.Current.IsEnabled) { $b.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke(); return }
    Start-Sleep -Milliseconds 200
  }
  throw "button '$name' not found or disabled"
}
# Enabled state of the first button whose name starts with $prefix (labels may follow).
function Button-Enabled($prefix) {
  $all = $script:Win.FindAll($TS::Descendants, (New-Object System.Windows.Automation.PropertyCondition($AE::ControlTypeProperty, $CT::Button)))
  foreach ($b in $all) { if ($b.Current.Name.StartsWith($prefix)) { return $b.Current.IsEnabled } }
  return $false
}
function Texts { $script:Win.FindAll($TS::Descendants, (New-Object System.Windows.Automation.PropertyCondition($AE::ControlTypeProperty, $CT::Text))) | ForEach-Object { $_.Current.Name } }
function Has-Text($regex) { return [bool]((Texts) | Where-Object { $_ -match $regex } | Select-Object -First 1) }
function Set-Edit($name, $value) {
  $e = Find-Named $name $CT::Edit
  if (-not $e) { throw "edit '$name' not found" }
  $e.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue($value)
}
function Shot($file) {
  $h = [IntPtr]$script:Win.Current.NativeWindowHandle
  [Q01W]::SetForegroundWindow($h) | Out-Null
  Start-Sleep -Milliseconds 300
  $r = New-Object Q01W+RECT
  [Q01W]::DwmGetWindowAttribute($h, 9, [ref]$r, 16) | Out-Null
  $bmp = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
  $g.Dispose()
  if ($file) { $bmp.Save((Join-Path $Shots $file), [System.Drawing.Imaging.ImageFormat]::Png) }
  return @{ Bitmap = $bmp; Left = $r.L; Top = $r.T }
}
# Windows "save as" dialog of the app: type the path, press Save.
function Fill-SaveDialog($path) {
  $dlg = $null
  for ($i = 0; $i -lt 50 -and -not $dlg; $i++) {
    Start-Sleep -Milliseconds 200
    $c = New-Object System.Windows.Automation.AndCondition(
      (New-Object System.Windows.Automation.PropertyCondition($AE::ProcessIdProperty, $script:App.Id)),
      (New-Object System.Windows.Automation.PropertyCondition($AE::ClassNameProperty, "#32770")))
    $dlg = $AE::RootElement.FindFirst($TS::Descendants, $c)
  }
  if (-not $dlg) { throw "save dialog not found" }
  # The dialog opens with the keyboard focus in the file name box. The box is
  # not reliably reachable through UI Automation on this Windows build, so
  # bring the dialog to the front and type the full path, then Enter.
  Add-Type -AssemblyName System.Windows.Forms
  try { $dlg.SetFocus() } catch {}
  [Q01W]::SetForegroundWindow([IntPtr]$dlg.Current.NativeWindowHandle) | Out-Null
  Start-Sleep -Milliseconds 600
  [System.Windows.Forms.SendKeys]::SendWait("^a")
  [System.Windows.Forms.SendKeys]::SendWait(($path -replace '([+^%~(){}\[\]])', '{$1}'))
  Start-Sleep -Milliseconds 200
  [System.Windows.Forms.SendKeys]::SendWait("{ENTER}")
}

function Scenario-NewSave {
  $target = Join-Path $OutDir "q01-gui-new.show.json"
  if (Test-Path $target) { Remove-Item $target }
  Start-App ""
  try {
    Press "タイムライン"; Press "新規ショー"; Start-Sleep -Milliseconds 400
    $new = Has-Text "新規・未保存"
    Press "レイヤーを追加"; Start-Sleep -Milliseconds 300
    Press "常時レイヤーを追加"; Start-Sleep -Milliseconds 300
    Press "新規プリセット"; Start-Sleep -Milliseconds 300
    $dirty = Has-Text "未保存"
    Record "G1.1" "GUI：新規ショーと編集" "新規（未保存）にレイヤー・常時レイヤー・プリセットを追加できる" ($new -and $dirty) "new $new, dirty $dirty"
    Shot "q01-gui-new.png" | Out-Null
    Press "名前を付けて保存…"
    Fill-SaveDialog $target
    $saved = $false
    for ($i = 0; $i -lt 50 -and -not $saved; $i++) { Start-Sleep -Milliseconds 200; $saved = (Test-Path $target) -and (Has-Text "保存済み") }
    $ok = $saved -and ((& $Probe render --show $target --at 0 --json | ConvertFrom-Json).leds_bottom_left.Count -eq 64)
    Record "G1.2" "GUI：名前を付けて保存（Windows のダイアログ）" "ファイルができ、render で読める、保存済み表示" $ok $target
  } finally { Stop-App }
}

function Scenario-Reopen {
  Start-App $Sample
  try {
    Press "タイムライン"; Press "ショーを読み込んで編集"; Start-Sleep -Milliseconds 500
    $before = (Get-FileHash $Sample).Hash
    $undo = Button-Enabled "元に戻す"
    Press "保存"; Start-Sleep -Milliseconds 600
    $after = (Get-FileHash $Sample).Hash
    Record "G2.1" "GUI：再読込と無変更の保存" "Undo 履歴は空、無変更の保存でバイト一致" ((-not $undo) -and ($before -eq $after)) "undo enabled $undo, hash equal $($before -eq $after)"
  } finally { Stop-App }
}

function Scenario-Preview {
  $tUs = 12345000
  Start-App $Sample
  try {
    Press "ショーを読み込む"; Start-Sleep -Milliseconds 400
    Set-Edit "位置を指定（秒）" ("{0:0.000}" -f ($tUs / 1e6))
    Press "移動"; Start-Sleep -Milliseconds 800
    $cap = Shot "q01-gui-preview.png"
    $grid = Find-Named "LED preview" $CT::Group 10
    if (-not $grid) { $grid = $script:Win.FindFirst($TS::Descendants, (New-Object System.Windows.Automation.PropertyCondition($AE::NameProperty, "LED preview"))) }
    $r = $grid.Current.BoundingRectangle
    # CSS: padding 10, border 1, 8 cells of 42 px with 5 px gaps (styles.css .pads).
    $scale = $r.Width / (2 + 2 * 10 + 8 * 42 + 7 * 5)
    $tempo = 120
    $json = & $Probe render --show $Sample --at $tUs --beat ($tUs / 1e6 * $tempo / 60) --tempo $tempo --json | ConvertFrom-Json
    $worst = 0
    for ($y = 0; $y -lt 8; $y++) {
      for ($x = 0; $x -lt 8; $x++) {
        $row = 7 - $y  # screen row from the top
        $cx = $r.Left + $scale * (1 + 10 + $x * 47 + 21) - $cap.Left
        $cy = $r.Top + $scale * (1 + 10 + $row * 47 + 21) - $cap.Top
        $px = $cap.Bitmap.GetPixel([int]$cx, [int]$cy)
        $hex = $json.leds_bottom_left[$y * 8 + $x]
        $er = [Convert]::ToInt32($hex.Substring(1, 2), 16); $eg = [Convert]::ToInt32($hex.Substring(3, 2), 16); $eb = [Convert]::ToInt32($hex.Substring(5, 2), 16)
        $d = [Math]::Max([Math]::Abs($px.R - $er), [Math]::Max([Math]::Abs($px.G - $eg), [Math]::Abs($px.B - $eb)))
        if ($d -gt $worst) { $worst = $d }
      }
    }
    $cap.Bitmap.Dispose()
    Record "G3.1" "GUI：任意位置プレビューの画素 = render" "64パッドの画面の色が render の RGB と±3以内（7bit 量子化と描画）" ($worst -le 3) ("t={0} us, 最大差 {1}" -f $tUs, $worst)
  } finally { Stop-App }
}

function Scenario-Follow {
  Start-App $Sample
  $bridge = $null
  try {
    Press "開始"; Start-Sleep -Milliseconds 400
    $bridge = Start-Process -FilePath python -ArgumentList "`"$Repo\live-bridge\sim\fake_bridge.py`" --steady 25 --start-ms 4000" -PassThru -WindowStyle Hidden
    Start-Sleep -Seconds 3
    $locked = Has-Text "^locked$"
    Press "タイムライン"; Press "ショーを読み込んで編集"; Start-Sleep -Milliseconds 400
    Press "レイヤーを追加"; Start-Sleep -Milliseconds 800
    $adopted = Has-Text "Runtime に反映"
    Shot "q01-gui-follow-edit.png" | Out-Null
    Press "演奏"
    Press "BLACKOUT"; Start-Sleep -Milliseconds 600
    $blk = Has-Text "blackout"
    Shot "q01-gui-follow-blackout.png" | Out-Null
    Press "BLACKOUT 中（解除）"; Start-Sleep -Milliseconds 400
    Press "全ポートを再接続"; Start-Sleep -Milliseconds 800
    Shot "q01-gui-follow.png" | Out-Null
    Record "G4.1" "GUI：追従、編集の反映、Blackout、再接続" "locked、編集が rev 2 で反映、Blackout の暗転理由、再接続後も動作" ($locked -and $adopted -and $blk) "locked $locked, rev2 $adopted, blackout $blk"
    Press "停止"; Start-Sleep -Milliseconds 600
  } finally {
    if ($bridge -and -not $bridge.HasExited) { $bridge.Kill() }
    # The edit above is unsaved: discard it on close.
    if ($script:App -and -not $script:App.HasExited) {
      $script:App.CloseMainWindow() | Out-Null; Start-Sleep -Milliseconds 800
      try { Press "保存せずに終了" } catch {}
      if (-not $script:App.WaitForExit(10000)) { $script:App.Kill() }
    }
  }
}

function Scenario-Close {
  $hash = (Get-FileHash $Sample).Hash
  Start-App $Sample
  try {
    Press "タイムライン"; Press "ショーを読み込んで編集"; Start-Sleep -Milliseconds 400
    Press "レイヤーを追加"; Start-Sleep -Milliseconds 400
    $script:App.CloseMainWindow() | Out-Null; Start-Sleep -Milliseconds 900
    $asked = (-not $script:App.HasExited) -and (Has-Text "未保存の変更があります")
    Shot "q01-gui-close.png" | Out-Null
    Press "キャンセル"; Start-Sleep -Milliseconds 600
    $stayed = -not $script:App.HasExited
    $script:App.CloseMainWindow() | Out-Null; Start-Sleep -Milliseconds 900
    Press "保存せずに終了"
    $exited = $script:App.WaitForExit(10000)
    $unchanged = (Get-FileHash $Sample).Hash -eq $hash
    Record "G5.1" "GUI：未保存で閉じる → キャンセル" "確認が出る、キャンセルで残る、保存せずに終了でファイル不変" ($asked -and $stayed -and $exited -and $unchanged) "asked $asked, stayed $stayed, exited $exited, file unchanged $unchanged"
  } finally { Stop-App }
}

# omoiiro.exe and every descendant process (WebView2 browser, renderers, GPU ...).
function Get-TreeIds([int]$RootId) {
  $all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId
  $ids = New-Object System.Collections.Generic.List[int]
  $ids.Add($RootId)
  for ($k = 0; $k -lt $ids.Count; $k++) {
    foreach ($p in $all) { if ($p.ParentProcessId -eq $ids[$k] -and -not $ids.Contains([int]$p.ProcessId)) { $ids.Add([int]$p.ProcessId) } }
  }
  return $ids
}
function Get-TreeWorkingSet([int]$RootId) {
  $ids = Get-TreeIds $RootId
  $procs = $ids | ForEach-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue }
  return @{ Total = ($procs | Measure-Object WorkingSet64 -Sum).Sum; App = (Get-Process -Id $RootId).WorkingSet64; Count = @($procs).Count }
}

# NFR07 memory criterion (design, 2026-10-28): from the baseline sample to the
# end, growth <= 10 % and <= +50 MB, and no monotonic rise (see below).
function Judge-Memory($samples, [int]$baselineSeconds) {
  $base = $samples | Where-Object { $_.t_s -ge $baselineSeconds } | Select-Object -First 1
  $note = ""
  if (-not $base) { $base = $samples[0]; $note = "（基準 $baselineSeconds 秒に届かないので最初の標本を基準にした）" }
  $after = @($samples | Where-Object { $_.t_s -ge $base.t_s })
  $last = $after[-1]
  $growth = $last.tree_mb - $base.tree_mb
  $ratio = if ($base.tree_mb -gt 0) { $growth / $base.tree_mb } else { 0 }
  # A monotonic trend: every sample higher than the one before AND more than
  # 1 % in total (sub-1 % wobble is measurement noise, not a leak).
  $monotonic = $false
  if ($after.Count -ge 4 -and $ratio -gt 0.01) {
    $monotonic = $true
    for ($k = 1; $k -lt $after.Count; $k++) { if ($after[$k].tree_mb -le $after[$k - 1].tree_mb) { $monotonic = $false } }
  }
  $ok = ($ratio -le 0.10) -and ($growth -le 50) -and (-not $monotonic)
  return @{ Ok = $ok; Text = ("基準 {0:N1} MB（{1} s）→ 最後 {2:N1} MB（{3} s）、増加 {4:+0.0;-0.0} MB（{5:P1}）、単調増加 {6}{7}" -f $base.tree_mb, $base.t_s, $last.tree_mb, $last.t_s, $growth, $ratio, $monotonic, $note) }
}

function Scenario-Soak {
  $showFile = if ($Show) { $Show } else { $Sample }
  $samplesFile = Join-Path $OutDir ("soak-samples-{0}.jsonl" -f (Get-Date -Format "yyyyMMdd-HHmmss"))
  Start-App $showFile
  $bridge = $null
  try {
    Press "開始"; Start-Sleep -Milliseconds 400
    if (-not $NoFakeBridge) {
      $bridge = Start-Process -FilePath python -ArgumentList "`"$Repo\live-bridge\sim\fake_bridge.py`" --steady $($SoakSeconds + 30) --start-ms 0" -PassThru -WindowStyle Hidden
    }
    $t0 = Get-Date
    $samples = @()
    $next = 10
    while ($true) {
      $el = ((Get-Date) - $t0).TotalSeconds
      if ($el -ge $next -or $el -ge $SoakSeconds) {
        $m = Get-TreeWorkingSet $script:App.Id
        $row = [ordered]@{ t_s = [int]$el; app_mb = [math]::Round($m.App / 1MB, 1); tree_mb = [math]::Round($m.Total / 1MB, 1); processes = $m.Count }
        Add-Content -Path $samplesFile -Value ($row | ConvertTo-Json -Compress) -Encoding utf8
        $samples += [pscustomobject]$row
        "  t={0,5} s  omoiiro.exe {1,7:N1} MB  tree {2,7:N1} MB ({3} processes)" -f $row.t_s, $row.app_mb, $row.tree_mb, $row.processes
        if ($el -ge $SoakSeconds) { break }
        $next = if ($next -eq 10) { $SampleSeconds } else { $next + $SampleSeconds }
        if ($next -gt $SoakSeconds) { $next = $SoakSeconds }
      }
      if ($script:App.HasExited) { throw "the app exited during the soak" }
      Start-Sleep -Seconds 1
    }
    Shot "q01-gui-soak.png" | Out-Null
    $verdict = Judge-Memory $samples $BaselineSeconds
    Record "G6.1" "GUI：メモリー（NFR07）" ("{0} 秒、{1} 秒ごとに子孫プロセス込みの working set。基準から増加 10% 以内かつ +50 MB 以内、単調増加なし" -f $SoakSeconds, $SampleSeconds) $verdict.Ok ($verdict.Text + "; 標本 " + $samplesFile)
    Press "停止"
  } finally {
    if ($bridge -and -not $bridge.HasExited) { $bridge.Kill() }
    Stop-App
  }
}

if (-not (Test-Path $Exe)) { throw "build the app first: $Exe" }
if (-not $Show -and -not (Test-Path $Sample)) { throw "run 'omoiiro-probe q01' first: $Sample" }
Backup-Settings
try {
  $list = if ($Scenario -eq "all") { "new-save", "reopen", "preview", "follow", "close" } else { @($Scenario) }
  foreach ($s in $list) {
    switch ($s) {
      "new-save" { Scenario-NewSave }
      "reopen" { Scenario-Reopen }
      "preview" { Scenario-Preview }
      "follow" { Scenario-Follow }
      "close" { Scenario-Close }
      "soak" { Scenario-Soak }
    }
  }
} finally { Restore-Settings }
