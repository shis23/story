<#
.SYNOPSIS
Runs local fail-closed preflight for Gitea/host release runners.

.DESCRIPTION
Checks OS-side tools and explicit intent flags, then writes a machine-readable
readiness report. Never claims remote CI, GUI, or device acceptance. Does not
register or deploy runners.

.PARAMETER Profile
windows-host | android-host | ci-gates

.PARAMETER RequireBundle
Require cargo-tauri for Windows bundle evidence (explicit authorization).

.PARAMETER RequireApk
Require ANDROID_HOME/NDK_HOME for APK evidence (explicit authorization).

.PARAMETER OutputPath
Optional path for the JSON report. Defaults to stdout only when omitted.

.PARAMETER FailClosed
Exit non-zero when the profile is not ready (default: true via exit code).

.EXAMPLE
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-release-runner-preflight.ps1 -Profile windows-host
#>
[CmdletBinding()]
param(
    [ValidateSet('windows-host', 'android-host', 'ci-gates')]
    [string]$Profile = 'windows-host',
    [switch]$RequireBundle,
    [switch]$RequireApk,
    [string]$OutputPath,
    [switch]$PassThru
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

$repoRoot = (Get-Location).ProviderPath
try {
    $gitRoot = (& git rev-parse --show-toplevel 2>$null)
    if ($LASTEXITCODE -eq 0 -and $gitRoot) {
        $repoRoot = (Resolve-Path -LiteralPath $gitRoot).ProviderPath
    }
} catch { }

$commonPath = Join-Path $repoRoot 'scripts\release-build\ReleaseBuild.Common.ps1'
if (-not (Test-Path -LiteralPath $commonPath)) {
    throw "Missing release build helpers: $commonPath"
}
. $commonPath
$repoRoot = Find-ReleaseRepoRoot

try {
    $report = Test-ReleaseRunnerPreflight `
        -Profile $Profile `
        -RepoRoot $repoRoot `
        -RequireBundle:$RequireBundle `
        -RequireApk:$RequireApk

    $json = $report | ConvertTo-Json -Depth 8
    Write-Host $json
    if (-not [string]::IsNullOrWhiteSpace($OutputPath)) {
        $dir = Split-Path -Parent $OutputPath
        if ($dir -and -not (Test-Path -LiteralPath $dir)) {
            New-Item -ItemType Directory -Force -Path $dir | Out-Null
        }
        $utf8NoBom = New-Object System.Text.UTF8Encoding $false
        [System.IO.File]::WriteAllText($OutputPath, $json, $utf8NoBom)
        Write-Host ("Wrote preflight report: {0}" -f (Protect-ReleasePath -Text $OutputPath -RepoRoot $repoRoot))
    }

    if ($PassThru) {
        return $report
    }

    if (-not $report.ready) {
        Write-Host ("PREFLIGHT NOT READY: status={0} missing={1}" -f $report.status, ($report.missing -join ',')) -ForegroundColor Yellow
        exit 2
    }

    Write-Host ("PREFLIGHT READY for profile {0} (host-only={1})" -f $report.profile, $report.intents.host_only) -ForegroundColor Green
    exit 0
} catch {
    $safe = Get-ReleaseSafeErrorDetails -ErrorRecord $_ -RepoRoot $repoRoot
    Write-Host ("PREFLIGHT FAILED: {0}" -f $safe.message) -ForegroundColor Red
    exit 1
}
