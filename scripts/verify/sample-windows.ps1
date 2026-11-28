<#
.SYNOPSIS
  Samples xReveal on Windows every 5 s into a CSV (#207, #191). No admin.

.DESCRIPTION
  Start it before xReveal. It waits for xreveal.exe (or takes -ProcessId), samples it with typeperf on
  Process V2 every 5 s, and keeps going after xReveal exits: the rows say so, and when it is waiting by
  name it picks up the next xreveal.exe that starts. Ctrl+C (or -Minutes) stops it and writes summary.md.

  Per sample (samples.csv; MB are MiB, as Task Manager counts them):
    cpu_pct            Process V2 % Processor Time / logical CPUs: the share of the whole machine.
    cpu_utility_pct    the same scaled by the machine's Processor Utility / Processor Time (an estimate of
                       the frequency-scaled CPU Utility column).
    gpu_pct            the max over the process's GPU Engine(pid_*) instances: Task Manager's GPU column.
    private_ws_mb      Working Set - Private: Task Manager's Memory.
    private_bytes_mb   Private Bytes: Task Manager's Commit size, the one to watch for leaks.
    working_set_mb, handles, threads
    gpu_dedicated_mb, gpu_shared_mb   GPU Process Memory(pid_*) Dedicated and Shared Usage.
    machine_cpu_pct, machine_utility_pct   the whole machine's % Processor Time and % Processor Utility.
    mode, window, display   fullscreen when one of its windows covers its monitor, windowed when one is
                       visible, none when it is minimised or hidden. Physical pixels.
  The folder also keeps typeperf's raw CSVs (typeperf-<pid>-<n>.csv, with counters-<pid>-<n>.txt),
  windows.csv (the window state every 5 s) and meta.json (the machine, the monitors, setup.toml).
  typeperf fixes its GPU engine instances when it starts, so the script starts it 10 s after xReveal
  (once it has drawn), checks the instances every minute, and starts a new typeperf if more appear.

.PARAMETER ProcessId
  Sample this process. Default: wait for -Name.

.PARAMETER Name
  The process to wait for, without .exe. Default: xreveal.

.PARAMETER Minutes
  Stop after this many minutes. Default: until Ctrl+C.

.PARAMETER Out
  Output folder. Default: .\xreveal-samples-<time>.

.PARAMETER Analyze
  A folder from an earlier run: rebuild samples.csv from its raw files and write summary.md and
  minutes.csv again. scripts/verify/analyze.py writes the same summary from the same folder.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\sample-windows.ps1 -Minutes 30
#>
param(
    [int]$ProcessId = 0,
    [string]$Name = 'xreveal',
    [double]$Minutes = 0,
    [int]$IntervalSeconds = 5,
    [string]$Out,
    [string]$Analyze
)
$ErrorActionPreference = 'Stop'
$inv = [Globalization.CultureInfo]::InvariantCulture

# --- Win32: physical pixels (per-monitor DPI aware), a process's visible windows, monitor bounds and modes
Add-Type -TypeDefinition @'
using System; using System.Collections.Generic; using System.Runtime.InteropServices; using System.Text;
public static class XrWin {
  [DllImport("user32.dll")] static extern bool SetProcessDpiAwarenessContext(IntPtr v);
  public static void DpiAware() { try { SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch {} }
  delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern bool IsIconic(IntPtr h);
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
  // mode|window|display for the process: fullscreen if a visible window covers its monitor; else the largest.
  public static string State(uint pid) {
    string mode = "none", window = "", display = ""; long best = -1;
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p);
      if (p != pid || !IsWindowVisible(h) || IsIconic(h)) return true;
      RECT r; GetWindowRect(h, out r);
      var mi = new MONITORINFOEX(); mi.cbSize = Marshal.SizeOf(typeof(MONITORINFOEX));
      GetMonitorInfo(MonitorFromWindow(h, 2), ref mi); var m = mi.rcMonitor;
      bool full = r.L <= m.L && r.T <= m.T && r.R >= m.R && r.B >= m.B;
      long area = (long)(r.R - r.L) * (r.B - r.T);
      if ((full && mode != "fullscreen") || (full == (mode == "fullscreen") && area > best)) {
        mode = full ? "fullscreen" : "windowed"; best = area;
        window = (r.R - r.L) + "x" + (r.B - r.T) + " at (" + r.L + "," + r.T + ")";
        display = mi.szDevice + " " + Hz(mi.szDevice); }
      return true; }, IntPtr.Zero);
    return mode + "|" + window + "|" + display; }
}
'@
[XrWin]::DpiAware()

