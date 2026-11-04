# Builds Nimbus-Setup.exe for the native client.
# Needs: the release build of nimbus-native.exe, the server folder (LiveKit + token service),
# and NSIS (the one Tauri already downloaded, or any makensis on PATH).
# Run: powershell -ExecutionPolicy Bypass -File installer\build.ps1

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$native = Split-Path -Parent $here
$root = Split-Path -Parent $native

$exe = Join-Path $env:LOCALAPPDATA "nimbus-native-target\release\nimbus-native.exe"
$server = Join-Path $env:LOCALAPPDATA "Nimbus Native\server"
$icon = Join-Path $native "assets\icon.ico"
$out = Join-Path $root "dist\Nimbus-Setup.exe"

$make = (Get-Command makensis -ErrorAction SilentlyContinue).Source
if (-not $make) { $make = Join-Path $env:LOCALAPPDATA "tauri\NSIS\makensis.exe" }
foreach ($p in @($exe, $server, $icon, $make)) { if (-not (Test-Path $p)) { throw "Missing: $p" } }

New-Item -ItemType Directory -Force (Split-Path $out) | Out-Null
& $make "/DEXE=$exe" "/DSERVER=$server" "/DICON=$icon" "/DOUT=$out" (Join-Path $here "nimbus-native.nsi")
if ($LASTEXITCODE -ne 0) { throw "makensis failed ($LASTEXITCODE)" }
Write-Host "Made $out"
