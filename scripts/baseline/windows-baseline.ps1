<#
.SYNOPSIS
  xReveal first Windows baseline (wayfinder ticket pwn233/xReveal#194). Throwaway, not the real sampler.

.DESCRIPTION
  Launches xReveal and samples it with typeperf every 5 s (no admin). Timeline:
    0-20 min   Windowed, nobody touching the machine (the soak). Press F3 for the stats overlay
               in the first 15 s, then hands off.
    20 min     The script beeps and asks you to press F11 in xReveal (Fullscreen).
    20-25 min  Fullscreen: 2 min settling, 3 min measured.
  The Windowed baseline is minutes 17-20 of the soak, and the Fullscreen baseline is the last 3 minutes.
  The folder it writes has samples.csv (raw), meta.json and summary.md. Post summary.md on the ticket.

.PARAMETER Exe
  Path to xreveal.exe. Defaults to the installed copy.
#>
param(
    [string]$Exe = "$env:LOCALAPPDATA\Programs\xReveal\xreveal.exe",
    [int]$SoakMinutes = 20,
    [int]$FullscreenSettleMinutes = 2,
    [int]$FullscreenMeasureMinutes = 3,
    [int]$IntervalSeconds = 5
)
$ErrorActionPreference = 'Stop'

# --- Win32: physical pixels (per-monitor DPI aware), the windows of a process, monitor bounds and refresh rates
Add-Type -TypeDefinition @'
using System; using System.Collections.Generic; using System.Runtime.InteropServices; using System.Text;
public static class XrWin {
  [DllImport("user32.dll")] static extern bool SetProcessDpiAwarenessContext(IntPtr v);
  public static void DpiAware() { try { SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch {} }
  delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] static extern IntPtr MonitorFromWindow(IntPtr h, uint f);
  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)] public struct MONITORINFOEX {
    public int cbSize; public RECT rcMonitor; public RECT rcWork; public uint dwFlags;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string szDevice; }
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern bool GetMonitorInfo(IntPtr m, ref MONITORINFOEX i);
  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)] public struct DEVMODE {
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string dmDeviceName;
    public short dmSpecVersion, dmDriverVersion, dmSize, dmDriverExtra; public int dmFields;
    public int dmPositionX, dmPositionY, dmDisplayOrientation, dmDisplayFixedOutput;
    public short dmColor, dmDuplex, dmYResolution, dmTTOption, dmCollate;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string dmFormName;
    public short dmLogPixels; public int dmBitsPerPel, dmPelsWidth, dmPelsHeight, dmDisplayFlags, dmDisplayFrequency; }
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern bool EnumDisplaySettings(string d, int m, ref DEVMODE dm);
  public static string Hz(string device) { var dm = new DEVMODE(); dm.dmSize = (short)Marshal.SizeOf(typeof(DEVMODE));
    return EnumDisplaySettings(device, -1, ref dm) ? dm.dmPelsWidth + "x" + dm.dmPelsHeight + " @ " + dm.dmDisplayFrequency + " Hz" : "?"; }
  public static List<string> Windows(uint pid, out bool fullscreen) {
    var list = new List<string>(); bool fs = false;
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p);
      if (p != pid || !IsWindowVisible(h)) return true;
      RECT r; GetWindowRect(h, out r); var sb = new StringBuilder(256); GetWindowText(h, sb, 256);
      var mi = new MONITORINFOEX(); mi.cbSize = Marshal.SizeOf(typeof(MONITORINFOEX));
      GetMonitorInfo(MonitorFromWindow(h, 2), ref mi); var m = mi.rcMonitor;
      bool full = r.L <= m.L && r.T <= m.T && r.R >= m.R && r.B >= m.B; if (full) fs = true;
      list.Add(string.Format("'{0}' {1}x{2} at ({3},{4}) on {5} [{6}]{7}", sb, r.R - r.L, r.B - r.T, r.L, r.T,
        mi.szDevice, Hz(mi.szDevice), full ? " = whole monitor" : ""));
      return true; }, IntPtr.Zero);
    fullscreen = fs; return list; }
}
'@
[XrWin]::DpiAware()
Add-Type -AssemblyName System.Windows.Forms

