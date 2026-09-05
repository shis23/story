<#
.SYNOPSIS
Runs the StoryForge release gate on Windows PowerShell.

.DESCRIPTION
Runs the release checks in a fixed fail-fast order:
secret scan, release-helper tests, cargo fmt, cargo clippy, cargo test,
frontend unit tests, component tests, browser suites and frontend build.

The secret scan checks tracked and untracked build inputs and excludes target,
node_modules, frontend/dist, and .git. It reports locations without echoing
matching line contents. Pass -EvidenceRoot to additionally scan a repo-external
evidence directory (Gate 8 review P1-2: evidence may hold real proxy keys below
the long-key threshold).

.PARAMETER DryRun
Prints the steps and commands without running the release gate checks.

.PARAMETER SecretScanOnly
Runs only the secret scan step. Useful for quick pre-commit verification.

.PARAMETER EvidenceRoot
Optional repo-external directory to include in the secret scan (e.g. the
storyforge-evidence tree). Defaults to $env:STORYFORGE_EVIDENCE_ROOT.

.EXAMPLE
powershell -ExecutionPolicy Bypass -File scripts/verify-release.ps1

.EXAMPLE
powershell -ExecutionPolicy Bypass -File scripts/verify-release.ps1 -DryRun

.EXAMPLE
powershell -ExecutionPolicy Bypass -File scripts/verify-release.ps1 -SecretScanOnly
#>
[CmdletBinding()]
param(
    [switch]$DryRun,
    [switch]$SecretScanOnly,
    [string]$EvidenceRoot
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

$script:StepNumber = 0
$TotalSteps = if ($SecretScanOnly) { 1 } else { 11 }

function Find-RepoRoot {
    $start = (Get-Location).ProviderPath

    try {
        $gitRoot = (& git -C $start rev-parse --show-toplevel 2>$null)
        if ($LASTEXITCODE -eq 0 -and $gitRoot) {
            return (Resolve-Path -LiteralPath $gitRoot).ProviderPath
        }
    } catch {
        # Fall through to marker-based discovery.
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

function Format-Command {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Command
    )

    return ($Command | ForEach-Object {
        if ($_ -match '\s') {
            '"' + ($_ -replace '"', '\"') + '"'
        } else {
            $_
        }
    }) -join ' '
}

function Start-ReleaseStep {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    $script:StepNumber += 1
    Write-Host ''
    Write-Host ("[{0}/{1}] {2}" -f $script:StepNumber, $TotalSteps, $Name) -ForegroundColor Cyan
}

function Invoke-NativeStep {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,

        [Parameter(Mandatory = $true)]
        [string]$WorkingDirectory,

        [Parameter(Mandatory = $true)]
        [string[]]$Command
    )

    Start-ReleaseStep -Name $Name

    if ($DryRun) {
        Write-Host ("DRY RUN: cd {0}" -f $WorkingDirectory)
        Write-Host ("DRY RUN: {0}" -f (Format-Command -Command $Command))
        return
    }

    Push-Location -LiteralPath $WorkingDirectory
    try {
        $executable = $Command[0]
        $arguments = @()
        if ($Command.Count -gt 1) {
            $arguments = $Command[1..($Command.Count - 1)]
        }

        Write-Host ("RUN: {0}" -f (Format-Command -Command $Command))
        & $executable @arguments
        $exitCode = $LASTEXITCODE
        if ($exitCode -ne 0) {
            throw "Step '$Name' failed with exit code $exitCode."
        }
    } finally {
        Pop-Location
    }
}

try {
    $repoRoot = Find-RepoRoot
    $frontendRoot = Join-Path $repoRoot 'frontend'

    # Reuse production secret-scan helper (tracked + untracked build inputs).
    $commonPath = Join-Path $repoRoot 'scripts\release-build\ReleaseBuild.Common.ps1'
    if (-not (Test-Path -LiteralPath $commonPath)) {
        throw "Missing release build helpers: $commonPath"
    }
    . $commonPath

    Write-Host ("StoryForge release gate root: {0}" -f $repoRoot)
    if ($DryRun) {
        Write-Host 'Dry run enabled; commands will be printed but not executed.'
    }

    Start-ReleaseStep -Name 'secret scan'
    if ($DryRun) {
        Write-Host 'DRY RUN: scan Git-tracked and untracked build-input files for common secret patterns'
    } elseif ($EvidenceRoot -or $env:STORYFORGE_EVIDENCE_ROOT) {
        $scanRoot = if ($EvidenceRoot) { $EvidenceRoot } else { $env:STORYFORGE_EVIDENCE_ROOT }
        Invoke-ReleaseSecretScan -RepoRoot $repoRoot -EvidenceRoots $scanRoot
    } else {
        Invoke-ReleaseSecretScan -RepoRoot $repoRoot
    }
    if ($SecretScanOnly) {
        Write-Host ''
        Write-Host 'Secret scan passed.' -ForegroundColor Green
        exit 0
    }

    Invoke-NativeStep -Name 'release helper contracts (Pester)' -WorkingDirectory $repoRoot -Command @('powershell', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $repoRoot 'scripts\tests\run-release-build-tests.ps1'))
    Invoke-NativeStep -Name 'cargo fmt --check' -WorkingDirectory $repoRoot -Command @('cargo', 'fmt', '--check')
    Invoke-NativeStep -Name 'cargo clippy --workspace --all-targets -- -D warnings' -WorkingDirectory $repoRoot -Command @('cargo', 'clippy', '--workspace', '--all-targets', '--', '-D', 'warnings')
    Invoke-NativeStep -Name 'cargo test --workspace' -WorkingDirectory $repoRoot -Command @('cargo', 'test', '--workspace')
    Invoke-NativeStep -Name 'frontend npm.cmd test' -WorkingDirectory $frontendRoot -Command @('npm.cmd', 'test')
    Invoke-NativeStep -Name 'frontend npm.cmd run test:ui (vitest)' -WorkingDirectory $frontendRoot -Command @('npm.cmd', 'run', 'test:ui')
    # Gate 8 review F domain P2: the real Chromium CSP gate previously lived
    # only in package.json and ran in no automatic gate; wire it in so real
    # browser enforcement is covered, not just jsdom/string-level CSP checks.
    Invoke-NativeStep -Name 'frontend npm.cmd run test:csp (Chromium)' -WorkingDirectory $frontendRoot -Command @('npm.cmd', 'run', 'test:csp')
    Invoke-NativeStep -Name 'frontend npm.cmd run test:mobile-chrome' -WorkingDirectory $frontendRoot -Command @('npm.cmd', 'run', 'test:mobile-chrome')
    Invoke-NativeStep -Name 'frontend npm.cmd run smoke:ui' -WorkingDirectory $frontendRoot -Command @('npm.cmd', 'run', 'smoke:ui')
    Invoke-NativeStep -Name 'frontend npm.cmd run build' -WorkingDirectory $frontendRoot -Command @('npm.cmd', 'run', 'build')

    Write-Host ''
    Write-Host 'Release gate passed.' -ForegroundColor Green
    exit 0
} catch {
    Write-Host ''
    Write-Error $_.Exception.Message
    exit 1
}
