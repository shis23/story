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
    $commonPath = Join-Path $repoRoot 'scripts\release-build\ReleaseBuild.Common.ps1'
    if (-not (Test-Path -LiteralPath $commonPath)) {
        throw "Release build helper file not found: $commonPath"
    }
    . $commonPath
    $testFiles = @(
        (Join-Path $repoRoot 'scripts\tests\ReleaseBuild.Tests.ps1')
        (Join-Path $repoRoot 'scripts\tests\ReleaseBuild.Pipeline.Tests.ps1')
        (Join-Path $repoRoot 'scripts\tests\ReleaseBuild.CI.Tests.ps1')
        (Join-Path $repoRoot 'scripts\tests\ReleaseBuild.RunnerReadiness.Tests.ps1')
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

    # These tracked suites intentionally retain Pester 3/4 `Should Be` syntax.
    # Pester 5 discovers the files but rejects that assertion grammar, which
    # can turn a correctly configured Windows gate into an all-red false alarm.
    $pesterModule = @(Get-Module -ListAvailable -Name Pester |
            Where-Object { $_.Version.Major -lt 5 } |
            Sort-Object Version -Descending |
            Select-Object -First 1)
    if ($pesterModule.Count -ne 1) {
        throw "No compatible Pester 3.x/4.x module is installed. Install pinned Pester 4.10.1 before running this suite."
    }

    Remove-Module Pester -Force -ErrorAction SilentlyContinue
    Import-Module $pesterModule[0].Path -Force -ErrorAction Stop
    $pesterModule = Get-Module Pester | Select-Object -First 1
    $version = $pesterModule.Version
    Write-Host ("Using Pester {0}" -f $version)

    foreach ($testFile in $testFiles) {
        $result = Invoke-Pester -Path $testFile -PassThru
        Assert-ReleasePesterResult -Result $result -Label $testFile
    }

    Write-Host 'Release build tests passed.' -ForegroundColor Green
    exit 0
} catch {
    Write-Host ''
    Write-Error $_.Exception.Message
    exit 1
}
