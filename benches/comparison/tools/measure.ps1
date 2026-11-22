#requires -Version 7
param(
    [ValidateSet('comparison', 'profiles', 'memory', 'native')] [string] $Suite = 'comparison',
    [ValidateRange(1, 1000)] [int] $Runs = 5,
    [ValidateRange(1, 100)] [double] $MaxBusy = 10,
    [ValidateRange(1, 100)] [double] $MaxDuring = 20,
    [string] $Out
)
$ErrorActionPreference = 'Stop'
$bench = (Resolve-Path "$PSScriptRoot/..").Path
$target = (Resolve-Path "$bench/../../target/comparison").Path
if (!$Out) { $Out = Join-Path $target "$Suite-$(Get-Date -Format yyyyMMdd-HHmmss).jsonl" }
if (Test-Path -LiteralPath $Out) { throw "Choose a fresh output file: $Out" }
if (!('MeasurePid' -as [type])) {
    Add-Type -TypeDefinition (Get-Content -LiteralPath "$PSScriptRoot/Measure.csx" -Raw)
}

$jit = "$target/apps/jit/compare-skathi.exe"
$aot = "$target/apps/aot/compare-skathi.exe"
$electron = "$target/apps/compare-electron-win32-x64/compare-electron.exe"
$tauri = "$target/apps/tauri/tauri-app.exe"
$sk = @{ SK_WINDOW_FOCUS = 'never'; SK_MEMORY = 'default'; SK_IDLE_PACING = '1' }
$apps = switch ($Suite) {
    'comparison' {
        @{ name = 'skathi'; exe = $jit; env = $sk }
        @{ name = 'electron'; exe = $electron }
        @{ name = 'tauri'; exe = $tauri }
    }
    'profiles' {
        @{ name = 'jit-default'; exe = $jit; env = $sk }
        @{ name = 'jit-low'; exe = $jit; env = $sk; override = @{ SK_MEMORY = 'low' } }
        @{ name = 'jit-unpaced'; exe = $jit; env = $sk; override = @{ SK_IDLE_PACING = '0' } }
        @{ name = 'aot-default'; exe = $aot; env = $sk; override = @{ SK_AOT_MEMORY_PROFILE = 'default' } }
        @{ name = 'aot-compact'; exe = $aot; env = $sk; override = @{ SK_AOT_MEMORY_PROFILE = 'compact' } }
        @{ name = 'aot-compact-low'; exe = $aot; env = $sk; override = @{ SK_AOT_MEMORY_PROFILE = 'compact'; SK_MEMORY = 'low' } }
        @{ name = 'trimmed-compact-low'; exe = "$target/apps/trimmed/compare-skathi.exe"; env = $sk; override = @{ SK_AOT_MEMORY_PROFILE = 'compact'; SK_MEMORY = 'low' } }
        @{ name = 'full'; exe = "$target/apps/full/compare-skathi.exe"; env = $sk }
    }
    'memory' {
        $exe = "$target/apps/standalone/sk-app.exe"
        $envs = $sk + @{ SK_PAYLOAD = "$target/apps/standalone/app.skpk" }
        @{ name = 'standalone-default'; exe = $exe; env = $envs }
        @{ name = 'standalone-low'; exe = $exe; env = $envs; override = @{ SK_MEMORY = 'low' } }
    }
    'native' {
        foreach ($kind in 'vec','slice') {
            @{ name = "skathi-$kind"; exe = "$target/apps/$kind/marshal.exe"; env = $sk; payload = "$target/apps/$kind/app.skpk" }
        }
        @{ name = 'electron'; exe = $electron; env = @{ COMPARE_BENCH = '1' } }
        @{ name = 'tauri'; exe = $tauri; env = @{ COMPARE_BENCH = '1' } }
    }
}
foreach ($app in $apps) {
    if (!(Test-Path -LiteralPath $app.exe -PathType Leaf)) { throw "Build $($app.name) first: $($app.exe)" }
}

function Sample([uint32] $root) {
    $tree = [MeasurePid]::Tree($root)
    if (!$tree.Count) { throw "Process $root exited before the sample" }
    $ws = 0; $private = 0; $commit = 0
    foreach ($id in $tree.Keys) {
        $m = [MeasurePid]::Mem($id)
        if (!$m[0]) { throw "Could not read the working set of process $id" }
        $ws += $m[0]; $private += $m[1]; $commit += $m[2]
    }
    $gpu = $null
    try {
        $counters = (Get-Counter '\GPU Process Memory(*)\Dedicated Usage' -ErrorAction Stop).CounterSamples
        $gpu = 0
        foreach ($counter in $counters) {
            if ($counter.InstanceName -match '^pid_(\d+)_' -and $tree.ContainsKey([uint32]$Matches[1])) {
                $gpu += $counter.CookedValue
            }
        }
        $gpu = [math]::Round($gpu / 1MB, 2)
    } catch { Write-Warning "GPU counter unavailable: $_" }
    [ordered]@{
        processes = $tree.Count
        names = ($tree.Values | Sort-Object) -join ', '
        ws_mb = [math]::Round($ws / 1MB, 2)
        private_ws_mb = [math]::Round($private / 1MB, 2)
        commit_mb = [math]::Round($commit / 1MB, 2)
        gpu_dedicated_mb = $gpu
    }
}

