#!/usr/bin/env pwsh
# Build the PyPI wheel for the current platform.
#
# Locates maturin automatically so it does not need to be on PATH.
# Usage:  ./scripts/build-wheel.ps1
#
# Output: target/wheels/sap_odata_cli-<version>-py3-none-<platform>.whl

$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

$candidates = @()

# 1. Already on PATH (normal case for most contributors).
$onPath = (Get-Command maturin -ErrorAction SilentlyContinue).Source
if ($onPath) { $candidates += $onPath }

# 2. Local tool directory used on this machine (maturin is a standalone exe;
#    `python -m maturin` does not work, so the binary itself must be found).
$candidates += Get-ChildItem 'D:\AiTools\WorkBuddy_WorkSpace\.tools\*\bin\maturin.exe' `
    -ErrorAction SilentlyContinue | ForEach-Object { $_.FullName }

# 3. cargo-installed maturin.
$candidates += (Join-Path $HOME '.cargo\bin\maturin.exe')

$maturin = $candidates | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1

if (-not $maturin) {
    Write-Host 'maturin not found.' -ForegroundColor Red
    Write-Host 'Install it with one of:' -ForegroundColor Yellow
    Write-Host '  pip install maturin'
    Write-Host '  cargo install maturin'
    exit 1
}

Write-Host "Using maturin: $maturin" -ForegroundColor DarkGray

& $maturin build --release --out target/wheels

if ($LASTEXITCODE -ne 0) {
    Write-Host 'Build failed.' -ForegroundColor Red
    exit $LASTEXITCODE
}

$wheel = Get-ChildItem target/wheels/*.whl | Sort-Object LastWriteTime -Descending | Select-Object -First 1
Write-Host ''
Write-Host "Built: $($wheel.FullName)" -ForegroundColor Green
Write-Host ''
Write-Host 'Install locally to verify:' -ForegroundColor DarkGray
Write-Host "  pip install `"$($wheel.FullName)`""
