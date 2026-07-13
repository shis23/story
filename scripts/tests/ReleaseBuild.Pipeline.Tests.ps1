# Pipeline-level dry-run and fail-closed checks for release build scripts.
# Run via scripts/tests/run-release-build-tests.ps1 (includes this file).

$ErrorActionPreference = 'Stop'

$RepoRoot = (& git rev-parse --show-toplevel 2>$null)
if (-not $RepoRoot) {
    throw 'Unable to locate repository root for pipeline tests.'
}
$RepoRoot = (Resolve-Path -LiteralPath $RepoRoot).ProviderPath

function Invoke-PwshFile {
    param(
        [Parameter(Mandatory = $true)][string]$File,
        [string[]]$Args = @()
    )

    $allArgs = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $File) + $Args
    $previousErrorAction = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $output = & powershell.exe @allArgs 2>&1
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousErrorAction
    }
    return [pscustomobject]@{
        ExitCode = $exitCode
        Output   = @($output | ForEach-Object { "$_" })
    }
}

Describe 'Release build script parser' {
    It 'parses run-release-build.ps1 without errors' {
        $path = Join-Path $RepoRoot 'scripts\run-release-build.ps1'
        $errors = $null
        $null = [System.Management.Automation.Language.Parser]::ParseFile($path, [ref]$null, [ref]$errors)
        @($errors).Count | Should Be 0
    }

    It 'parses run-android-host-pipeline.ps1 without errors' {
        $path = Join-Path $RepoRoot 'scripts\run-android-host-pipeline.ps1'
        $errors = $null
        $null = [System.Management.Automation.Language.Parser]::ParseFile($path, [ref]$null, [ref]$errors)
        @($errors).Count | Should Be 0
    }

    It 'parses ReleaseBuild.Common.ps1 without errors' {
        $path = Join-Path $RepoRoot 'scripts\release-build\ReleaseBuild.Common.ps1'
        $errors = $null
        $null = [System.Management.Automation.Language.Parser]::ParseFile($path, [ref]$null, [ref]$errors)
        @($errors).Count | Should Be 0
    }
}