function Iso([datetime]$t) { $t.ToString('yyyy-MM-ddTHH:mm:ss.fffzzz', $inv) }
function ParseIso([string]$s) { [datetimeoffset]::Parse($s, $inv).LocalDateTime }
# typeperf's timestamp: "MM/dd/yyyy HH:mm:ss.fff" on Windows 11, local time. $null if it doesn't parse.
function TpTime($s) { $d = [datetime]::MinValue
    if ([datetime]::TryParseExact("$s".Trim(), 'MM/dd/yyyy HH:mm:ss.fff', $inv, [Globalization.DateTimeStyles]::None, [ref]$d)) { return $d }
    if ([datetime]::TryParse("$s".Trim(), [ref]$d)) { return $d }
    $null }
function Num($v) { $d = 0.0; if ([double]::TryParse(("$v").Trim(), [Globalization.NumberStyles]::Float, $inv, [ref]$d)) { $d } else { [double]::NaN } }
function Save-Meta { $script:meta | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $script:out 'meta.json') -Encoding utf8 }

# --- xReveal's saved setup (window mode, the stats overlay, the video format), as flat "section.key" values
$setupToml = "$env:LOCALAPPDATA\xReveal\setup.toml"
function Read-Setup { $s = @{}; $sec = ''
    if (-not (Test-Path $setupToml)) { return $null }
    foreach ($l in Get-Content $setupToml) {
        if ($l -match '^\s*\[+([^\]]+)\]+\s*$') { $sec = $Matches[1] + '.' }
        elseif ($l -match '^\s*([\w-]+)\s*=\s*"?([^"]*?)"?\s*$') { if (-not $s.ContainsKey("$sec$($Matches[1])")) { $s["$sec$($Matches[1])"] = $Matches[2] } } }
    [ordered]@{
        window_mode  = $(if ($s['window-mode']) { $s['window-mode'] } else { 'windowed (default)' })
        stats_shown  = $(if ($s['stats.shown']) { $s['stats.shown'] } else { 'false (default)' })
        video_format = "$($s['video-format.width'])x$($s['video-format.height']) @ $($s['video-format.fps'])"
    } }

# --- samples.csv from the raw files: typeperf's samples, with the window state of the nearest windows.csv row
$columns = 'time', 'pid', 'alive', 'mode', 'window', 'display', 'cpu_pct', 'cpu_utility_pct', 'gpu_pct', 'private_ws_mb',
    'private_bytes_mb', 'working_set_mb', 'gpu_dedicated_mb', 'gpu_shared_mb', 'handles', 'threads', 'machine_cpu_pct', 'machine_utility_pct'
