# Memory and latency of the release model worker: router only, then with the extractor loaded.
#
#   cargo build --release -p jitmo-model-worker
#   powershell -File tools/perf/measure_worker_memory.ps1 [-Exe target\release\jitmo-model-worker.exe]
#
# Needs models/router and models/extractor (see crates/model-worker/README.md).
param(
    [string]$Exe = "target\release\jitmo-model-worker.exe",
    [string]$Models = "models\router",
    [string]$Extractor = "models\extractor",
    [string]$Data = "spikes\sp2-extractor\data\doc4.txt"
)

$ErrorActionPreference = "Stop"
$sentences = Get-Content $Data | ForEach-Object { ($_ -split '\|', 2)[1] -replace '\[\w+\|([^\]]*)\]', '$1' }

function Start-Worker([string]$extra) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = (Resolve-Path $Exe).Path
    $psi.Arguments = "--models `"$Models`" --extractor `"$Extractor`" $extra"
    $psi.RedirectStandardInput = $true
    $psi.RedirectStandardOutput = $true
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    [System.Diagnostics.Process]::Start($psi)
}

$script:id = 0
function Call($p, [string]$type, [string]$text) {
    $script:id++
    $t = ConvertTo-Json $text -Compress
    $line = if ($type -eq "ping") { "{`"type`":`"ping`",`"id`":$($script:id)}" }
            elseif ($type -eq "understand") { "{`"type`":`"understand`",`"id`":$($script:id),`"text`":$t,`"context`":{}}" }
            else { "{`"type`":`"extract`",`"id`":$($script:id),`"text`":$t,`"context`":{}}" }
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $p.StandardInput.WriteLine($line)
    $p.StandardInput.Flush()
    $reply = $p.StandardOutput.ReadLine()
    $sw.Stop()
    [pscustomobject]@{ Ms = $sw.Elapsed.TotalMilliseconds; Reply = $reply }
}

function Mem($p) {
    $p.Refresh()
    "{0,5:N0} MB working set, {1,5:N0} MB private, {2,5:N0} MB peak working set" -f ($p.WorkingSet64 / 1MB), ($p.PrivateMemorySize64 / 1MB), ($p.PeakWorkingSet64 / 1MB)
}

function Stats($ms) {
    $s = $ms | Sort-Object
    "{0} calls: mean {1:N1} ms, median {2:N1} ms, p95 {3:N1} ms, max {4:N1} ms" -f $s.Count, ($s | Measure-Object -Average).Average, $s[[int]($s.Count / 2)], $s[[int]($s.Count * 0.95)], $s[-1]
}

$p = Start-Worker "--idle-secs 0"
$null = Call $p ping ""
"router loaded, idle:            " + (Mem $p)
$r = $sentences | Select-Object -First 30 | ForEach-Object { (Call $p understand $_).Ms }
"after 30 understand calls:      " + (Mem $p)
"  understand: " + (Stats $r)

$first = Call $p extract $sentences[0]
"first extract (loads GLiNER):   {0:N0} ms" -f $first.Ms
"extractor loaded:               " + (Mem $p)
$e = $sentences | ForEach-Object { (Call $p extract $_).Ms }
"after $($sentences.Count) extract calls:      " + (Mem $p)
"  extract:    " + (Stats $e)
"  sample reply: " + (Call $p extract "Tell Ved on WhatsApp I'm on my way").Reply
$p.StandardInput.Close()
$null = $p.WaitForExit(5000)

# The extractor goes away with the worker's idle exit.
$q = Start-Worker "--idle-secs 3"
$null = Call $q extract $sentences[0]
"idle test: extractor loaded:    " + (Mem $q)
$sw = [System.Diagnostics.Stopwatch]::StartNew()
$exited = $q.WaitForExit(8000)
"idle test: worker exited after idle = $exited (after {0:N1} s)" -f $sw.Elapsed.TotalSeconds
if (-not $exited) { $q.Kill() }
