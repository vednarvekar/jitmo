# Assembles models/router/ (model_int8.onnx, tokenizer.json, head.json) from
# models-src/router/ (committed head) plus the SP1 spike folder (encoder files).
param(
    [string]$SpikeDir = "",
    [string]$Dest = ""
)
$ErrorActionPreference = "Stop"
$repo = Resolve-Path (Join-Path $PSScriptRoot "..\..")
if (-not $SpikeDir) { $SpikeDir = Join-Path $repo "spikes\sp1-router\models" }
if (-not $Dest) { $Dest = Join-Path $repo "models\router" }

$head = Join-Path $repo "models-src\router\head.json"
if (-not (Test-Path $head)) { throw "missing $head" }
foreach ($f in "model_int8.onnx", "tokenizer.json") {
    if (-not (Test-Path (Join-Path $SpikeDir $f))) {
        throw "missing $f in $SpikeDir (pass -SpikeDir pointing at spikes\sp1-router\models of a checkout that has it; see models-src\router\README.md)"
    }
}
New-Item -ItemType Directory -Force $Dest | Out-Null
Copy-Item $head (Join-Path $Dest "head.json") -Force
foreach ($f in "model_int8.onnx", "tokenizer.json") {
    Copy-Item (Join-Path $SpikeDir $f) (Join-Path $Dest $f) -Force
}
Get-ChildItem $Dest | Select-Object Name, Length
