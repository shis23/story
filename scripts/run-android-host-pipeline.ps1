<#
.SYNOPSIS
Runs Android host-side release checks and optional APK evidence collection.

.DESCRIPTION
Reuses the fail-closed Android host smoke path (frontend build, capability
tests, arm64 infra-util check). When -BuildApk is set and ANDROID_HOME/NDK_HOME
permit, builds debug/release arm64 APKs, inspects ABI/native/SQLite entries,
normalizes Gradle/Kotlin/Tauri/proguard warnings, applies size budgets, and
writes a host-only evidence manifest.

Does not install to a device, does not sign/publish, and does not claim device
acceptance.

.PARAMETER DryRun
Print planned steps without executing builds.

.PARAMETER BuildApk
Attempt debug and unsigned release arm64 APK builds when SDK/NDK are present.

.PARAMETER SkipSecretScan
Skip the fail-closed Git-tracked secret scan. Prefer leaving this off for release evidence.

.PARAMETER KeepRuns
Number of previous android evidence runs to retain (default 5).

.PARAMETER OutputDir
Optional explicit evidence directory.

.EXAMPLE
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-android-host-pipeline.ps1 -DryRun

.EXAMPLE
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-android-host-pipeline.ps1 -BuildApk
#>
[CmdletBinding()]
param(
    [switch]$DryRun,
    [switch]$BuildApk,
    [switch]$SkipSecretScan,
    [int]$KeepRuns = 5,
    [string]$OutputDir
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

$script:StepNumber = 0
$script:Warnings = New-Object System.Collections.Generic.List[string]
$script:Notes = New-Object System.Collections.Generic.List[string]
$script:Artifacts = New-Object System.Collections.Generic.List[object]
$script:WarningReports = New-Object System.Collections.Generic.List[object]
$script:ApkInspections = New-Object System.Collections.Generic.List[object]

# Resolve repo root early so helpers can be dot-sourced into script scope.
$script:RepoRoot = (Get-Location).ProviderPath
try {
    $gitRoot = (& git rev-parse --show-toplevel 2>$null)
    if ($LASTEXITCODE -eq 0 -and $gitRoot) {
        $script:RepoRoot = (Resolve-Path -LiteralPath $gitRoot).ProviderPath
    }
} catch { }

$commonPath = Join-Path $script:RepoRoot 'scripts\release-build\ReleaseBuild.Common.ps1'
if (-not (Test-Path -LiteralPath $commonPath)) {
    throw "Missing release build helpers: $commonPath"
}
. $commonPath
$script:RepoRoot = Find-ReleaseRepoRoot

function Start-AndroidHostStep {
    param([Parameter(Mandatory = $true)][string]$Name)
    $script:StepNumber += 1
    Write-Host ''
    Write-Host ("[{0}] {1}" -f $script:StepNumber, $Name) -ForegroundColor Cyan
}

function Format-EnvPathState {
    param([Parameter(Mandatory = $true)][string]$Name)
    $value = [Environment]::GetEnvironmentVariable($Name)
    if ([string]::IsNullOrWhiteSpace($value)) { return '<not set>' }
    if (Test-Path -LiteralPath $value) { return (Protect-ReleasePath -Text ("{0} (exists)" -f $value) -RepoRoot $script:RepoRoot) }
    return (Protect-ReleasePath -Text ("{0} (missing)" -f $value) -RepoRoot $script:RepoRoot)
}

function Invoke-AndroidHostCommand {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [Parameter(Mandatory = $true)][string[]]$Command,
        [string]$LogPath,
        [switch]$AllowFail
    )

    Start-AndroidHostStep -Name $Name
    $formatted = Format-ReleaseCommand -Command $Command
    $safeFormatted = Protect-ReleasePath -Text $formatted -RepoRoot $script:RepoRoot
    if ($DryRun) {
        Write-Host ("DRY RUN: cd {0}" -f (Protect-ReleasePath -Text $WorkingDirectory -RepoRoot $script:RepoRoot))
        Write-Host ("DRY RUN: {0}" -f $safeFormatted)
        return @{ ExitCode = 0; Lines = @() }
    }

    Push-Location -LiteralPath $WorkingDirectory
    try {
        Write-Host ("RUN: {0}" -f $safeFormatted)
        # Native tools often write warnings to stderr. Capture without turning
        # those records into terminating errors under $ErrorActionPreference=Stop.
        $prevEap = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try {
            $output = & $Command[0] @($Command[1..($Command.Count - 1)]) 2>&1
            $code = $LASTEXITCODE
        } finally {
            $ErrorActionPreference = $prevEap
        }
        $lines = @($output | ForEach-Object {
            Protect-ReleasePath -Text "$_" -RepoRoot $script:RepoRoot
        })
        if ($LogPath) {
            $lines | Set-Content -LiteralPath $LogPath -Encoding utf8
        }
        foreach ($line in $lines) {
            Write-Host $line
        }
        if ($null -eq $code) { $code = 0 }
        if ($code -ne 0 -and -not $AllowFail) {
            Assert-ReleaseExitCode -ExitCode $code -StepName $Name
        }
        return @{ ExitCode = $code; Lines = $lines }
    } finally {
        Pop-Location
    }
}

