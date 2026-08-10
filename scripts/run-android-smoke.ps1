<#
.SYNOPSIS
Runs the StoryForge Android host-side smoke checks on Windows PowerShell.

.DESCRIPTION
Runs Android readiness checks in a fixed fail-fast order:
frontend build, Tauri capability tests, and Android arm64 infra-util check.

With -BuildApk, also builds debug and unsigned release arm64 APKs. The APK
build path requires ANDROID_HOME and NDK_HOME to be set to existing directories.
This script reports SDK/NDK/JAVA_HOME paths but does not require adb or a device.

.PARAMETER DryRun
Prints the steps and commands without running the smoke checks.

.PARAMETER BuildApk
Also runs debug and unsigned release APK builds.

.EXAMPLE
powershell -ExecutionPolicy Bypass -File scripts/run-android-smoke.ps1

.EXAMPLE
powershell -ExecutionPolicy Bypass -File scripts/run-android-smoke.ps1 -DryRun

.EXAMPLE
powershell -ExecutionPolicy Bypass -File scripts/run-android-smoke.ps1 -BuildApk
#>
[CmdletBinding()]
param(
    [switch]$DryRun,
    [switch]$BuildApk
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

$script:StepNumber = 0
$TotalSteps = if ($BuildApk) { 5 } else { 3 }

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

function Format-EnvPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    $value = [Environment]::GetEnvironmentVariable($Name)
    if ([string]::IsNullOrWhiteSpace($value)) {
        return '<not set>'
    }

    if (Test-Path -LiteralPath $value) {
        return ("{0} (exists)" -f $value)
    }

    return ("{0} (missing)" -f $value)
}

function Write-AndroidEnvSummary {
    Write-Host ''
    Write-Host 'Android environment summary:' -ForegroundColor Cyan
    Write-Host ("  ANDROID_HOME: {0}" -f (Format-EnvPath -Name 'ANDROID_HOME'))
    Write-Host ("  NDK_HOME:      {0}" -f (Format-EnvPath -Name 'NDK_HOME'))
    Write-Host ("  JAVA_HOME:     {0}" -f (Format-EnvPath -Name 'JAVA_HOME'))
    Write-Host '  adb/device:    not required for this host-side smoke'
}

function Get-AndroidBuildPathIssue {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    $value = [Environment]::GetEnvironmentVariable($Name)
    if ([string]::IsNullOrWhiteSpace($value)) {
        return "$Name is required for -BuildApk but is not set."
    }

    if (-not (Test-Path -LiteralPath $value -PathType Container)) {
        return "$Name is required for -BuildApk but does not point to an existing directory: $value"
    }

    return $null
}

function Assert-AndroidBuildEnv {
    $issues = @(
        Get-AndroidBuildPathIssue -Name 'ANDROID_HOME'
        Get-AndroidBuildPathIssue -Name 'NDK_HOME'
    ) | Where-Object { $null -ne $_ }

    if ($issues.Count -gt 0) {
        throw ("Android APK build environment is incomplete:{0}  {1}" -f [Environment]::NewLine, ($issues -join ([Environment]::NewLine + '  ')))
    }
}

function Start-SmokeStep {
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

    Start-SmokeStep -Name $Name

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
        if ($executable -eq 'cargo' -and ($arguments -contains 'test')) {
            # Capture cargo test output so a filter that matches nothing
            # ("running 0 tests", exit 0) is caught instead of silently passing
            # (Gate 8 review P2-D4).
            $captured = & $executable @arguments 2>&1
            $exitCode = $LASTEXITCODE
            $captured | Out-Host
            if ($exitCode -eq 0) {
                $text = $captured -join "`n"
                $passed = 0
                $failed = 0
                foreach ($m in [regex]::Matches($text, 'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed')) {
                    $passed += [int]$m.Groups[2].Value
                    $failed += [int]$m.Groups[3].Value
                }
                if ($passed -eq 0 -and $failed -eq 0) {
                    throw "Step '$Name' ran 0 tests; update or remove the stale filter."
                }
            }
        } else {
            & $executable @arguments
            $exitCode = $LASTEXITCODE
        }
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
    $tauriAppRoot = Join-Path $repoRoot 'crates\tauri-app'

    Write-Host ("StoryForge Android smoke root: {0}" -f $repoRoot)
    Write-AndroidEnvSummary

    if ($DryRun) {
        Write-Host ''
        Write-Host 'Dry run enabled; commands will be printed but not executed.'
    }

    if ($BuildApk -and -not $DryRun) {
        Assert-AndroidBuildEnv
    }

    Invoke-NativeStep -Name 'frontend npm.cmd run build' -WorkingDirectory $frontendRoot -Command @('npm.cmd', 'run', 'build')
    Invoke-NativeStep -Name 'cargo test -p storyforge --test capabilities' -WorkingDirectory $repoRoot -Command @('cargo', 'test', '-p', 'storyforge', '--test', 'capabilities')
    Invoke-NativeStep -Name 'cargo check -p storyforge-infra-util --target aarch64-linux-android' -WorkingDirectory $repoRoot -Command @('cargo', 'check', '-p', 'storyforge-infra-util', '--target', 'aarch64-linux-android')

    if ($BuildApk) {
        Invoke-NativeStep -Name 'cargo tauri android build debug arm64 APK' -WorkingDirectory $tauriAppRoot -Command @('cargo', 'tauri', 'android', 'build', '--debug', '--target', 'aarch64', '--ci', '--split-per-abi', '--apk')
        Invoke-NativeStep -Name 'cargo tauri android build unsigned release arm64 APK' -WorkingDirectory $tauriAppRoot -Command @('cargo', 'tauri', 'android', 'build', '--target', 'aarch64', '--ci', '--split-per-abi', '--apk')
    }

    Write-Host ''
    Write-Host 'Android host-side smoke passed.' -ForegroundColor Green
    exit 0
} catch {
    Write-Host ''
    Write-Error $_.Exception.Message
    exit 1
}
