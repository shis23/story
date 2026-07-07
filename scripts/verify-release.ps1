<#
.SYNOPSIS
Runs the StoryForge release gate on Windows PowerShell.

.DESCRIPTION
Runs the release checks in a fixed fail-fast order:
secret scan, cargo fmt, cargo clippy, cargo test, frontend tests, and frontend build.

The secret scan checks Git-tracked files only and excludes target, node_modules,
frontend/dist, and .git. It reports rule names and locations without echoing
matching line contents.

.PARAMETER DryRun
Prints the steps and commands without running the release gate checks.

.PARAMETER SecretScanOnly
Runs only the secret scan step. Useful for quick pre-commit verification.

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
    [switch]$SecretScanOnly
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

$script:StepNumber = 0
$TotalSteps = if ($SecretScanOnly) { 1 } else { 6 }

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

function Invoke-SecretScan {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RepoRoot
    )

    Start-ReleaseStep -Name 'secret scan'

    $pathspecs = @(
        '.',
        ':(exclude)target/**',
        ':(exclude)node_modules/**',
        ':(exclude)frontend/dist/**',
        ':(exclude).git/**'
    )

    if ($DryRun) {
        Write-Host 'DRY RUN: scan Git-tracked files for common secret patterns'
        Write-Host ("DRY RUN: excludes {0}" -f (($pathspecs | Where-Object { $_ -like ':(exclude)*' }) -join ', '))
        return
    }

    $rules = @(
        @{
            Name = 'private key block'
            Pattern = '-----BEGIN (RSA|DSA|EC|OPENSSH|PGP) PRIVATE KEY-----'
        },
        @{
            Name = 'AWS access key id'
            Pattern = 'AKIA[0-9A-Z]{16}'
        },
        @{
            Name = 'OpenAI-style API key'
            Pattern = 'sk-[A-Za-z0-9_-]{20,}'
        },
        @{
            Name = 'Slack token'
            Pattern = 'xox[baprs]-[0-9A-Za-z-]{10,}'
        },
        @{
            Name = 'authorization header'
            Pattern = '(Authorization|X-Api-Key)[[:space:]]*:[[:space:]]*(token|Bearer|Basic)?[[:space:]]*[A-Za-z0-9_./+=-]{20,}'
        },
        @{
            Name = 'secret assignment'
            Pattern = '(api[_-]?key|secret|token|password|passwd|authorization)[[:space:]]*[:=][[:space:]]*[''"][^''"]{16,}[''"]'
        }
    )

    $findings = New-Object System.Collections.Generic.List[string]
    $scanTargets = @(
        @{
            Name = 'worktree'
            Args = @()
        },
        @{
            Name = 'index'
            Args = @('--cached')
        }
    )

    foreach ($target in $scanTargets) {
        foreach ($rule in $rules) {
            $output = & git -C $RepoRoot grep @($target.Args) -n -I -E -e $($rule.Pattern) -- @pathspecs 2>&1
            $exitCode = $LASTEXITCODE

            if ($exitCode -eq 1) {
                continue
            }

            if ($exitCode -ne 0) {
                throw "Secret scan failed while running $($target.Name) rule '$($rule.Name)': $($output -join [Environment]::NewLine)"
            }

            foreach ($line in $output) {
                if ($line -match '^(.+?):([0-9]+):') {
                    $findings.Add(("{0} {1} at {2}:{3}" -f $target.Name, $rule.Name, $Matches[1], $Matches[2]))
                } else {
                    $findings.Add(("{0} {1} at unknown location" -f $target.Name, $rule.Name))
                }
            }
        }
    }

    if ($findings.Count -gt 0) {
        Write-Host 'Potential secret material found:' -ForegroundColor Red
        $findings | Sort-Object -Unique | ForEach-Object {
            Write-Host ("  {0}" -f $_)
        }

        throw 'Secret scan failed. Remove the secret material or replace it with a safe reference before releasing.'
    }

    Write-Host 'OK: secret scan found no matches in Git-tracked files.'
}

try {
    $repoRoot = Find-RepoRoot
    $frontendRoot = Join-Path $repoRoot 'frontend'

    Write-Host ("StoryForge release gate root: {0}" -f $repoRoot)
    if ($DryRun) {
        Write-Host 'Dry run enabled; commands will be printed but not executed.'
    }

    Invoke-SecretScan -RepoRoot $repoRoot
    if ($SecretScanOnly) {
        Write-Host ''
        Write-Host 'Secret scan passed.' -ForegroundColor Green
        exit 0
    }

    Invoke-NativeStep -Name 'cargo fmt --check' -WorkingDirectory $repoRoot -Command @('cargo', 'fmt', '--check')
    Invoke-NativeStep -Name 'cargo clippy --workspace --all-targets -- -D warnings' -WorkingDirectory $repoRoot -Command @('cargo', 'clippy', '--workspace', '--all-targets', '--', '-D', 'warnings')
    Invoke-NativeStep -Name 'cargo test --workspace' -WorkingDirectory $repoRoot -Command @('cargo', 'test', '--workspace')
    Invoke-NativeStep -Name 'frontend npm.cmd test' -WorkingDirectory $frontendRoot -Command @('npm.cmd', 'test')
    Invoke-NativeStep -Name 'frontend npm.cmd run build' -WorkingDirectory $frontendRoot -Command @('npm.cmd', 'run', 'build')

    Write-Host ''
    Write-Host 'Release gate passed.' -ForegroundColor Green
    exit 0
} catch {
    Write-Host ''
    Write-Error $_.Exception.Message
    exit 1
}
