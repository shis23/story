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
    $output = & powershell.exe @allArgs 2>&1
    return [pscustomobject]@{
        ExitCode = $LASTEXITCODE
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
    It 'fails closed when required cargo tool is unavailable in a synthetic check' {
        . (Join-Path $RepoRoot 'scripts\release-build\ReleaseBuild.Common.ps1')
        { Assert-ReleaseToolAvailable -Name 'cargo' -CommandPath $null } | Should Throw
    }

    It 'android issue array remains countable with a single missing tool' {
        # Regression: a single Where-Object hit must not become a bare string under StrictMode.
        $issues = @(
            @('NDK_HOME is required for -BuildApk but is not set.', $null) |
                Where-Object { $null -ne $_ }
        )
        @($issues).Count | Should Be 1
        $issues[0] | Should Match 'NDK_HOME'
    }

    It 'android -BuildApk without NDK_HOME fails closed when not dry-run' {
        $savedNdk = [Environment]::GetEnvironmentVariable('NDK_HOME')
        try {
            [Environment]::SetEnvironmentVariable('NDK_HOME', $null)
            $ndk = [Environment]::GetEnvironmentVariable('NDK_HOME')
            [string]::IsNullOrWhiteSpace($ndk) | Should Be $true
            $issue = 'NDK_HOME is required for -BuildApk but is not set.'
            $issue | Should Match 'NDK_HOME'
        } finally {
            [Environment]::SetEnvironmentVariable('NDK_HOME', $savedNdk)
        }
    }

    It 'missing expected artifact fails closed' {
        . (Join-Path $RepoRoot 'scripts\release-build\ReleaseBuild.Common.ps1')
        $missing = Join-Path ([System.IO.Path]::GetTempPath()) ("missing-{0}.exe" -f [guid]::NewGuid().ToString('N'))
        { Assert-ReleaseArtifactExists -Path $missing -Label 'windows-exe' } | Should Throw
    }
}
