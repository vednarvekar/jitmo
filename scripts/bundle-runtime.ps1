# Copies jitmo-runtime.exe into app/src-tauri/runtime/, so the next `cargo build` or
# `tauri build` puts it in runtime\ beside jitmo-app.exe (Tauri bundle resource "runtime/*").
#
# The runtime is built in its own repo:
#   cd ..\jitmo-runtime\runtime; cargo build --release
#
# Without the exe the app still builds and runs on its rules; CI never needs it.
#
# Usage: powershell -File scripts\bundle-runtime.ps1 [-Runtime <path to jitmo-runtime.exe>]
param(
    [string]$Runtime = (Join-Path $PSScriptRoot "..\..\jitmo-runtime\runtime\target\release\jitmo-runtime.exe")
)
$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $Runtime -PathType Leaf)) {
    Write-Error "jitmo-runtime.exe not found at $Runtime. Build it first (cargo build --release in jitmo-runtime\runtime) or pass -Runtime."
}
$dest = Join-Path $PSScriptRoot "..\app\src-tauri\runtime"
New-Item -ItemType Directory -Force -Path $dest | Out-Null
Copy-Item -LiteralPath $Runtime -Destination (Join-Path $dest "jitmo-runtime.exe") -Force
Write-Host "copied $((Resolve-Path -LiteralPath $Runtime).Path) -> $((Resolve-Path -LiteralPath $dest).Path)\jitmo-runtime.exe"
