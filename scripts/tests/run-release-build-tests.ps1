<#
.SYNOPSIS
Runs PowerShell unit tests for the release build pipeline helpers.

.DESCRIPTION
Invokes Pester against scripts/tests/ReleaseBuild.Tests.ps1. Fail-closed:
any failed or skipped-as-error assertion exits non-zero.

.PARAMETER DryRun
Prints the Pester invocation without running tests.

.EXAMPLE
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/run-release-build-tests.ps1
#>
[CmdletBinding()]
param(
    [switch]$DryRun
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

function Find-RepoRoot {
    $start = (Get-Location).ProviderPath
    try {
        $gitRoot = (& git -C $start rev-parse --show-toplevel 2>$null)
        if ($LASTEXITCODE -eq 0 -and $gitRoot) {
            return (Resolve-Path -LiteralPath $gitRoot).ProviderPath
        }
    } catch {
        # Fall through.
    }

    $dir = Get-Item -LiteralPath $start
    while ($null -ne $dir) {
        $gitMarker = Join-Path $dir.FullName '.git'
        $cargoToml = Join-Path $dir.FullName 'Cargo.toml'
        if ((Test-Path -LiteralPath $gitMarker) -and (Test-Path -LiteralPath $cargoToml)) {
            return $dir.FullName
        }
        $dir = $dir.Parent
    }

    throw "Unable to locate the StoryForge repository root from '$start'."
}

try {
    $repoRoot = Find-RepoRoot
    $testFiles = @(
        (Join-Path $repoRoot 'scripts\tests\ReleaseBuild.Tests.ps1')
        (Join-Path $repoRoot 'scripts\tests\ReleaseBuild.Pipeline.Tests.ps1')
    )

    foreach ($testFile in $testFiles) {
        if (-not (Test-Path -LiteralPath $testFile)) {
            throw "Release build test file not found: $testFile"
        }
    }

    Write-Host ("StoryForge release-build tests root: {0}" -f $repoRoot)
    foreach ($testFile in $testFiles) {
        Write-Host ("Test file: {0}" -f $testFile)
    }

    if ($DryRun) {
        Write-Host 'DRY RUN: Import-Module Pester; Invoke-Pester <test-files>'
        exit 0
    }

    if (-not (Get-Module -ListAvailable -Name Pester)) {
        throw 'Pester module is not installed. Install with: Install-Module Pester -Scope CurrentUser -Force'
    }

    Import-Module Pester -ErrorAction Stop

    # Support both Pester 3.x (Windows default) and 4+/5+ if present.
    $pesterModule = Get-Module Pester
    $version = $pesterModule.Version
    Write-Host ("Using Pester {0}" -f $version)

    $totalFailed = 0
    if ($version.Major -ge 5) {
        $config = New-PesterConfiguration
        $config.Run.Path = $testFiles
        $config.Run.Exit = $false
        $config.Output.Verbosity = 'Detailed'
        $result = Invoke-Pester -Configuration $config
        if ($result.FailedCount -gt 0) {
            $totalFailed = [int]$result.FailedCount
        }
    } else {
        foreach ($testFile in $testFiles) {
            $result = Invoke-Pester -Path $testFile -PassThru
            if ($null -eq $result) {
                throw "Pester returned no result object for $testFile"
            }
            if ($result.PSObject.Properties.Name -contains 'FailedCount') {
                $totalFailed += [int]$result.FailedCount
            } elseif ($result.PSObject.Properties.Name -contains 'Failed') {
                $totalFailed += [int]$result.Failed.Count
            }
        }
    }

    if ($totalFailed -gt 0) {
        throw ("Release build tests failed: {0} failed assertion(s)." -f $totalFailed)
    }

    Write-Host 'Release build tests passed.' -ForegroundColor Green
    exit 0
} catch {
    Write-Host ''
    Write-Error $_.Exception.Message
    exit 1
}
