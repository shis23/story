<#
.SYNOPSIS
Offline-verifies a release evidence package directory.

.DESCRIPTION
Validates manifest schema, provenance, dependency inventory, staged subjects,
and `.sha256` sidecars. Fail closed on missing subject/sidecar, hash mismatch,
BOM sidecar, path escape, reparse points, schema drift, identity mismatch,
out-of-bounds remote_ci claims, partial/failed status, or secret-like notes.

Does not claim remote CI, GUI, or device acceptance. Dry-run packages require
-AllowDryRun and are labeled as non-remote evidence. Error output is path- and
secret-redacted.

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
    # Redact any residual host paths before printing machine-readable output.
    $safeResult = Protect-ReleaseObject -Value $result -RepoRoot $repoRoot
    $json = $safeResult | ConvertTo-Json -Depth 6
    $json = Protect-ReleasePath -Text $json -RepoRoot $repoRoot
    Write-Host $json

    if (-not $result.Valid) {
        Write-Host 'EVIDENCE VERIFICATION FAILED (fail-closed)' -ForegroundColor Red
        foreach ($err in @($result.Errors)) {
            $safeErr = Protect-ReleasePath -Text ([string]$err) -RepoRoot $repoRoot
            Write-Host ("  - {0}" -f $safeErr)
        }
        if ($result.build_status -eq 'partial' -or $result.build_status -eq 'failed') {
            Write-Host ("build_status={0} is not an acceptable offline success state." -f $result.build_status) -ForegroundColor Yellow
        }
        exit 1
    }

    if ($result.build_status -eq 'partial' -or $result.build_status -eq 'failed') {
        # Defense in depth: never print PASSED for non-success statuses.
        Write-Host ("EVIDENCE VERIFICATION FAILED (fail-closed): build_status={0}" -f $result.build_status) -ForegroundColor Red
        exit 1
    }

    Write-Host ("EVIDENCE VERIFICATION PASSED (subjects={0}, remote_ci_claim={1}, remote_ci_claimed={2}, build_status={3})" -f `
        $result.subject_count, $result.remote_ci_claim, $result.remote_ci_claimed, $result.build_status) -ForegroundColor Green
    exit 0
} catch {
    $safe = Get-ReleaseSafeErrorDetails -ErrorRecord $_ -RepoRoot $repoRoot
    $msg = Protect-ReleasePath -Text $safe.message -RepoRoot $repoRoot
    Write-Host ("EVIDENCE VERIFICATION ERROR: {0}" -f $msg) -ForegroundColor Red
    exit 1
}