function Convert-Run([string]$dir, $meta) {
    $logical = [double]$meta.logical_cpus
    $win = @(); $wcsv = Join-Path $dir 'windows.csv'
    if (Test-Path $wcsv) { $win = @(Import-Csv $wcsv | ForEach-Object { $_ | Add-Member t (ParseIso $_.time) -PassThru }) }
    $rows = [Collections.Generic.List[object]]::new()
    foreach ($p in @($meta.processes)) {
        $pidS = "$($p.pid)"
        $myWin = @($win | Where-Object { $_.pid -eq $pidS -and $_.alive -eq '1' }); $wi = 0
        $files = @(Get-ChildItem $dir -Filter "typeperf-$pidS-*.csv" | Sort-Object { [int](($_.BaseName -split '-')[-1]) })
        $segs = [Collections.Generic.List[object]]::new()
        foreach ($f in $files) { $r = @(Import-Csv $f.FullName); if ($r.Count) { $segs.Add($r) } }
        for ($si = 0; $si -lt $segs.Count; $si++) {
            $seg = $segs[$si]
            $cols = $seg[0].PSObject.Properties.Name; $stampCol = $cols[0]
            $pick = { param($re) @($cols | Where-Object { $_ -match $re }) }
            $c = @{
                cpu = & $pick '\\Process V2\(.*\)\\% Processor Time$' | Select-Object -First 1
                wsp = & $pick 'Working Set - Private$' | Select-Object -First 1
                pb  = & $pick 'Private Bytes$' | Select-Object -First 1
                ws  = & $pick '\)\\Working Set$' | Select-Object -First 1
                h   = & $pick 'Handle Count$' | Select-Object -First 1
                t   = & $pick 'Thread Count$' | Select-Object -First 1
                mc  = & $pick 'Processor Information\(_Total\)\\% Processor Time$' | Select-Object -First 1
                mu  = & $pick '% Processor Utility$' | Select-Object -First 1
            }
            $gpuCols = & $pick 'GPU Engine.*Utilization Percentage$'
            $dedCols = & $pick 'Dedicated Usage$'; $shCols = & $pick 'Shared Usage$'
            $next = [datetime]::MaxValue
            if ($si + 1 -lt $segs.Count) { $n1 = TpTime ($segs[$si + 1][0].PSObject.Properties | Select-Object -First 1).Value; if ($n1) { $next = $n1 } }
            foreach ($r in $seg) {
                $t = TpTime $r.$stampCol
                if (-not $t) { continue }
                if ($t -ge $next) { break }   # the next typeperf took over
                $pb = Num $r.($c.pb)
                $gv = @($gpuCols | ForEach-Object { Num $r.$_ } | Where-Object { -not [double]::IsNaN($_) })
                $cpu = (Num $r.($c.cpu)) / $logical
                $mc = Num $r.($c.mc); $mu = if ($c.mu) { Num $r.($c.mu) } else { [double]::NaN }
                # The window row at this sample: the last one up to 2.5 s after it (both lists are in time order).
                while ($wi + 1 -lt $myWin.Count -and $myWin[$wi + 1].t -le $t.AddSeconds(2.5)) { $wi++ }
                $w = if ($myWin.Count) { $myWin[$wi] } else { $null }
                $o = [ordered]@{ time = Iso $t; pid = $pidS; alive = $(if ([double]::IsNaN($pb) -or $pb -le 0) { '0' } else { '1' })
                    mode = "$($w.mode)"; window = "$($w.window)"; display = "$($w.display)" }
                $vals = [ordered]@{
                    cpu_pct = $cpu
                    cpu_utility_pct = $(if ($mc -gt 0) { $cpu * $mu / $mc } else { [double]::NaN })
                    gpu_pct = $(if ($gv.Count) { ($gv | Measure-Object -Maximum).Maximum } else { [double]::NaN })
                    private_ws_mb = (Num $r.($c.wsp)) / 1MB
                    private_bytes_mb = $pb / 1MB
                    working_set_mb = (Num $r.($c.ws)) / 1MB
                    gpu_dedicated_mb = $(if ($dedCols.Count) { (@($dedCols | ForEach-Object { Num $r.$_ }) | Measure-Object -Sum).Sum / 1MB } else { [double]::NaN })
                    gpu_shared_mb = $(if ($shCols.Count) { (@($shCols | ForEach-Object { Num $r.$_ }) | Measure-Object -Sum).Sum / 1MB } else { [double]::NaN })
                    handles = Num $r.($c.h)
                    threads = Num $r.($c.t)
                    machine_cpu_pct = $mc
                    machine_utility_pct = $mu
                }
                foreach ($k in $vals.Keys) { $o[$k] = if ([double]::IsNaN($vals[$k])) { '' } else { $vals[$k].ToString('0.####', $inv) } }
                $rows.Add([pscustomobject]$o)
            }
        }
    }
    # The ticks with no xReveal (before it starts, after it exits) keep the timeline.
    foreach ($w in @($win | Where-Object { $_.alive -ne '1' })) {
        $o = [ordered]@{}; foreach ($k in $columns) { $o[$k] = '' }
        $o.time = $w.time; $o.pid = $w.pid; $o.alive = '0'
        $rows.Add([pscustomobject]$o)
    }
    $sorted = @($rows | Sort-Object { ParseIso $_.time })
    $sorted | Select-Object $columns | Export-Csv (Join-Path $dir 'samples.csv') -NoTypeInformation -Encoding utf8
    $sorted.Count
}

# --- summary.md: the same rules and format as analyze.py (keep the two in step)
function Median([double[]]$v) { $v = @($v | Where-Object { -not [double]::IsNaN($_) } | Sort-Object); if (-not $v.Count) { return [double]::NaN }
    $m = [int][math]::Floor($v.Count / 2); if ($v.Count % 2) { $v[$m] } else { ($v[$m - 1] + $v[$m]) / 2 } }
function P95([double[]]$v) { $v = @($v | Where-Object { -not [double]::IsNaN($_) } | Sort-Object); if (-not $v.Count) { return [double]::NaN }
    $v[[int][math]::Min($v.Count - 1, [math]::Ceiling(0.95 * $v.Count) - 1)] }
function Fit($pts) {   # least squares through (minute, value): slope per hour, sigma around the line, max - min
    $pts = @($pts | Where-Object { -not [double]::IsNaN($_[1]) })
    if ($pts.Count -lt 3) { return @([double]::NaN, [double]::NaN, [double]::NaN) }
    $mx = ($pts | ForEach-Object { $_[0] } | Measure-Object -Average).Average; $my = ($pts | ForEach-Object { $_[1] } | Measure-Object -Average).Average
    $sxx = 0.0; $sxy = 0.0; foreach ($p in $pts) { $sxx += ($p[0] - $mx) * ($p[0] - $mx); $sxy += ($p[0] - $mx) * ($p[1] - $my) }
    $b = $sxy / $sxx; $a = $my - $b * $mx; $ss = 0.0; foreach ($p in $pts) { $e = $p[1] - ($a + $b * $p[0]); $ss += $e * $e }
    $ys = @($pts | ForEach-Object { $_[1] })
    @(($b * 60), [math]::Sqrt($ss / [math]::Max(1, $pts.Count - 2)), (($ys | Measure-Object -Maximum).Maximum - ($ys | Measure-Object -Minimum).Minimum)) }