Describe 'Release build dry-run' {
    It 'run-release-build.ps1 -DryRun exits 0 and writes dry-run manifest' {
        $outDir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-win-dry-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $outDir | Out-Null
        try {
            $scriptPath = Join-Path $RepoRoot 'scripts\run-release-build.ps1'
            $result = Invoke-PwshFile -File $scriptPath -Args @('-DryRun', '-OutputDir', $outDir)
            $result.ExitCode | Should Be 0
            ($result.Output -join "`n") | Should Match 'DRY RUN'
            ($result.Output -join "`n") | Should Match 'npm\.cmd ci'
            $manifestPath = Join-Path $outDir 'manifest.json'
            Test-Path -LiteralPath $manifestPath | Should Be $true
            $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
            $manifest.build_status | Should Be 'dry-run'
            $manifest.acceptance.gui | Should Be 'not_claimed'
            $manifest.acceptance.android_device | Should Be 'not_claimed'
            (Get-Content -LiteralPath $manifestPath -Raw) | Should Not Match 'sk-[A-Za-z0-9]{10,}'
        } finally {
            Remove-Item -LiteralPath $outDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'run-android-host-pipeline.ps1 -DryRun exits 0 and does not claim device acceptance' {
        $outDir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-and-dry-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $outDir | Out-Null
        try {
            $scriptPath = Join-Path $RepoRoot 'scripts\run-android-host-pipeline.ps1'
            $result = Invoke-PwshFile -File $scriptPath -Args @('-DryRun', '-OutputDir', $outDir)
            $result.ExitCode | Should Be 0
            ($result.Output -join "`n") | Should Match 'DRY RUN'
            ($result.Output -join "`n") | Should Match 'npm\.cmd ci'
            $manifestPath = Join-Path $outDir 'manifest.json'
            Test-Path -LiteralPath $manifestPath | Should Be $true
            $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
            $manifest.build_status | Should Be 'dry-run'
            $manifest.acceptance.android_device | Should Be 'not_claimed'
            $manifest.target | Should Be 'aarch64-linux-android'
        } finally {
            Remove-Item -LiteralPath $outDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Describe 'Release build fail-closed behavior' {
    It 'default repository secret scan passes without an allowlist' {
        . (Join-Path $RepoRoot 'scripts\release-build\ReleaseBuild.Common.ps1')
        { Invoke-ReleaseSecretScan -RepoRoot $RepoRoot } | Should Not Throw
    }

    It 'fails closed when required cargo tool is unavailable in a synthetic check' {
        . (Join-Path $RepoRoot 'scripts\release-build\ReleaseBuild.Common.ps1')
        { Assert-ReleaseToolAvailable -Name 'cargo' -CommandPath $null } | Should Throw
    }

    It 'android issue array remains countable with a single missing tool' {
        . (Join-Path $RepoRoot 'scripts\release-build\ReleaseBuild.Common.ps1')
        $issues = @(Get-ReleaseAndroidBuildPathIssues)
        # Production helper always returns an array even for a single issue.
        @($issues).Count | Should BeGreaterThan 0
    }

    It 'android -BuildApk prerequisites use production helper and fail closed without NDK_HOME' {
        . (Join-Path $RepoRoot 'scripts\release-build\ReleaseBuild.Common.ps1')
        $savedNdk = [Environment]::GetEnvironmentVariable('NDK_HOME')
        $savedAndroid = [Environment]::GetEnvironmentVariable('ANDROID_HOME')
        try {
            [Environment]::SetEnvironmentVariable('NDK_HOME', $null)
            if ([string]::IsNullOrWhiteSpace($savedAndroid)) {
                [Environment]::SetEnvironmentVariable('ANDROID_HOME', 'C:\missing-android-sdk-for-test')
            }
            $issues = @(Get-ReleaseAndroidBuildPathIssues)
            (@($issues | Where-Object { $_ -match 'NDK_HOME' }).Count) | Should BeGreaterThan 0
            { Assert-ReleaseAndroidBuildEnvironment } | Should Throw
        } finally {
            [Environment]::SetEnvironmentVariable('NDK_HOME', $savedNdk)
            [Environment]::SetEnvironmentVariable('ANDROID_HOME', $savedAndroid)
        }
    }

    It 'missing expected artifact fails closed' {
        . (Join-Path $RepoRoot 'scripts\release-build\ReleaseBuild.Common.ps1')
        $missing = Join-Path ([System.IO.Path]::GetTempPath()) ("missing-{0}.exe" -f [guid]::NewGuid().ToString('N'))
        { Assert-ReleaseArtifactExists -Path $missing -Label 'windows-exe' } | Should Throw
    }

    It 'partial and failed statuses are fail-closed for process exit' {
        . (Join-Path $RepoRoot 'scripts\release-build\ReleaseBuild.Common.ps1')
        (Get-ReleaseProcessExitCode -BuildStatus 'partial') | Should Not Be 0
        (Get-ReleaseProcessExitCode -BuildStatus 'failed') | Should Not Be 0
        (Get-ReleaseProcessExitCode -BuildStatus 'ok') | Should Be 0
        (Get-ReleaseProcessExitCode -BuildStatus 'dry-run') | Should Be 0
    }

    It 'frontend dependency install policy is npm ci only' {
        $scriptPath = Join-Path $RepoRoot 'scripts\run-release-build.ps1'
        $text = Get-Content -LiteralPath $scriptPath -Raw
        $text | Should Match 'npm\.cmd'', ''ci'''
        $text | Should Not Match "npm\.cmd', 'install'"
        $text | Should Not Match 'npm\.cmd", "install"'
    }

    It 'production Android -BuildApk fails before host builds when prerequisites are missing' {
        $outDir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-and-fail-{0}" -f [guid]::NewGuid().ToString('N'))
        $savedNdk = [Environment]::GetEnvironmentVariable('NDK_HOME')
        $savedAndroid = [Environment]::GetEnvironmentVariable('ANDROID_HOME')
        try {
            [Environment]::SetEnvironmentVariable('NDK_HOME', $null)
            [Environment]::SetEnvironmentVariable('ANDROID_HOME', 'C:\missing-android-sdk-for-production-test')
            $scriptPath = Join-Path $RepoRoot 'scripts\run-android-host-pipeline.ps1'
            $result = Invoke-PwshFile -File $scriptPath -Args @('-BuildApk', '-SkipSecretScan', '-OutputDir', $outDir)
            $result.ExitCode | Should Not Be 0
            ($result.Output -join "`n") | Should Match 'Android APK build environment is incomplete'
            ($result.Output -join "`n") | Should Not Match 'frontend npm\.cmd run build'
        } finally {
            [Environment]::SetEnvironmentVariable('NDK_HOME', $savedNdk)
            [Environment]::SetEnvironmentVariable('ANDROID_HOME', $savedAndroid)
            Remove-Item -LiteralPath $outDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}
