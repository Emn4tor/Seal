# Builds the Windows app bundles: an .msi (WiX) and an .exe (NSIS) installer.
# See README.md §1 for prerequisites (MSVC Build Tools, WebView2).
#
# Usage: .\scripts\build-windows.ps1

$ErrorActionPreference = "Stop"

if ($env:OS -ne "Windows_NT") {
    Write-Error "This script builds the Windows app and must run on Windows."
    exit 1
}

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location (Join-Path $RepoRoot "apps\desktop")

# Bakes the official Seal network in as the recommended server option on
# the first-run screen — a build-time default, not a runtime lock-in;
# "Custom server" and "Local test server" stay available regardless.
$env:SEAL_DEFAULT_DIRECTORY_URL = "https://seal.emn4tor.de"

npm install
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

npm run tauri build -- --bundles msi,nsis
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host ""
Write-Host "Done. Installers at:"
Get-ChildItem -Recurse -Path (Join-Path $RepoRoot "target\release\bundle") -Include *.msi, *.exe |
    ForEach-Object { Write-Host $_.FullName }