function F([double]$v, [int]$d, [string]$unit = '') { if ([double]::IsNaN($v)) { 'n/a' } else { $v.ToString("N$d", $inv) + $unit } }

function Write-Summary([string]$dir) {
    $meta = Get-Content (Join-Path $dir 'meta.json') -Raw | ConvertFrom-Json
    $interval = [double]$meta.interval_s; $perMinute = 60.0 / $interval
    $rowsCfg = @(
        @('cpu_pct', 'CPU, share of the machine', 2, '%', '<= 5%', 5.0),
        @('cpu_utility_pct', 'CPU, scaled to Processor Utility (Task Manager basis, estimate)', 2, '%', '', $null),
        @('gpu_pct', 'GPU, process (max engine)', 1, '%', '<= 10%', 10.0),
        @('private_ws_mb', 'Memory: private working set (Task Manager "Memory")', 1, ' MB', 'ceiling below', $null),
        @('private_bytes_mb', 'Private Bytes (Task Manager "Commit size")', 1, ' MB', '', $null),
        @('working_set_mb', 'Working set', 1, ' MB', '', $null),
        @('gpu_dedicated_mb', 'Dedicated GPU memory', 1, ' MB', 'ceiling below', $null),
        @('gpu_shared_mb', 'Shared GPU memory', 1, ' MB', '', $null),
        @('handles', 'Handles', 0, '', '', $null),
        @('threads', 'Threads', 0, '', '', $null),
        @('machine_cpu_pct', 'Whole machine CPU (% Processor Time)', 1, '%', '', $null),
        @('machine_utility_pct', 'Whole machine CPU (% Processor Utility)', 1, '%', '', $null))
    $fitCfg = @(@('private_bytes_mb', 'Private Bytes'), @('private_ws_mb', 'Private working set'), @('gpu_dedicated_mb', 'Dedicated GPU memory'))
    $minCfg = @(@('private_bytes_mb', 'Private Bytes MB', 1), @('private_ws_mb', 'Private WS MB', 1), @('gpu_dedicated_mb', 'Dedicated GPU MB', 1),
        @('handles', 'Handles', 0), @('threads', 'Threads', 0), @('cpu_pct', 'CPU %', 2), @('gpu_pct', 'GPU %', 1))
    $ceilCfg = @(@('private_ws_mb', 'Memory (private working set)', 40, 40), @('gpu_dedicated_mb', 'Dedicated GPU memory', 120, 290))
    $states = @(@('windowed', 'Windowed'), @('fullscreen', 'Fullscreen'))
    $cols = @($rowsCfg | ForEach-Object { $_[0] })

    $procs = @($meta.processes)
    if (-not $procs.Count) { throw "meta.json lists no xReveal process; nothing was sampled." }
    $main = $procs[0]; $pidS = "$($main.pid)"; $launch = ParseIso $main.start
    $samples = @(Import-Csv (Join-Path $dir 'samples.csv') | Where-Object { $_.pid -eq $pidS -and $_.alive -eq '1' } | ForEach-Object {
        $s = [ordered]@{ t = ParseIso $_.time; mode = $_.mode; window = $_.window; display = $_.display }
        foreach ($c in $cols) { $s[$c] = Num $_.$c }
        $s.min = [int][math]::Floor(($s.t - $launch).TotalSeconds / 60) + 1
        [pscustomobject]$s })
    if (-not $samples.Count) { throw "no samples of pid $pidS in samples.csv." }

    $phases = @{}; $ceilMin = @{}
    foreach ($st in $states) {
        $ss = @($samples | Where-Object { $_.mode -eq $st[0] }); if (-not $ss.Count) { continue }
        $end = $ss[-1].t; $last = @($ss | Where-Object { $_.t -gt $end.AddMinutes(-3) })
        $ph = @{ from = $last[0].min; to = $last[-1].min; med = @{}; p95 = @{} }
        foreach ($c in $cols) { $ph.med[$c] = Median @($last | ForEach-Object { $_.$c }); $ph.p95[$c] = P95 @($last | ForEach-Object { $_.$c }) }
        $phases[$st[0]] = $ph
        $ceilMin[$st[0]] = @($ss | Group-Object min | Where-Object { $_.Count -ge $perMinute / 2 })
    }
    $minutes = @($samples | Group-Object min | Sort-Object { [int]$_.Name } | ForEach-Object {
        $g = @($_.Group); $o = [ordered]@{ min = [int]$_.Name; n = $g.Count
            mode = ($g | Group-Object mode | Sort-Object Count -Descending | Select-Object -First 1).Name }
        if (-not $o.mode) { $o.mode = '-' }
        foreach ($c in $cols) { $o[$c] = Median @($g | ForEach-Object { $_.$c }) }
        [pscustomobject]$o })
    $minutes | ForEach-Object { $m = $_; $o = [ordered]@{ min = $m.min; n = $m.n; mode = $m.mode }
        foreach ($c in $cols) { $o[$c] = if ([double]::IsNaN($m.$c)) { '' } else { [math]::Round($m.$c, 3).ToString($inv) } }; [pscustomobject]$o } |
        Export-Csv (Join-Path $dir 'minutes.csv') -NoTypeInformation -Encoding utf8

    $tail = @($minutes | Where-Object { $_.min -ge 11 -and $_.min -le 20 })
    $fits = @{}; foreach ($fc in $fitCfg) { $fits[$fc[0]] = Fit @($tail | ForEach-Object { , @([double]$_.min, [double]$_.($fc[0])) }) }
    $slope = $fits['private_bytes_mb'][0]; $sigma = $fits['private_bytes_mb'][1]
    $m11 = ($tail | Where-Object { $_.min -eq 11 }).private_bytes_mb; $m20 = ($tail | Where-Object { $_.min -eq 20 }).private_bytes_mb
    if ($null -eq $m11) { $m11 = [double]::NaN }; if ($null -eq $m20) { $m20 = [double]::NaN }

    $L = [Collections.Generic.List[string]]::new()
    $L.Add("# xReveal Windows samples, $($meta.stamp)"); $L.Add('')
    $L.Add("- **Windows:** $($meta.os_version)  ")
    $L.Add("- **CPU:** $($meta.cpu), $($meta.logical_cpus) logical; **RAM:** $($meta.ram_gb) GB  ")
    $L.Add("- **GPU:** $(@($meta.gpus) -join '; ')  ")
    $L.Add("- **Monitors:** $(@($meta.monitors) -join '; ')  ")
    $L.Add("- **xReveal:** $($meta.xreveal); pid $pidS, launched $($launch.ToString('HH:mm:ss'))  ")
    $L.Add("- **Sampler:** $($meta.sampler), every $interval s, $($samples.Count) samples of pid $pidS (min $($samples[0].min)-$($samples[-1].min))  ")
    foreach ($pair in @(@('at the start', $meta.setup_start), @('at the end', $meta.setup_end))) {
        if ($pair[1]) { $L.Add("- **setup.toml $($pair[0]):** window mode $($pair[1].window_mode), stats overlay $($pair[1].stats_shown), video format $($pair[1].video_format)  ") } }
    foreach ($st in $states) {
        $seen = @($samples | Where-Object { $_.mode -eq $st[0] } | Group-Object { if ($_.display) { "$($_.window) on $($_.display)" } else { $_.window } } | Sort-Object Count -Descending)
        if ($seen.Count) { $L.Add("- **$($st[1]) window:** $(($seen | ForEach-Object { "$($_.Name) ($($_.Count) samples)" }) -join '; ')  ") } }
    $other = @($samples | Where-Object { $_.mode -ne 'windowed' -and $_.mode -ne 'fullscreen' }).Count
    if ($other) { $L.Add("- **No window on screen** (minimised, hidden or another Space) in $other samples.  ") }
    if ($main.exited) { $L.Add("- **xReveal exited** at $((ParseIso $main.exited).ToString('HH:mm:ss'))$(if ($null -ne $main.exit_code -and "$($main.exit_code)" -ne '') { " (exit code $($main.exit_code))" }).  ") }
    foreach ($p in @($procs | Select-Object -Skip 1)) { $L.Add("- Note: another xreveal process, pid $($p.pid), launched $((ParseIso $p.start).ToString('HH:mm:ss')), was sampled too; it isn't in this summary.  ") }
    foreach ($n in @($meta.notes)) { if ($n) { $L.Add("- Note: $n  ") } }
    $L[$L.Count - 1] = $L[$L.Count - 1].TrimEnd()

    $shown = @($states | Where-Object { $phases.ContainsKey($_[0]) })
    $L.Add(''); $L.Add('## Budgets (medians; p95 in brackets)'); $L.Add('')
    if ($shown.Count) {
        $L.Add('| | ' + (($shown | ForEach-Object { "$($_[1]) (last 3 min: min $($phases[$_[0]].from)-$($phases[$_[0]].to))" }) -join ' | ') + ' | Budget |')
        $L.Add('|---|' + ('---|' * $shown.Count) + '---|')
        foreach ($rc in $rowsCfg) {
            $cells = @(); $ok = @()
            foreach ($st in $shown) { $ph = $phases[$st[0]]; $med = $ph.med[$rc[0]]
                $cells += "$(F $med $rc[2] $rc[3]) ($(F $ph.p95[$rc[0]] $rc[2]))"
                if ($null -ne $rc[5] -and -not [double]::IsNaN($med)) { $ok += ($med -le $rc[5]) } }
            $budget = $rc[4]; if ($ok.Count) { $budget += $(if ($ok -contains $false) { ': OVER' } else { ': ok' }) }
            $L.Add("| $($rc[1]) | $($cells -join ' | ') | $budget |") }
    } else { $L.Add("No samples with xReveal's window on screen, so no Windowed or Fullscreen medians.") }

    $L.Add(''); $L.Add('## Memory ceilings (spec §4: the highest 1-minute median in each state, whole run)'); $L.Add('')
    $L.Add('| | Windowed highest (minute) | Fullscreen highest (minute) | Ceiling | Verdict |'); $L.Add('|---|---|---|---|---|')
    foreach ($cc in $ceilCfg) {
        $cells = @(); $verdict = @()
        for ($i = 0; $i -lt 2; $i++) { $st = $states[$i][0]; $limit = $cc[2 + $i]
            $meds = @(@($ceilMin[$st]) | Where-Object { $_ } | ForEach-Object { [pscustomobject]@{ v = Median @($_.Group | ForEach-Object { $_.($cc[0]) }); m = [int]$_.Name } } | Where-Object { -not [double]::IsNaN($_.v) })
            if (-not $meds.Count) { $cells += 'not seen'; continue }
            $top = $meds | Sort-Object v, m -Descending | Select-Object -First 1
            $cells += "$(F $top.v 1 ' MB') ($($top.m))"; $verdict += "$st $(if ($top.v -le $limit) { 'ok' } else { 'OVER' })" }
        $L.Add("| $($cc[1]) | $($cells -join ' | ') | <= $($cc[2]) / $($cc[3]) MB | $(if ($verdict.Count) { $verdict -join ', ' } else { 'n/a' }) |") }
    $L.Add('')
    $L.Add("The ceilings hold for the reference setup only (spec §4: the Windows PC's 3440x1440 monitor with a 1711x974 window; the MacBook's built-in screen with a 1280x887 pt window; overlay on). Check the window lines above.")

    $L.Add(''); $L.Add('## 1-minute medians from launch'); $L.Add('')
    $L.Add('| min | mode | ' + (($minCfg | ForEach-Object { $_[1] }) -join ' | ') + ' |'); $L.Add('|---|---|' + ('---|' * $minCfg.Count))
    foreach ($m in $minutes) { $L.Add("| $($m.min) | $($m.mode) | $(($minCfg | ForEach-Object { F $m.($_[0]) $_[2] }) -join ' | ') |") }

    $L.Add(''); $L.Add('## Minutes 11-20 (after the warm-up)'); $L.Add('')
    $L.Add('| | Slope | Wobble (sigma) of 1-min medians around the line | Max - min |'); $L.Add('|---|---|---|---|')
    foreach ($fc in $fitCfg) { $f = $fits[$fc[0]]; $L.Add("| $($fc[1]) | $(F $f[0] 2) MB/h | $(F $f[1] 2) MB | $(F $f[2] 2) MB |") }
    $L.Add('')
    $settled = if ([double]::IsNaN($slope) -or [double]::IsNaN($m11) -or [double]::IsNaN($m20)) { 'n/a (no minutes 11 and 20 in the run)' }
        elseif ([math]::Abs($slope) -lt 6 -and [math]::Abs($m20 - $m11) -lt 3) { 'yes' } else { 'NO: look at the table' }
    $L.Add("- **Settled by minute 10 (Private Bytes):** $settled (rule: slope under 6 MB/h and minutes 11 and 20 within 3 MB).")
    $holds = if ([double]::IsNaN($sigma)) { 'n/a' } elseif ($sigma -lt 1.6) { 'holds' } else { 'DOES NOT HOLD' }
    $L.Add("- **2-hour soak assumption (sigma < ~1.6 MB):** Private Bytes sigma = $(F $sigma 2) MB -> $holds.")
    $tailModes = @($tail | ForEach-Object { $_.mode } | Sort-Object -Unique)
    if ($tailModes.Count -gt 1) { $L.Add("- Note: the window state changed within minutes 11-20 ($($tailModes -join ', ')), so the line mixes states.") }
    $L.Add('- Ten points is a rough sigma; it is a check of the assumption, not a leak verdict.')
    $path = Join-Path $dir 'summary.md'
    [IO.File]::WriteAllLines($path, $L, [Text.UTF8Encoding]::new($false))
    $path
}