function CpuPercent([uint32] $root) {
    $before = @{}
    foreach ($id in [MeasurePid]::Tree($root).Keys) { $before[$id] = [MeasurePid]::Cpu($id) }
    $clock = [Diagnostics.Stopwatch]::StartNew()
    Start-Sleep -Seconds 10
    $elapsed = $clock.Elapsed.TotalMilliseconds
    $after = [MeasurePid]::Tree($root)
    if ($after.Count -ne $before.Count) { throw "Process tree changed during idle CPU sample" }
    $ticks = 0
    foreach ($id in $after.Keys) {
        if (!$before.ContainsKey($id)) { throw "Process $id joined during idle CPU sample" }
        $ticks += [MeasurePid]::Cpu($id) - $before[$id]
    }
    [math]::Round(($ticks / 10000) / $elapsed * 100, 3)
}

function Wait-Quiet {
    $deadline = [DateTime]::UtcNow.AddMinutes(3)
    do {
        $c = (Get-Counter '\Processor(_Total)\% Processor Time' -MaxSamples 3).CounterSamples
        $busy = ($c | Measure-Object CookedValue -Average).Average
        if ($busy -lt $MaxBusy) { return [math]::Round($busy, 2) }
        Start-Sleep -Seconds 5
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "System CPU stayed above $MaxBusy% for three minutes"
}

function Measure-App($app, [int] $round) {
    $busy = Wait-Quiet
    $psi = [Diagnostics.ProcessStartInfo]::new($app.exe)
    $psi.UseShellExecute = $false
    $psi.WorkingDirectory = Split-Path $app.exe
    foreach ($key in @($psi.Environment.Keys)) {
        if ($key -match '^(SK_|REDJS_|MIMALLOC_|ELECTRON_|VSCODE_|COMPARE_BENCH$|WEBVIEW2_)') {
            [void]$psi.Environment.Remove($key)
        }
    }
    foreach ($settings in $app.env, $app.override) {
        if ($settings) { foreach ($key in $settings.Keys) { $psi.Environment[$key] = $settings[$key] } }
    }
    if ($app.payload) { $psi.ArgumentList.Add($app.payload) }
    $systemBefore = [MeasurePid]::SystemCpu()
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $proc = [Diagnostics.Process]::Start($psi)
    $root = [uint32]$proc.Id
    try {
        $row = [ordered]@{ app = $app.name; run = $round; system_cpu_before_pct = $busy }
        if ($Suite -eq 'native') {
            $prefix = if ($app.payload) { 'Marshal bench' } else { 'Compare bench' }
            do {
                $title = [MeasurePid]::FindTitle($prefix, $root)
                if ($title) { break }
                if ($proc.HasExited) { throw "Exited with code $($proc.ExitCode)" }
                Start-Sleep -Milliseconds 50
            } while ($clock.ElapsedMilliseconds -lt 120000)
            $checksum = if ($app.payload) { 'check=1/100000/10000/10000/10000/320000/320000/52428800/52428800/52428800/52428800/52428800/52428800/52428800$' } else { 'check=10000/52428800$' }
            if ($title -notmatch $checksum) { throw "Missing or incorrect native-call result: $title" }
            $row.result = $title
        } else {
            $times = [MeasurePid]::WaitReady($clock, 30000, $root)
            if ($times[1] -lt 0) { throw "Window did not signal ready (exited: $($proc.HasExited))" }
            if ($round -eq 0) { return }
            while ($clock.ElapsedMilliseconds -lt 5000) { Start-Sleep -Milliseconds 20 }
            $row.ready_ms = [math]::Round($times[1], 1)
            $row.window_ms = [math]::Round($times[0], 1)
            $row.took_foreground = [MeasurePid]::Tree($root).ContainsKey([MeasurePid]::ForegroundPid())
            $row.idle = Sample $root
            $row.idle_cpu_pct = CpuPercent $root
            if ($clock.ElapsedMilliseconds -ge $times[1] + 20000) { throw "Idle CPU sample overlapped the scroll timer" }
            if ($Suite -eq 'comparison') {
                do {
                    if ([MeasurePid]::FindTitle('Compare scrolled', $root)) { break }
                    if ($clock.ElapsedMilliseconds -gt 60000 -or $proc.HasExited) { throw "Window did not finish scrolling" }
                    Start-Sleep -Milliseconds 20
                } while ($true)
                $row.scrolled_at_ms = $clock.ElapsedMilliseconds
                Start-Sleep -Seconds 3
                $row.after_scroll = Sample $root
                $row.after_scroll_cpu_pct = CpuPercent $root
            }
        }
        $systemAfter = [MeasurePid]::SystemCpu()
        $totalTicks = $systemAfter[1] - $systemBefore[1]
        $row.system_cpu_during_pct = [math]::Round(100 * (1 - ($systemAfter[0] - $systemBefore[0]) / $totalTicks), 2)
        if ($row.system_cpu_during_pct -ge $MaxDuring) { throw "System CPU averaged $($row.system_cpu_during_pct)% during sample" }
        return $row
    } finally {
        if (!$proc.HasExited) { $proc.Kill($true); $proc.WaitForExit() }
        $proc.Dispose()
        Start-Sleep -Seconds 2
    }
}

$first = if ($Suite -eq 'native') { 1 } else { 0 }
for ($round = $first; $round -le $Runs; $round++) {
    foreach ($app in $apps) {
        for ($attempt = 1; $attempt -le 3; $attempt++) {
            try {
                $row = Measure-App $app $round
                if ($row) {
                    $row.attempt = $attempt
                    $row | ConvertTo-Json -Compress -Depth 4 | Add-Content -LiteralPath $Out
                }
                Write-Host "$Suite $($app.name) $round/$Runs"
                break
            } catch {
                [ordered]@{ app = $app.name; run = $round; attempt = $attempt; reason = $_.Exception.Message } |
                    ConvertTo-Json -Compress | Add-Content -LiteralPath "$Out.discarded"
                Write-Warning "$($app.name) round $round attempt $attempt : $_"
                if ($attempt -eq 3) { throw }
            }
        }
    }
}
Write-Host $Out
