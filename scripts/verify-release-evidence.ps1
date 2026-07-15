<#
.SYNOPSIS
Offline-verifies a release evidence package directory.

.DESCRIPTION
Validates manifest schema, provenance, dependency inventory, staged subjects,
and `.sha256` sidecars. Fail closed on missing subject/sidecar, hash mismatch,
BOM sidecar, path escape, schema drift, or secret-like notes/warnings.

Does not claim remote CI, GUI, or device acceptance. Dry-run packages require
-AllowDryRun and are labeled as non-remote evidence.

.PARAMETER EvidenceDir
Path to an evidence directory containing manifest.json.

.PARAMETER AllowDryRun
Accept local dry-run packages (still not remote CI).

.EXAMPLE
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-release-evidence.ps1 -EvidenceDir artifacts/release-build/windows-...
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$EvidenceDir,
    [switch]$AllowDryRun
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
    $result = Test-ReleaseEvidencePackage -EvidenceDir $EvidenceDir -AllowDryRun:$AllowDryRun
    $json = $result | ConvertTo-Json -Depth 6
    Write-Host $json
    if (-not $result.Valid) {
        Write-Host 'EVIDENCE VERIFICATION FAILED' -ForegroundColor Red
        foreach ($err in @($result.Errors)) {
            Write-Host ("  - {0}" -f $err)
        }
        exit 1
    }

    Write-Host ("EVIDENCE VERIFICATION PASSED (subjects={0}, remote_ci_claimed={1}, build_status={2})" -f `
        $result.subject_count, $result.remote_ci_claimed, $result.build_status) -ForegroundColor Green
    exit 0
} catch {
    $safe = Get-ReleaseSafeErrorDetails -ErrorRecord $_ -RepoRoot $repoRoot
    Write-Host ("EVIDENCE VERIFICATION ERROR: {0}" -f $safe.message) -ForegroundColor Red
    exit 1
}