if ($Analyze) {
    $out = (Resolve-Path $Analyze).Path
    $meta = Get-Content (Join-Path $out 'meta.json') -Raw | ConvertFrom-Json
    $n = Convert-Run $out $meta
    Write-Host "sample-windows: $n rows in $out\samples.csv"
    Write-Host "sample-windows: wrote $(Write-Summary $out)"
    return
}

# --- The machine
if (-not ((typeperf -qx "Process V2" 2>$null | Select-Object -First 1) -match 'Process V2')) { throw "This Windows has no 'Process V2' counter set (Windows 11 has it); the sampler needs it to tell processes apart." }
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$out = if ($Out) { $Out } else { Join-Path (Get-Location) "xreveal-samples-$stamp" }
New-Item -ItemType Directory $out -Force | Out-Null
$out = (Resolve-Path $out).Path
Add-Type -AssemblyName System.Windows.Forms
$cv = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
$meta = [ordered]@{
    os = 'windows'; stamp = $stamp; interval_s = $IntervalSeconds
    sampler = 'sample-windows.ps1 (typeperf, Process V2)'
    # ProductName still says "Windows 10" on Windows 11 (build 22000 and later).
    os_version = "$(if ([int]$cv.CurrentBuild -ge 22000) { $cv.ProductName -replace 'Windows 10', 'Windows 11' } else { $cv.ProductName }) $($cv.DisplayVersion), build $($cv.CurrentBuild).$($cv.UBR)"
    cpu = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name.Trim()
    logical_cpus = [Environment]::ProcessorCount
    ram_gb = [math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB, 1)
    gpus = @(Get-CimInstance Win32_VideoController | ForEach-Object { "$($_.Name) (driver $($_.DriverVersion))" })
    monitors = @([System.Windows.Forms.Screen]::AllScreens | ForEach-Object { "$($_.DeviceName) $([XrWin]::Hz($_.DeviceName))$(if ($_.Primary) {' (primary)'})" })
    setup_start = Read-Setup
    sampler_start = Iso (Get-Date)
    processes = @()
    notes = @()
}
Save-Meta
Write-Host "sample-windows: writing $out every $IntervalSeconds s. Ctrl+C stops." -ForegroundColor Cyan