function Get-ZipEntryNames {
    param([Parameter(Mandatory = $true)][string]$ZipPath)

    Add-Type -AssemblyName System.IO.Compression.FileSystem -ErrorAction SilentlyContinue
    $zip = [System.IO.Compression.ZipFile]::OpenRead($ZipPath)
    try {
        return @($zip.Entries | ForEach-Object { $_.FullName })
    } finally {
        $zip.Dispose()
    }
}

function Add-ApkArtifactAndInspect {
    param(
        [Parameter(Mandatory = $true)][string]$ApkPath,
        [Parameter(Mandatory = $true)][string]$Kind,
        [datetime]$NotBeforeUtc,
        [switch]$RequireFresh
    )

    if (-not (Test-Path -LiteralPath $ApkPath -PathType Leaf)) {
        $rel = Get-RelativeReleasePath -RepoRoot $script:RepoRoot -FullPath $ApkPath
        $script:Artifacts.Add((New-ReleaseArtifactRecord -RelativePath $rel -SizeBytes 0 -Sha256 $null -Kind $Kind -Status 'missing'))
        return $false
    }

    $item = Get-Item -LiteralPath $ApkPath
    if ($RequireFresh -and -not (Test-ReleaseArtifactIsFresh -FileInfo $item -NotBeforeUtc $NotBeforeUtc)) {
        $relStale = Get-RelativeReleasePath -RepoRoot $script:RepoRoot -FullPath $item.FullName
        $script:Warnings.Add(("stale APK ignored for current SHA evidence: {0}" -f $relStale))
        Write-Host ("WARNING: ignoring stale APK (older than build start): {0}" -f $relStale) -ForegroundColor Yellow
        return $false
    }

    $rel = Get-RelativeReleasePath -RepoRoot $script:RepoRoot -FullPath $item.FullName
    $sha = Get-ReleaseFileSha256 -Path $item.FullName
    $script:Artifacts.Add((New-ReleaseArtifactRecord -RelativePath $rel -SizeBytes ([long]$item.Length) -Sha256 $sha -Kind $Kind -Status 'present'))

    $budgets = Get-ReleaseSizeBudgets
    if ($budgets.Contains($Kind)) {
        $budgetResult = Get-ReleaseSizeBudgetResult -Label $Kind -SizeBytes ([long]$item.Length) -BudgetBytes ([long]$budgets[$Kind])
        if ($budgetResult.Status -eq 'warning' -and $budgetResult.Warning) {
            $script:Warnings.Add($budgetResult.Warning)
            Write-Host ("WARNING: {0}" -f $budgetResult.Warning) -ForegroundColor Yellow
        }
    }

    try {
        $entries = Get-ZipEntryNames -ZipPath $item.FullName
        $info = Get-ReleaseApkInspection -Entries $entries -ApkLabel $item.Name
        Assert-ReleaseApkInspection -Inspection $info
        $script:ApkInspections.Add($info)
        if (-not $info.sqlite_bundled) {
            $script:Notes.Add(("APK {0}: no exact bundled sqlite/sqlcipher .so matched; runtime may still use linked sqlite" -f $item.Name))
        }
    } catch {
        $safeMessage = Protect-ReleasePath -Text $_.Exception.Message -RepoRoot $script:RepoRoot
        throw ("APK inspection failed closed for {0}: {1}" -f $item.Name, $safeMessage)
    }

    return $true
}