function Get-XrWindows([int]$ProcessId) { $fs = $false; $w = [XrWin]::Windows([uint32]$ProcessId, [ref]$fs); [pscustomobject]@{ Fullscreen = $fs; Windows = @($w) } }
function Beep { 1..3 | ForEach-Object { [console]::Beep(880, 250); Start-Sleep -Milliseconds 150 } }

if (-not (Test-Path $Exe)) { throw "xreveal.exe not found at $Exe. Pass -Exe <path>." }
if (Get-Process xreveal -ErrorAction SilentlyContinue) { throw "xReveal is already running. Quit it first: the soak must start from launch." }

$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$out = Join-Path (Get-Location) "xreveal-baseline-$stamp"
New-Item -ItemType Directory $out | Out-Null

# --- The machine
$cv = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
$meta = [ordered]@{
    windows   = "$($cv.ProductName) $($cv.DisplayVersion), build $($cv.CurrentBuild).$($cv.UBR)"
    cpu       = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name.Trim()
    logicalCpus = [Environment]::ProcessorCount
    gpus      = @(Get-CimInstance Win32_VideoController | ForEach-Object { "$($_.Name) (driver $($_.DriverVersion))" })
    ramGB     = [math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB, 1)
    monitors  = @([System.Windows.Forms.Screen]::AllScreens | ForEach-Object { "$($_.DeviceName) $([XrWin]::Hz($_.DeviceName))$(if ($_.Primary) {' (primary)'})" })
    # The exe has no version resource; the installer records its version in the Uninstall key.
    xreveal   = "$((Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\xReveal' -ErrorAction SilentlyContinue).DisplayVersion) (exe built $((Get-Item $Exe).LastWriteTime.ToString('yyyy-MM-dd HH:mm')))"
    exe       = $Exe
    intervalSeconds = $IntervalSeconds
}

# --- Process V2 (instance "xreveal:<pid>") where it exists, else the old Process set
$hasV2 = (typeperf -qx "Process V2" 2>$null | Select-Object -First 1) -match 'Process V2'
$meta.processSet = if ($hasV2) { 'Process V2' } else { 'Process' }