$winCsv = Join-Path $out 'windows.csv'
'"time","pid","alive","mode","window","display"' | Set-Content $winCsv -Encoding utf8
$cur = $null; $usedPid = $false; $waitingSaid = $false
$clock = [Diagnostics.Stopwatch]::StartNew(); $k = 0

function Engines([int]$id) { @(typeperf -qx "GPU Engine" 2>$null | Where-Object { $_ -match "pid_${id}_" }).Count }
function Start-Typeperf($c) {
    $c.seg++
    $id = $c.pid
    $counters = @(
        "\Process V2(${Name}:$id)\% Processor Time", "\Process V2(${Name}:$id)\Working Set - Private",
        "\Process V2(${Name}:$id)\Private Bytes", "\Process V2(${Name}:$id)\Working Set",
        "\Process V2(${Name}:$id)\Handle Count", "\Process V2(${Name}:$id)\Thread Count",
        "\GPU Engine(pid_${id}_*)\Utilization Percentage",
        "\GPU Process Memory(pid_${id}_*)\Dedicated Usage", "\GPU Process Memory(pid_${id}_*)\Shared Usage",
        "\Processor Information(_Total)\% Processor Time", "\Processor Information(_Total)\% Processor Utility")
    $cf = Join-Path $out "counters-$id-$($c.seg).txt"; $counters | Set-Content $cf -Encoding ascii
    $csv = Join-Path $out "typeperf-$id-$($c.seg).csv"
    $old = $c.tp
    $c.tp = Start-Process typeperf -ArgumentList @('-cf', "`"$cf`"", '-si', $IntervalSeconds, '-f', 'CSV', '-o', "`"$csv`"", '-y') -PassThru -WindowStyle Hidden
    $c.engines = Engines $id
    if ($old -and -not $old.HasExited) { Start-Sleep -Seconds ($IntervalSeconds + 1); Stop-Process $old -ErrorAction SilentlyContinue }
    Write-Host "sample-windows: typeperf #$($c.seg) for pid $id, $($c.engines) GPU engine instances"
}

try {
    while ($true) {
        if ($Minutes -gt 0 -and $clock.Elapsed.TotalMinutes -ge $Minutes) { break }
        $now = Get-Date

        # Attach: the given process once, or the newest one called -Name.
        if (-not $cur) {
            $p = $null
            if ($ProcessId -and -not $usedPid) {
                $usedPid = $true
                $p = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
                if (-not $p) { throw "No process $ProcessId." }
                $Name = $p.ProcessName
            } elseif (-not $ProcessId) {
                $p = Get-Process $Name -ErrorAction SilentlyContinue | Sort-Object StartTime -Descending | Select-Object -First 1
                if (-not $p -and -not $waitingSaid) { Write-Host "sample-windows: waiting for $Name.exe to start..."; $waitingSaid = $true }
            }
            if ($p) {
                $null = $p.Handle   # keeps a handle, so ExitCode is readable after it exits
                $exe = $p.Path
                $ver = (Get-Item $exe).VersionInfo.ProductVersion
                if (-not $ver) { $ver = "$((Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\xReveal' -ErrorAction SilentlyContinue).DisplayVersion) (from the Uninstall key)" }
                if (-not $meta.Contains('xreveal')) { $meta.xreveal = "$ver (exe built $((Get-Item $exe).LastWriteTime.ToString('yyyy-MM-dd HH:mm')))"; $meta.exe = $exe }
                $entry = [ordered]@{ pid = $p.Id; start = Iso $p.StartTime; attached = Iso $now; exited = $null; exit_code = $null; path = $exe }
                $meta.processes = @($meta.processes) + , $entry; Save-Meta
                $cur = @{ pid = $p.Id; proc = $p; entry = $entry; tp = $null; seg = 0; engines = 0; nextCheck = $null
                    tpAt = $p.StartTime.AddSeconds(10) }   # GPU engine instances exist once it has drawn
                $waitingSaid = $false
                Write-Host "sample-windows: sampling $Name (pid $($p.Id)), launched $($entry.start)" -ForegroundColor Cyan
            }
        }

        $state = 'none||'; $alive = '0'; $pidS = ''
        if ($cur) {
            $pidS = "$($cur.pid)"
            if ($cur.proc.HasExited) {
                $cur.entry.exited = Iso $now
                try { $cur.entry.exit_code = $cur.proc.ExitCode } catch {}
                if ($cur.tp -and -not $cur.tp.HasExited) { Stop-Process $cur.tp -ErrorAction SilentlyContinue }
                Save-Meta
                Write-Host "sample-windows: pid $pidS exited (code $($cur.entry.exit_code)); still sampling." -ForegroundColor Yellow
                $cur = $null
            } else {
                $alive = '1'; $state = [XrWin]::State([uint32]$cur.pid)
                if (-not $cur.tp -and $now -ge $cur.tpAt) { Start-Typeperf $cur; $cur.nextCheck = (Get-Date).AddMinutes(1) }
                elseif ($cur.tp -and $cur.tp.HasExited) { $meta.notes = @($meta.notes) + "typeperf stopped by itself at $($now.ToString('HH:mm:ss')) (exit $($cur.tp.ExitCode)); restarted."; Save-Meta; Start-Typeperf $cur; $cur.nextCheck = (Get-Date).AddMinutes(1) }
                elseif ($cur.tp -and $now -ge $cur.nextCheck) {
                    if ((Engines $cur.pid) -gt $cur.engines) { Start-Typeperf $cur }
                    $cur.nextCheck = (Get-Date).AddMinutes(1) }
            }
        }
        $parts = $state -split '\|', 3
        $line = @((Iso $now), $pidS, $alive, $parts[0], $parts[1], $parts[2]) | ForEach-Object { '"' + ("$_" -replace '"', '""') + '"' }
        Add-Content $winCsv ($line -join ',') -Encoding utf8

        $k++
        $wait = $k * $IntervalSeconds * 1000 - $clock.ElapsedMilliseconds
        if ($wait -gt 0) { Start-Sleep -Milliseconds $wait }
    }
} finally {
    if ($cur -and $cur.tp -and -not $cur.tp.HasExited) { Stop-Process $cur.tp -ErrorAction SilentlyContinue }
    $meta.sampler_end = Iso (Get-Date)
    $meta.setup_end = Read-Setup
    Save-Meta
    if (@($meta.processes).Count) {
        Start-Sleep -Seconds 1
        $meta2 = Get-Content (Join-Path $out 'meta.json') -Raw | ConvertFrom-Json
        $n = Convert-Run $out $meta2
        Write-Host "sample-windows: $n rows in $out\samples.csv"
        Write-Host "sample-windows: wrote $(Write-Summary $out)" -ForegroundColor Green
    } else { Write-Host "sample-windows: $Name never ran; no summary. Window log in $winCsv" }
}
