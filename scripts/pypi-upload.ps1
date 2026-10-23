#!/usr/bin/env pwsh
# Upload the built wheels to TestPyPI or PyPI.
#
# Usage:
#   ./scripts/pypi-upload.ps1 -Target testpypi -Token $env:TWINE_PASSWORD
#   ./scripts/pypi-upload.ps1 -Target pypi
#
# Run ./scripts/build-wheel.ps1 first if target/wheels is empty.
#
# Notes:
#   * PyPI and TestPyPI both reject username/password now -- only API tokens
#     (or OIDC trusted publishing) work. The username must be `__token__`.
#   * TestPyPI and PyPI are separate accounts; a TestPyPI token will not
#     authenticate against pypi.org.

param(
    [ValidateSet('testpypi', 'pypi')]
    [string]$Target = 'testpypi',

    [string]$Token
)

$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

$wheels = Get-ChildItem target/wheels/*.whl -ErrorAction SilentlyContinue
if (-not $wheels) {
    Write-Host 'No wheels in target/wheels. Run ./scripts/build-wheel.ps1 first.' -ForegroundColor Red
    exit 1
}

if (-not $Token) { $Token = $env:TWINE_PASSWORD }
if (-not $Token) {
    Write-Host 'No token supplied.' -ForegroundColor Red
    Write-Host '  -Token "pypi-..."   or   $env:TWINE_PASSWORD = "pypi-..."'
    Write-Host '  TestPyPI: https://test.pypi.org/manage/account/token/'
    Write-Host '  PyPI:     https://pypi.org/manage/account/token/'
    exit 1
}

$url = if ($Target -eq 'testpypi') {
    'https://test.pypi.org/legacy/'
} else {
    'https://upload.pypi.org/legacy/'
}

# Prefer twine on PATH; fall back to the local tool directory (twine is a
# pure-Python package, so it is invoked as `python -m twine` with PYTHONPATH).
$twineOnPath = Get-Command twine -ErrorAction SilentlyContinue
$useModule   = $false
if (-not $twineOnPath) {
    $localTwine = 'D:\AiTools\WorkBuddy_WorkSpace\.tools\twine'
    if (-not (Test-Path $localTwine)) {
        Write-Host 'twine not found. Install with: pip install twine' -ForegroundColor Red
        exit 1
    }
    $env:PYTHONPATH = $localTwine
    $useModule = $true
}

function Invoke-Twine {
    param([string[]]$TwineArgs)
    if ($useModule) {
        & python -m twine @TwineArgs
    } else {
        & twine @TwineArgs
    }
}

$env:TWINE_USERNAME = '__token__'
$env:TWINE_PASSWORD = $Token

Write-Host "Target : $Target" -ForegroundColor DarkGray
Write-Host "Upload : $url" -ForegroundColor DarkGray
Write-Host "Files  : $($wheels.Count) wheel(s)" -ForegroundColor DarkGray
Write-Host ''

Write-Host 'Checking metadata...' -ForegroundColor DarkGray
Invoke-Twine @('check', $wheels.FullName)
if ($LASTEXITCODE -ne 0) {
    Write-Host 'twine check failed -- fix metadata before uploading.' -ForegroundColor Red
    exit $LASTEXITCODE
}

Write-Host ''
Write-Host 'Uploading...' -ForegroundColor DarkGray
Invoke-Twine (@('upload', '--repository-url', $url, '--skip-existing') + $wheels.FullName)

if ($LASTEXITCODE -eq 0) {
    Write-Host ''
    Write-Host 'Uploaded.' -ForegroundColor Green
    if ($Target -eq 'testpypi') {
        Write-Host 'Verify install with both indexes (deps are not on TestPyPI):' -ForegroundColor DarkGray
        Write-Host '  pip install --index-url https://test.pypi.org/simple/ --extra-index-url https://pypi.org/simple/ sap-odata-cli-hsin'
    } else {
        Write-Host 'Verify install with:  pip install sap-odata-cli-hsin' -ForegroundColor DarkGray
    }
} else {
    Write-Host 'Upload failed. See the error above.' -ForegroundColor Red
    exit $LASTEXITCODE
}