function Find-Arm64Apks {
    param(
        [string]$RepoRoot,
        [datetime]$NotBeforeUtc
    )

    # Prefer the current-run Gradle outputs; do not harvest arbitrary historical APKs from target/.
    $searchRoots = @(
        (Join-Path $RepoRoot 'crates\tauri-app\gen\android\app\build\outputs\apk')
    )

    $found = @()
    foreach ($root in $searchRoots) {
        if (-not (Test-Path -LiteralPath $root)) { continue }
        $found += @(Get-ChildItem -LiteralPath $root -Recurse -Filter '*.apk' -File -ErrorAction SilentlyContinue |
            Where-Object {
                ($_.Name -match 'arm64|aarch64' -or $_.FullName -match 'arm64|aarch64') -and
                (Test-ReleaseArtifactIsFresh -FileInfo $_ -NotBeforeUtc $NotBeforeUtc)
            })
    }
    return @($found | Sort-Object FullName -Unique)
}

try {
    Write-Host ("StoryForge Android host pipeline root: {0}" -f (Protect-ReleasePath -Text $script:RepoRoot -RepoRoot $script:RepoRoot))
    Write-Host 'Android environment summary:' -ForegroundColor Cyan
    Write-Host ("  ANDROID_HOME: {0}" -f (Format-EnvPathState -Name 'ANDROID_HOME'))
    Write-Host ("  NDK_HOME:      {0}" -f (Format-EnvPathState -Name 'NDK_HOME'))
    Write-Host ("  JAVA_HOME:     {0}" -f (Format-EnvPathState -Name 'JAVA_HOME'))
    Write-Host '  adb/device:    not required / not used'

    if ($DryRun) {
        Write-Host 'Dry run enabled; commands will be printed but not executed.'
        $script:Notes.Add('dry-run mode')
    }

    $identity = Get-ReleaseGitIdentity -RepoRoot $script:RepoRoot
    $toolVersions = Get-ReleaseToolVersions
    $buildStartedUtc = (Get-Date).ToUniversalTime()
    $script:Notes.Add(('build_started_utc={0}' -f $buildStartedUtc.ToString('o')))

    if ([string]::IsNullOrWhiteSpace($OutputDir)) {
        $runDir = New-ReleaseRunDirectory -RepoRoot $script:RepoRoot -Prefix 'android'
    } else {
        $runDir = $OutputDir
        if (-not (Test-Path -LiteralPath $runDir)) {
            New-Item -ItemType Directory -Force -Path $runDir | Out-Null
        }
    }
    $script:Notes.Add(('evidence_dir={0}' -f (Get-RelativeReleasePath -RepoRoot $script:RepoRoot -FullPath $runDir)))

    if (-not $SkipSecretScan) {
        Start-AndroidHostStep -Name 'secret scan'
        if ($DryRun) {
            Write-Host 'DRY RUN: git-tracked secret scan'
        } else {
            Invoke-ReleaseSecretScan -RepoRoot $script:RepoRoot
        }
    } else {
        $script:Notes.Add('secret scan skipped by flag')
    }

    $androidBuildIssues = @()
    if ($BuildApk) {
        $androidBuildIssues = @(Get-ReleaseAndroidBuildPathIssues)
        if (@($androidBuildIssues).Count -gt 0) {
            if ($DryRun) {
                foreach ($issue in $androidBuildIssues) {
                    Write-Host ("DRY RUN NOTE: {0}" -f $issue)
                    $script:Notes.Add($issue)
                }
                $script:Notes.Add('APK build would fail-closed without ANDROID_HOME/NDK_HOME')
            } else {
                # Requested production APK evidence must fail before any host build work.
                Assert-ReleaseAndroidBuildEnvironment
            }
        }
    }

    $frontendRoot = Join-Path $script:RepoRoot 'frontend'
    $tauriAppRoot = Join-Path $script:RepoRoot 'crates\tauri-app'
    $buildStatus = 'ok'

    # Host-side smoke steps (no device).
    # Always recreate dependencies from package-lock for reproducible release input.
    Invoke-AndroidHostCommand -Name 'frontend npm.cmd ci' -WorkingDirectory $frontendRoot -Command @('npm.cmd', 'ci') | Out-Null
    Invoke-AndroidHostCommand -Name 'frontend npm.cmd run build' -WorkingDirectory $frontendRoot -Command @('npm.cmd', 'run', 'build') | Out-Null
    Invoke-AndroidHostCommand -Name 'cargo test -p storyforge --test capabilities' -WorkingDirectory $script:RepoRoot -Command @('cargo', 'test', '-p', 'storyforge', '--test', 'capabilities') | Out-Null
    Invoke-AndroidHostCommand -Name 'cargo check -p storyforge-infra-util --target aarch64-linux-android' -WorkingDirectory $script:RepoRoot -Command @('cargo', 'check', '-p', 'storyforge-infra-util', '--target', 'aarch64-linux-android') | Out-Null

    $apkAttempted = $false
    if ($BuildApk) {
        if (@($androidBuildIssues).Count -eq 0) {
            $apkAttempted = $true
            $debugLog = Join-Path $runDir 'android-debug-build.log'
            $releaseLog = Join-Path $runDir 'android-release-build.log'

            # APK builds are requested evidence: fail closed, do not degrade to partial exit 0.
            $debugResult = Invoke-AndroidHostCommand `
                -Name 'cargo tauri android build debug arm64 APK' `
                -WorkingDirectory $tauriAppRoot `
                -Command @('cargo', 'tauri', 'android', 'build', '--debug', '--target', 'aarch64', '--ci', '--split-per-abi', '--apk') `
                -LogPath $debugLog

            $releaseResult = Invoke-AndroidHostCommand `
                -Name 'cargo tauri android build unsigned release arm64 APK' `
                -WorkingDirectory $tauriAppRoot `
                -Command @('cargo', 'tauri', 'android', 'build', '--target', 'aarch64', '--ci', '--split-per-abi', '--apk') `
                -LogPath $releaseLog

            $allLines = @($debugResult.Lines) + @($releaseResult.Lines)
            $report = ConvertTo-ReleaseWarningReport -Lines $allLines -Source 'android-build' -RepoRoot $script:RepoRoot
            $script:WarningReports.Add($report)
            foreach ($w in $report.warnings) {
                $script:Warnings.Add(("{0}: {1}" -f $w.category, $w.message))
            }

            Start-AndroidHostStep -Name 'inspect APK artifacts'
            $apks = Find-Arm64Apks -RepoRoot $script:RepoRoot -NotBeforeUtc $buildStartedUtc
            if (@($apks).Count -eq 0) {
                $script:Warnings.Add('no fresh arm64 APK artifacts found after build attempt')
                $buildStatus = 'failed'
            } else {
                $freshCount = 0
                foreach ($apk in $apks) {
                    $kind = if ($apk.Name -match 'release') { 'android-release-apk' } else { 'android-debug-apk' }
                    if (Add-ApkArtifactAndInspect -ApkPath $apk.FullName -Kind $kind -NotBeforeUtc $buildStartedUtc -RequireFresh) {
                        $freshCount += 1
                    }
                }
                if ($freshCount -eq 0) {
                    $buildStatus = 'failed'
                }
                if ($buildStatus -ne 'failed') {
                    Assert-ReleaseRequiredApkKinds -Artifacts ([object[]]$script:Artifacts.ToArray())
                }
            }
        }
    } else {
        $script:Notes.Add('APK build not requested; host smoke only')
    }

    if ($DryRun) {
        $buildStatus = 'dry-run'
        $script:Artifacts.Add((New-ReleaseArtifactRecord -RelativePath 'crates/tauri-app/gen/android/.../app-arm64-debug.apk' -SizeBytes 0 -Sha256 $null -Kind 'android-debug-apk' -Status 'skipped'))
        $script:Artifacts.Add((New-ReleaseArtifactRecord -RelativePath 'crates/tauri-app/gen/android/.../app-arm64-release-unsigned.apk' -SizeBytes 0 -Sha256 $null -Kind 'android-release-apk' -Status 'skipped'))
    }

    $script:Notes.Add('host-only; android device acceptance not claimed')
    $script:Notes.Add('GUI acceptance not claimed')
    if (-not $apkAttempted -and $BuildApk) {
        $script:Notes.Add('APK build skipped due to missing prerequisites')
    }

    Start-AndroidHostStep -Name 'write evidence'
    $apkInspectionArr = if ($script:ApkInspections.Count -gt 0) { [object[]]$script:ApkInspections.ToArray() } else { @() }
    $warningReportArr = if ($script:WarningReports.Count -gt 0) { [object[]]$script:WarningReports.ToArray() } else { @() }
    $noteArrForEvidence = [string[]]@($script:Notes | ForEach-Object { [string]$_ })
    $evidence = Protect-ReleaseObject -Value ([pscustomobject]@{
        schema_version = 1
        commit = $identity.commit
        branch = $identity.branch
        tool_versions = $toolVersions
        apk_inspections = $apkInspectionArr
        warning_reports = $warningReportArr
        notes = $noteArrForEvidence
    }) -RepoRoot $script:RepoRoot
    $inspectionPath = Join-Path $runDir 'apk-inspection.json'
    Write-ReleaseJson -Object $evidence -Path $inspectionPath

    Start-AndroidHostStep -Name 'dependency / SBOM-style inventory'
    $inventory = New-ReleaseDependencyInventory `
        -CargoTomlPath (Join-Path $script:RepoRoot 'Cargo.toml') `
        -PackageLockPath (Join-Path $script:RepoRoot 'frontend\package-lock.json') `
        -PreferCargoTree:$false
    $inventoryPath = Join-Path $runDir 'dependency-inventory.json'
    Write-ReleaseJson -Object $inventory -Path $inventoryPath
    Write-Host ("Wrote inventory: {0}" -f (Get-RelativeReleasePath -RepoRoot $script:RepoRoot -FullPath $inventoryPath))
    $script:Notes.Add(('dependency_inventory_generator={0}' -f $inventory.generator))
    $inventoryEvidence = [pscustomobject]@{
        relative_path = Get-RelativeReleasePath -RepoRoot $runDir -FullPath $inventoryPath
        sha256 = Get-ReleaseFileSha256 -Path $inventoryPath
        component_count = @($inventory.components).Count
        generator = $inventory.generator
    }

    $warningArr = [string[]]@($script:Warnings | ForEach-Object { [string]$_ })
    $noteArr = [string[]]@($script:Notes | ForEach-Object { [string]$_ })
    if ($script:Artifacts.Count -gt 0) {
        $artifactArr = [object[]]$script:Artifacts.ToArray()
    } else {
        $artifactArr = [object[]]@()
    }
    $manifest = New-ReleaseBuildManifest `
        -Commit $identity.commit `
        -Branch $identity.branch `
        -Target 'aarch64-linux-android' `
        -ToolVersions $toolVersions `
        -Artifacts $artifactArr `
        -DependencyInventory $inventoryEvidence `
        -BuildStatus $buildStatus `
        -Warnings $warningArr `
        -Notes $noteArr `
        -RepoRoot $script:RepoRoot
    $manifestPath = Join-Path $runDir 'manifest.json'
    Write-ReleaseJson -Object $manifest -Path $manifestPath

    Start-AndroidHostStep -Name 'manifest schema validation'
    Assert-ReleaseManifestSchema -Manifest $manifest
    Write-Host 'Manifest schema validation passed.'

    Start-AndroidHostStep -Name 'artifact hash sidecars'
    if ($DryRun) {
        Write-Host 'DRY RUN: would write <artifact>.sha256 sidecar files for present artifacts'
    } else {
        $presentArtifacts = @($script:Artifacts | Where-Object { $_.status -eq 'present' })
        foreach ($art in $presentArtifacts) {
            $fullPath = Join-Path $script:RepoRoot ($art.relative_path -replace '/', '\')
            if (Test-Path -LiteralPath $fullPath -PathType Leaf) {
                $hashFile = Write-ReleaseHashFile -ArtifactPath $fullPath
                Write-Host ("Wrote hash sidecar: {0}" -f (Get-RelativeReleasePath -RepoRoot $script:RepoRoot -FullPath $hashFile))
                # Verify archive integrity for APK artifacts.
                if ($art.kind -match 'apk') {
                    Test-ReleaseArchiveIntegrity -Path $fullPath -ExpectedKind $art.kind
                    Write-Host ("Verified archive integrity: {0}" -f $art.relative_path)
                }
            }
        }
    }

    Start-AndroidHostStep -Name 'provenance attestation'
    $provArtifacts = if ($DryRun) { [object[]]@() } else { [object[]]@($script:Artifacts | Where-Object { $_.status -eq 'present' }) }
    if ($null -eq $provArtifacts) { $provArtifacts = [object[]]@() }
    $provenance = New-ReleaseProvenance `
        -Commit $identity.commit `
        -Branch $identity.branch `
        -Target 'aarch64-linux-android' `
        -Artifacts $provArtifacts `
        -RepoRoot $script:RepoRoot
    $provenancePath = Join-Path $runDir 'provenance.json'
    Write-ReleaseJson -Object $provenance -Path $provenancePath

    $summaryPath = Join-Path $runDir 'SUMMARY.txt'
    $summary = New-Object System.Collections.Generic.List[string]
    foreach ($line in @(
        'StoryForge Android host pipeline summary'
        ("commit={0}" -f $identity.commit)
        ("branch={0}" -f $identity.branch)
        ("build_status={0}" -f $buildStatus)
        ("apk_build_requested={0}" -f [bool]$BuildApk)
        ("artifacts={0}" -f $script:Artifacts.Count)
        ("warnings={0}" -f $script:Warnings.Count)
        'acceptance.gui=not_claimed'
        'acceptance.android_device=not_claimed'
        '--- warnings ---'
    )) { $summary.Add([string](Protect-ReleasePath -Text $line -RepoRoot $script:RepoRoot)) }
    foreach ($w in $script:Warnings) { $summary.Add([string](Protect-ReleasePath -Text $w -RepoRoot $script:RepoRoot)) }
    $summary.Add('--- notes ---')
    foreach ($n in $script:Notes) { $summary.Add([string](Protect-ReleasePath -Text $n -RepoRoot $script:RepoRoot)) }
    Set-Content -LiteralPath $summaryPath -Value $summary.ToArray() -Encoding utf8

    Start-AndroidHostStep -Name 'retention cleanup'
    $artifactRoot = Join-Path $script:RepoRoot 'artifacts\release-build'
    $targets = Get-ReleaseRetentionCleanupTargets `
        -Root $artifactRoot `
        -Keep $KeepRuns `
        -NamePrefixes @('windows-', 'android-') `
        -ProtectFullNames @($runDir)
    if ($DryRun) {
        Write-Host ("DRY RUN: would remove {0} old run dir(s), keep {1}" -f @($targets).Count, $KeepRuns)
    } else {
        foreach ($dir in $targets) {
            Write-Host ("Removing old run dir: {0}" -f (Get-RelativeReleasePath -RepoRoot $script:RepoRoot -FullPath $dir.FullName))
        }
        if ($null -ne $targets -and @($targets).Count -gt 0) {
            Remove-ReleaseRetentionTargets -Root $artifactRoot -Targets $targets
        }
    }

    Write-Host ''
    $exitCode = Get-ReleaseProcessExitCode -BuildStatus $buildStatus
    if ($exitCode -ne 0) {
        Write-Host ("Android host pipeline FAILED (fail-closed) status={0}." -f $buildStatus) -ForegroundColor Red
        exit $exitCode
    }

    Write-Host ("Android host pipeline finished with status={0}." -f $buildStatus) -ForegroundColor Green
    Write-Host 'Reminder: host smoke/APK build is not device PASS.' -ForegroundColor Yellow
    exit 0
} catch {
    $safeError = Get-ReleaseSafeErrorDetails -ErrorRecord $_ -RepoRoot $script:RepoRoot
    [Console]::Error.WriteLine(("ERROR: {0}" -f $safeError.message))
    if ($safeError.stack) {
        [Console]::Error.WriteLine(("STACK: {0}" -f $safeError.stack))
    }
    if ($safeError.position) {
        [Console]::Error.WriteLine(("POSITION: {0}" -f $safeError.position))
    }
    exit 1
}