Write-Host "`nLaunching xReveal. Press F3 in its window for the stats overlay within 15 s, then don't touch the machine for $SoakMinutes minutes." -ForegroundColor Cyan
$proc = Start-Process $Exe -PassThru
$launch = Get-Date
Start-Sleep -Seconds 10   # the picture is up within 1 s; GPU engine instances exist once it has drawn
$xrPid = $proc.Id
$inst = if ($hasV2) { "xreveal:$xrPid" } else { 'xreveal' }
$pset = $meta.processSet
$counters = @(
    "\$pset($inst)\% Processor Time",
    "\$pset($inst)\Working Set - Private",
    "\$pset($inst)\Private Bytes",
    "\$pset($inst)\Working Set",
    "\$pset($inst)\Handle Count",
    "\$pset($inst)\Thread Count",
    "\GPU Engine(pid_${xrPid}_*)\Utilization Percentage",
    "\GPU Process Memory(pid_${xrPid}_*)\Dedicated Usage",
    "\GPU Process Memory(pid_${xrPid}_*)\Shared Usage",
    "\Processor Information(_Total)\% Processor Time",
    "\Processor Information(_Total)\% Processor Utility"
)
$cf = Join-Path $out 'counters.txt'; $counters | Set-Content $cf -Encoding ascii
$csv = Join-Path $out 'samples.csv'
$totalSec = $SoakMinutes * 60 + ($FullscreenSettleMinutes + $FullscreenMeasureMinutes) * 60 + 6 * 60   # + time to press F11
$count = [math]::Ceiling($totalSec / $IntervalSeconds)
$tp = Start-Process typeperf -ArgumentList @('-cf', "`"$cf`"", '-si', $IntervalSeconds, '-sc', $count, '-f', 'CSV', '-o', "`"$csv`"", '-y') -PassThru -WindowStyle Hidden
$sampleStart = Get-Date
$meta.windowsAtStart = (Get-XrWindows $xrPid).Windows
if ((Get-XrWindows $xrPid).Fullscreen) { Write-Warning "xReveal opened in Fullscreen. Press F11 now to go Windowed; the first minute is affected (noted in the summary)."; $meta.openedFullscreen = $true }

# --- Soak, Windowed
$soakEnd = $launch.AddMinutes($SoakMinutes)
while ((Get-Date) -lt $soakEnd) {
    if ($tp.HasExited) { throw "typeperf stopped early (exit $($tp.ExitCode)). Check $csv and the counter names in $cf." }
    if ($proc.HasExited) { throw "xReveal exited during the soak (exit $($proc.ExitCode))." }
    $left = $soakEnd - (Get-Date)
    Write-Host ("`rSoak: {0:mm\:ss} left   " -f $left) -NoNewline
    Start-Sleep -Seconds 5
}
$meta.windowsWindowed = (Get-XrWindows $xrPid).Windows

# --- Fullscreen
Write-Host "`n"; Beep
Write-Host "Press F11 in xReveal now (click its picture first if it isn't focused). Waiting for Fullscreen..." -ForegroundColor Yellow
$fsAsk = Get-Date
while (-not (Get-XrWindows $xrPid).Fullscreen) {
    if (((Get-Date) - $fsAsk).TotalMinutes -gt 5) { throw "xReveal never covered its monitor. Was F11 pressed?" }
    Start-Sleep -Milliseconds 500
}
$fsStart = Get-Date
$meta.windowsFullscreen = (Get-XrWindows $xrPid).Windows
Write-Host "Fullscreen. Hands off for $($FullscreenSettleMinutes + $FullscreenMeasureMinutes) minutes." -ForegroundColor Cyan
$fsEnd = $fsStart.AddMinutes($FullscreenSettleMinutes + $FullscreenMeasureMinutes)
while ((Get-Date) -lt $fsEnd) {
    if ($proc.HasExited) { throw "xReveal exited in Fullscreen (exit $($proc.ExitCode))." }
    Write-Host ("`rFullscreen: {0:mm\:ss} left   " -f ($fsEnd - (Get-Date))) -NoNewline
    Start-Sleep -Seconds 5
}
Start-Sleep -Seconds ($IntervalSeconds * 2); Stop-Process $tp -ErrorAction SilentlyContinue; Start-Sleep -Seconds 1
Write-Host "`n"; Beep

# --- Read the samples. typeperf's first column is its timestamp ("MM/dd/yyyy HH:mm:ss.fff" on Windows 11);
# its interval drifts ~14 ms per sample, so row i is NOT sampleStart + i * interval. That is only the fallback.
$rows = @(Import-Csv $csv)
$stampCol = $rows[0].PSObject.Properties.Name | Select-Object -First 1
$meta.stampFallbacks = 0
function Stamp($s, [int]$i) { $d = [datetime]::MinValue
    if ([datetime]::TryParseExact("$s".Trim(), 'MM/dd/yyyy HH:mm:ss.fff', [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]::None, [ref]$d)) { return $d }
    if ([datetime]::TryParse("$s".Trim(), [ref]$d)) { return $d }
    $script:meta.stampFallbacks++; $sampleStart.AddSeconds($i * $IntervalSeconds) }
$meta.rows = $rows.Count
if ($rows.Count -lt ($SoakMinutes + $FullscreenSettleMinutes + $FullscreenMeasureMinutes) * 60 / $IntervalSeconds) { Write-Warning "Only $($rows.Count) samples in $csv; the run may have been cut short." }
$cols = $rows[0].PSObject.Properties.Name
function ColsLike([string]$re) { @($cols | Where-Object { $_ -match $re }) }
function Num($v) { $d = 0.0; if ([double]::TryParse(("$v").Trim(), [Globalization.NumberStyles]::Float, [Globalization.CultureInfo]::InvariantCulture, [ref]$d)) { $d } else { [double]::NaN } }
$c = @{
    cpu = ColsLike '\\% Processor Time$' | Where-Object { $_ -match 'xreveal' } | Select-Object -First 1
    wsp = ColsLike 'Working Set - Private$' | Select-Object -First 1
    pb  = ColsLike 'Private Bytes$' | Select-Object -First 1
    ws  = ColsLike '\)\\Working Set$' | Select-Object -First 1
    h   = ColsLike 'Handle Count$' | Select-Object -First 1
    t   = ColsLike 'Thread Count$' | Select-Object -First 1
    cpuMachine = ColsLike 'Processor Information\(_Total\)\\% Processor Time$' | Select-Object -First 1
    utilMachine = ColsLike '% Processor Utility$' | Select-Object -First 1
}
$gpuCols = ColsLike 'GPU Engine.*Utilization Percentage$'
$dedCols = ColsLike 'Dedicated Usage$'; $shCols = ColsLike 'Shared Usage$'
$meta.gpuEngineInstances = $gpuCols.Count
if (-not $gpuCols.Count) { Write-Warning "No GPU Engine columns in ${csv}: GPU % will be n/a." }
if (-not $dedCols.Count) { Write-Warning "No GPU Process Memory columns in ${csv}: GPU memory will be n/a." }
$n = $logical = $meta.logicalCpus
$samples = for ($i = 0; $i -lt $rows.Count; $i++) {
    $r = $rows[$i]
    $gpuVals = @($gpuCols | ForEach-Object { Num $r.$_ } | Where-Object { -not [double]::IsNaN($_) })
    [pscustomobject]@{
        t     = Stamp $r.$stampCol $i
        cpu   = (Num $r.($c.cpu)) / $logical
        gpu   = if ($gpuVals.Count) { ($gpuVals | Measure-Object -Maximum).Maximum } else { [double]::NaN }
        wspMB = (Num $r.($c.wsp)) / 1MB
        pbMB  = (Num $r.($c.pb)) / 1MB
        wsMB  = (Num $r.($c.ws)) / 1MB
        dedMB = if ($dedCols.Count) { (@($dedCols | ForEach-Object { Num $r.$_ }) | Measure-Object -Sum).Sum / 1MB } else { [double]::NaN }
        shMB  = if ($shCols.Count) { (@($shCols | ForEach-Object { Num $r.$_ }) | Measure-Object -Sum).Sum / 1MB } else { [double]::NaN }
        handles = Num $r.($c.h)
        threads = Num $r.($c.t)
        cpuMachine = Num $r.($c.cpuMachine)
        utilMachine = if ($c.utilMachine) { Num $r.($c.utilMachine) } else { [double]::NaN }
    }
}
# Task Manager's CPU column counts Processor Utility (frequency-scaled), not Processor Time. Estimate it by
# scaling the process's share by the machine's Utility / Time ratio in the same sample.
foreach ($s in $samples) { $s | Add-Member cpuUtil $(if ($s.cpuMachine -gt 0) { $s.cpu * $s.utilMachine / $s.cpuMachine } else { [double]::NaN }) }
function Median([double[]]$v) { $v = @($v | Where-Object { -not [double]::IsNaN($_) } | Sort-Object); if (-not $v.Count) { return [double]::NaN }
    $m = [int][math]::Floor($v.Count / 2); if ($v.Count % 2) { $v[$m] } else { ($v[$m - 1] + $v[$m]) / 2 } }
function P95([double[]]$v) { $v = @($v | Where-Object { -not [double]::IsNaN($_) } | Sort-Object); if (-not $v.Count) { return [double]::NaN }; $v[[int][math]::Min($v.Count - 1, [math]::Ceiling(0.95 * $v.Count) - 1)] }
function Between($from, $to) { @($samples | Where-Object { $_.t -ge $from -and $_.t -lt $to }) }
$fields = 'cpu', 'cpuUtil', 'gpu', 'wspMB', 'pbMB', 'wsMB', 'dedMB', 'shMB', 'handles', 'threads', 'cpuMachine', 'utilMachine'
function Phase($s) { $o = [ordered]@{ n = $s.Count }; foreach ($f in $fields) { $o[$f] = Median ($s.$f); $o["${f}P95"] = P95 ($s.$f) }; [pscustomobject]$o }

$win = Phase @(Between $launch.AddMinutes($SoakMinutes - 3) $soakEnd)
$fsMeasureFrom = $fsStart.AddMinutes($FullscreenSettleMinutes)
$full = Phase @(Between $fsMeasureFrom $fsEnd)

# --- The soak: 1-minute medians from launch
$minutes = for ($m = 0; $m -lt $SoakMinutes; $m++) {
    $s = @(Between $launch.AddMinutes($m) $launch.AddMinutes($m + 1))
    [pscustomobject]@{ min = $m + 1; n = $s.Count; pbMB = Median ($s.pbMB); wspMB = Median ($s.wspMB); dedMB = Median ($s.dedMB); handles = Median ($s.handles); threads = Median ($s.threads); cpu = Median ($s.cpu); gpu = Median ($s.gpu) }
}
# Least-squares line through minutes 11-20 (after the 10-minute warm-up); sigma = spread of the 1-min medians around it
function Fit($ys) { $ys = @($ys); if ($ys.Count -lt 3) { return [pscustomobject]@{ slopePerHour = [double]::NaN; sigma = [double]::NaN; range = [double]::NaN } }
    $xs = 1..$ys.Count; $mx = ($xs | Measure-Object -Average).Average; $my = ($ys | Measure-Object -Average).Average
    $sxy = 0.0; $sxx = 0.0; for ($i = 0; $i -lt $ys.Count; $i++) { $sxy += ($xs[$i] - $mx) * ($ys[$i] - $my); $sxx += ($xs[$i] - $mx) * ($xs[$i] - $mx) }
    $b = $sxy / $sxx; $a = $my - $b * $mx; $ss = 0.0; for ($i = 0; $i -lt $ys.Count; $i++) { $e = $ys[$i] - ($a + $b * $xs[$i]); $ss += $e * $e }
    $sd = [math]::Sqrt($ss / [math]::Max(1, $ys.Count - 2))
    [pscustomobject]@{ slopePerHour = $b * 60; sigma = $sd; range = (($ys | Measure-Object -Maximum).Maximum - ($ys | Measure-Object -Minimum).Minimum) } }
$tail = @($minutes | Where-Object { $_.min -gt 10 })
$fitPb = Fit @($tail.pbMB); $fitWsp = Fit @($tail.wspMB); $fitDed = Fit @($tail.dedMB)
$firstHalf = @($minutes | Where-Object { $_.min -le 10 })
$settled = ([math]::Abs($fitPb.slopePerHour) -lt 6) -and ([math]::Abs(($tail.pbMB | Select-Object -First 1) - ($tail.pbMB | Select-Object -Last 1)) -lt 3)

# --- summary.md
$f1 = { param($v) if ([double]::IsNaN($v)) { 'n/a' } else { '{0:N1}' -f $v } }
$f2 = { param($v) if ([double]::IsNaN($v)) { 'n/a' } else { '{0:N2}' -f $v } }
$L = [Collections.Generic.List[string]]::new()
$L.Add("# xReveal Windows baseline, $stamp"); $L.Add('')
$L.Add("- **Windows:** $($meta.windows)  ")
$L.Add("- **CPU:** $($meta.cpu), $($meta.logicalCpus) logical; **RAM:** $($meta.ramGB) GB  ")
$L.Add("- **GPU:** $($meta.gpus -join '; ')  ")
$L.Add("- **Monitors:** $($meta.monitors -join '; ')  ")
$L.Add("- **xReveal:** $($meta.xreveal); counters from ``$($meta.processSet)``, $($meta.gpuEngineInstances) GPU engine instances, every $IntervalSeconds s")
$L.Add("- **Windowed:** $($meta.windowsWindowed -join '; ')  ")
$L.Add("- **Fullscreen:** $($meta.windowsFullscreen -join '; ')")
if ($meta.openedFullscreen) { $L.Add('- Note: xReveal opened in Fullscreen and was switched to Windowed by hand in the first minute.') }
if ($meta.stampFallbacks) { $L.Add("- Note: $($meta.stampFallbacks) of $($rows.Count) typeperf timestamps didn't parse; those rows were placed by index.") }
$L.Add(''); $L.Add('## Budgets (medians; p95 in brackets)'); $L.Add('')
$L.Add('| | Windowed (soak min 18-20) | Fullscreen (last 3 min) | Budget |'); $L.Add('|---|---|---|---|')
$row = { param($label, $f, $fmt, $unit, $budget) $L.Add("| $label | $(& $fmt $win.$f)$unit ($(& $fmt $win."${f}P95")) | $(& $fmt $full.$f)$unit ($(& $fmt $full."${f}P95")) | $budget |") }
& $row 'CPU, share of the machine' 'cpu' $f2 '%' '<= 5%'
& $row 'CPU, scaled to Processor Utility (Task Manager basis, estimate)' 'cpuUtil' $f2 '%' ''
& $row 'GPU, process (max engine)' 'gpu' $f1 '%' '<= 10%'
& $row 'Memory: private working set (Task Manager "Memory")' 'wspMB' $f1 ' MB' '<= 100 MB'
& $row 'Private Bytes (Task Manager "Commit size")' 'pbMB' $f1 ' MB' ''
& $row 'Working set' 'wsMB' $f1 ' MB' ''
& $row 'Dedicated GPU memory' 'dedMB' $f1 ' MB' ''
& $row 'Shared GPU memory' 'shMB' $f1 ' MB' ''
& $row 'Handles' 'handles' $f1 '' ''
& $row 'Threads' 'threads' $f1 '' ''
& $row 'Whole machine CPU (% Processor Time)' 'cpuMachine' $f1 '%' ''
& $row 'Whole machine CPU (% Processor Utility)' 'utilMachine' $f1 '%' ''
$L.Add(''); $L.Add('## Soak, Windowed: 1-minute medians from launch'); $L.Add('')
$L.Add('| min | Private Bytes MB | Private WS MB | Dedicated GPU MB | Handles | Threads | CPU % | GPU % |'); $L.Add('|---|---|---|---|---|---|---|---|')
foreach ($m in $minutes) { $L.Add("| $($m.min) | $(& $f1 $m.pbMB) | $(& $f1 $m.wspMB) | $(& $f1 $m.dedMB) | $(& $f1 $m.handles) | $(& $f1 $m.threads) | $(& $f2 $m.cpu) | $(& $f1 $m.gpu) |") }
$L.Add(''); $L.Add('## Minutes 11-20 (after the warm-up)'); $L.Add('')
$L.Add('| | Slope | Wobble (sigma) of 1-min medians around the line | Max - min |'); $L.Add('|---|---|---|---|')
$L.Add("| Private Bytes | $(& $f2 $fitPb.slopePerHour) MB/h | $(& $f2 $fitPb.sigma) MB | $(& $f2 $fitPb.range) MB |")
$L.Add("| Private working set | $(& $f2 $fitWsp.slopePerHour) MB/h | $(& $f2 $fitWsp.sigma) MB | $(& $f2 $fitWsp.range) MB |")
$L.Add("| Dedicated GPU memory | $(& $f2 $fitDed.slopePerHour) MB/h | $(& $f2 $fitDed.sigma) MB | $(& $f2 $fitDed.range) MB |")
$L.Add('')
$L.Add("- **Settled by minute 10 (Private Bytes):** $(if ($settled) { 'yes' } else { 'NO: look at the table' }) (rule: slope under 6 MB/h and minutes 11 and 20 within 3 MB).")
$L.Add("- **2-hour soak assumption (sigma < ~1.6 MB):** Private Bytes sigma = $(& $f2 $fitPb.sigma) MB -> $(if ($fitPb.sigma -lt 1.6) { 'holds' } else { 'DOES NOT HOLD' }).")
$L.Add('- Ten points is a rough sigma; it is a check of the assumption, not a leak verdict.')
$L | Set-Content (Join-Path $out 'summary.md') -Encoding utf8
$meta | ConvertTo-Json -Depth 4 | Set-Content (Join-Path $out 'meta.json') -Encoding utf8

Write-Host "Done. xReveal is still running in Fullscreen." -ForegroundColor Green
Write-Host "Results: $out\summary.md"
Write-Host "Last: open Task Manager > Details, add the columns CPU, GPU, Memory (private working set), Commit size,"
Write-Host "Dedicated GPU memory and Shared GPU memory, and screenshot xreveal.exe's row. Then quit xReveal."
