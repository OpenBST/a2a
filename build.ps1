# build.ps1 — release-build + sync exe to D:\tools\a2a\a2a.exe
#
# Cargo always emits the binary at <target-dir>\release\a2a.exe (here:
# D:\tools\a2a\target\release\a2a.exe per .cargo/config.toml). For
# day-to-day double-click / `a2a` invocations we want a stable copy at
# D:\tools\a2a\a2a.exe — that's the path the welcome wizard adds to
# user PATH on first double-click. This script wraps cargo build and
# keeps that copy in sync.
#
# Usage (from D:\a2a or anywhere):
#   .\build.ps1                  # kill running a2a, build, copy
#   .\build.ps1 -SkipKill        # don't kill running a2a (will fail if .exe locked)
#   .\build.ps1 -SkipCopy        # build only; don't refresh D:\tools\a2a\a2a.exe

[CmdletBinding()]
param(
    [switch]$SkipKill,
    [switch]$SkipCopy
)

$ErrorActionPreference = 'Stop'

# Resolve repo root from this script's location so it works no matter
# where the user invokes it from.
$repoRoot = Split-Path -Parent $MyInvocation.MyCommand.Definition

# Kill any running a2a so we don't hit "failed to remove file ... a2a.exe
# (os error 5)" mid-build. .cargo/config.toml redirects target-dir to
# D:\tools\a2a\target so the lock would be on the file we're about to
# overwrite.
if (-not $SkipKill) {
    Get-Process -Name a2a -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Milliseconds 500
}

Push-Location $repoRoot
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) {
        Write-Host ""
        Write-Host "[!!] cargo build failed (exit $LASTEXITCODE). D:\tools\a2a\a2a.exe NOT refreshed." -ForegroundColor Red
        exit $LASTEXITCODE
    }
} finally {
    Pop-Location
}

if ($SkipCopy) {
    Write-Host ""
    Write-Host "[ok] build done; -SkipCopy: D:\tools\a2a\a2a.exe NOT refreshed." -ForegroundColor Green
    exit 0
}

$src = 'D:\tools\a2a\target\release\a2a.exe'
$dst = 'D:\tools\a2a\a2a.exe'

if (-not (Test-Path $src)) {
    Write-Host ""
    Write-Host "[!!] expected build artifact missing: $src" -ForegroundColor Red
    Write-Host "     (check .cargo/config.toml's target-dir setting)"
    exit 2
}

Copy-Item -Path $src -Destination $dst -Force
Write-Host ""
Write-Host "[ok] copied $src" -ForegroundColor Green
Write-Host "     -> $dst"
Write-Host ""
Write-Host "Both copies are byte-identical until the next cargo build. The"
Write-Host "double-click-friendly path is $dst (welcome wizard adds D:\tools\a2a to PATH)."
