<#
.SYNOPSIS
Runs a fail-closed Windows host release build and writes evidence artifacts.

.DESCRIPTION
Checks required tools, builds the frontend, builds/bundles the Rust/Tauri release
host target when available, writes a redacted artifact manifest, dependency
inventory, size-budget warnings, and optional retention cleanup.

This is host-only evidence. It does not claim desktop GUI acceptance, does not
sign/publish, does not call paid LLMs, and does not start the desktop GUI.

.PARAMETER DryRun
Print planned steps and write a dry-run manifest without executing builds.

.PARAMETER SkipFrontend
Skip frontend npm build (still records skip in the manifest).

.PARAMETER SkipBundle
Skip `cargo tauri build` and only run `cargo build --release -p storyforge`
when possible.

.PARAMETER SkipSecretScan
Skip the fail-closed Git-tracked secret scan. Prefer leaving this off for release evidence.

.PARAMETER KeepRuns
Number of previous artifacts/release-build runs to retain (default 5).

.PARAMETER OutputDir
Optional explicit output directory under which manifest files are written.
Defaults to artifacts/release-build/run-<timestamp>.

.EXAMPLE
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-release-build.ps1 -DryRun

.EXAMPLE
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-release-build.ps1
#>
[CmdletBinding()]
param(
    [switch]$DryRun,
    [switch]$SkipFrontend,
    [switch]$SkipBundle,
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

function Start-ReleaseBuildStep {
    param([Parameter(Mandatory = $true)][string]$Name)
    $script:StepNumber += 1
    Write-Host ''
    Write-Host ("[{0}] {1}" -f $script:StepNumber, $Name) -ForegroundColor Cyan
}

function Invoke-ReleaseBuildCommand {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [Parameter(Mandatory = $true)][string[]]$Command,
        [switch]$AllowFail
    )

    Start-ReleaseBuildStep -Name $Name
    $formatted = Format-ReleaseCommand -Command $Command
    $safeFormatted = Protect-ReleasePath -Text $formatted -RepoRoot $script:RepoRoot
    if ($DryRun) {
        Write-Host ("DRY RUN: cd {0}" -f (Protect-ReleasePath -Text $WorkingDirectory -RepoRoot $script:RepoRoot))
        Write-Host ("DRY RUN: {0}" -f $safeFormatted)
        return 0
    }

    Push-Location -LiteralPath $WorkingDirectory
    try {
        Write-Host ("RUN: {0}" -f $safeFormatted)
        # Capture native stdout/stderr and re-emit only after path/secret redaction.
        $prevEap = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try {
            $output = & $Command[0] @($Command[1..($Command.Count - 1)]) 2>&1
            $code = $LASTEXITCODE
        } finally {
            $ErrorActionPreference = $prevEap
        }
        foreach ($line in @($output)) {
            Write-Host (Protect-ReleasePath -Text "$line" -RepoRoot $script:RepoRoot)
        }
        if ($null -eq $code) { $code = 0 }
        if ($code -ne 0) {
            if ($AllowFail) {
                $script:Warnings.Add("step '$Name' exited $code (allowed non-fatal)")
                return $code
            }
            Assert-ReleaseExitCode -ExitCode $code -StepName $Name
        }
        return $code
    } finally {
        Pop-Location
    }
}

function Add-PresentArtifact {
    param(
        [Parameter(Mandatory = $true)][string]$FullPath,
        [Parameter(Mandatory = $true)][string]$Kind,
        [datetime]$NotBeforeUtc,
        [switch]$RequireFresh
    )

    if (-not (Test-Path -LiteralPath $FullPath -PathType Leaf)) {
        $relMissing = Get-RelativeReleasePath -RepoRoot $script:RepoRoot -FullPath $FullPath
        $script:Artifacts.Add((New-ReleaseArtifactRecord -RelativePath $relMissing -SizeBytes 0 -Sha256 $null -Kind $Kind -Status 'missing'))
        return $false
    }

    $item = Get-Item -LiteralPath $FullPath
    if ($RequireFresh -and -not (Test-ReleaseArtifactIsFresh -FileInfo $item -NotBeforeUtc $NotBeforeUtc)) {
        $relStale = Get-RelativeReleasePath -RepoRoot $script:RepoRoot -FullPath $item.FullName
        $script:Warnings.Add(("stale artifact ignored for current SHA evidence: {0}" -f $relStale))
        Write-Host ("WARNING: ignoring stale artifact (older than build start): {0}" -f $relStale) -ForegroundColor Yellow
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

    return $true
}

function Test-WorkingTreeCleanForRelease {
    param([string]$RepoRoot)

    Start-ReleaseBuildStep -Name 'input cleanliness check'
    if ($DryRun) {
        Write-Host 'DRY RUN: git status --porcelain'
        return
    }

    $status = & git -C $RepoRoot status --porcelain
    if ($LASTEXITCODE -ne 0) {
        throw 'git status failed during cleanliness check.'
    }

    if ($status) {
        $script:Notes.Add('working tree is dirty; release evidence still collected but not a clean-input claim')
        Write-Host 'NOTE: working tree is dirty; continuing with explicit note (not treated as clean input).' -ForegroundColor Yellow
        foreach ($line in $status) {
            Write-Host ("  {0}" -f (Protect-ReleasePath -Text $line -RepoRoot $RepoRoot))
        }
    } else {
        $script:Notes.Add('working tree clean at start of release build')
        Write-Host 'OK: working tree clean.'
    }
}

function Resolve-ExpectedWindowsArtifacts {
    param([string]$RepoRoot)

    $candidates = @(
        @{ Path = (Join-Path $RepoRoot 'target\release\storyforge.exe'); Kind = 'windows-exe' },
        @{ Path = (Join-Path $RepoRoot 'target\release\storyforge_lib.dll'); Kind = 'windows-dll' }
    )

    $bundleRoots = @(
        (Join-Path $RepoRoot 'target\release\bundle'),
        (Join-Path $RepoRoot 'crates\tauri-app\target\release\bundle')
    )

    foreach ($root in $bundleRoots) {
        if (-not (Test-Path -LiteralPath $root)) { continue }
        Get-ChildItem -LiteralPath $root -Recurse -File -ErrorAction SilentlyContinue |
            Where-Object { $_.Extension -in '.msi', '.exe' } |
            ForEach-Object {
                $kind = if ($_.Extension -eq '.msi') { 'windows-msi' } else { 'windows-nsis' }
                $candidates += @{ Path = $_.FullName; Kind = $kind }
            }
    }

    return $candidates
}

try {
    Write-Host ("StoryForge Windows release build root: {0}" -f (Protect-ReleasePath -Text $script:RepoRoot -RepoRoot $script:RepoRoot))
    if ($DryRun) {
        Write-Host 'Dry run enabled; heavy builds will not execute.'
        $script:Notes.Add('dry-run mode; no heavy builds executed')
    }

    $identity = Get-ReleaseGitIdentity -RepoRoot $script:RepoRoot
    $toolVersions = Get-ReleaseToolVersions
    $buildStartedUtc = (Get-Date).ToUniversalTime()
    $script:Notes.Add(('build_started_utc={0}' -f $buildStartedUtc.ToString('o')))

    if ([string]::IsNullOrWhiteSpace($OutputDir)) {
        $runDir = New-ReleaseRunDirectory -RepoRoot $script:RepoRoot -Prefix 'windows'
    } else {
        $runDir = $OutputDir
        if (-not (Test-Path -LiteralPath $runDir)) {
            New-Item -ItemType Directory -Force -Path $runDir | Out-Null
        }
    }
    $script:Notes.Add(('evidence_dir={0}' -f (Get-RelativeReleasePath -RepoRoot $script:RepoRoot -FullPath $runDir)))

    if (-not $SkipSecretScan) {
        Start-ReleaseBuildStep -Name 'secret scan'
        if ($DryRun) {
            Write-Host 'DRY RUN: git-tracked secret scan'
        } else {
            Invoke-ReleaseSecretScan -RepoRoot $script:RepoRoot
        }
    } else {
        $script:Notes.Add('secret scan skipped by flag')
    }

    Test-WorkingTreeCleanForRelease -RepoRoot $script:RepoRoot

    # Required tools (fail closed unless dry-run for optional tools).
    Start-ReleaseBuildStep -Name 'tool availability'
    $cargoCmd = Get-Command cargo -ErrorAction SilentlyContinue
    if (-not $cargoCmd) {
        throw "Required tool 'cargo' is missing."
    }
    Write-Host ("cargo: {0}" -f $toolVersions['cargo'])

    if (-not $SkipFrontend) {
        $npmCmd = Get-Command npm.cmd -ErrorAction SilentlyContinue
        if (-not $npmCmd -and -not $DryRun) {
            throw "Required tool 'npm.cmd' is missing for frontend build."
        }
        if ($npmCmd) {
            Write-Host ("npm: {0}" -f $toolVersions['npm'])
        } else {
            Write-Host 'DRY RUN: npm.cmd presence not enforced'
        }
    } else {
        $script:Notes.Add('frontend build skipped by flag')
    }

    $frontendRoot = Join-Path $script:RepoRoot 'frontend'
    $tauriAppRoot = Join-Path $script:RepoRoot 'crates\tauri-app'

    $buildStatus = 'ok'
    $bundleRequested = -not $SkipBundle

    if (-not $SkipFrontend) {
        # Always recreate dependencies from package-lock for reproducible release input.
        $null = Invoke-ReleaseBuildCommand -Name 'frontend npm.cmd ci' -WorkingDirectory $frontendRoot -Command @('npm.cmd', 'ci')
        $null = Invoke-ReleaseBuildCommand -Name 'frontend npm.cmd run build' -WorkingDirectory $frontendRoot -Command @('npm.cmd', 'run', 'build')
    } else {
        # Tauri generate_context! requires frontendDist to exist. Mirror bronze smoke:
        # create a local gitignored placeholder so host compile can proceed without GUI claim.
        $distDir = Join-Path $frontendRoot 'dist'
        $indexPath = Join-Path $distDir 'index.html'
        if (-not (Test-Path -LiteralPath $indexPath) -and -not $DryRun) {
            New-Item -ItemType Directory -Force -Path $distDir | Out-Null
            @(
                '<!doctype html>'
                '<html><head><meta charset="utf-8"><title>StoryForge release placeholder</title></head>'
                '<body><div id="app">StoryForge release placeholder</div></body></html>'
            ) | Set-Content -LiteralPath $indexPath -Encoding utf8
            $script:Notes.Add('created local frontend/dist placeholder because -SkipFrontend and dist was missing')
            Write-Host 'Created local frontend/dist placeholder for Tauri release compile context.' -ForegroundColor Yellow
        }
    }

    # Rust release binary.
    $null = Invoke-ReleaseBuildCommand -Name 'cargo build --release -p storyforge' -WorkingDirectory $script:RepoRoot -Command @('cargo', 'build', '--release', '-p', 'storyforge')

    if ($bundleRequested) {
        $prevEap = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try {
            $null = & cargo tauri --version 2>$null
            $tauriCode = $LASTEXITCODE
        } finally {
            $ErrorActionPreference = $prevEap
        }

        if ($tauriCode -eq 0) {
            # Bundle is part of the requested release path: fail closed on non-zero.
            $null = Invoke-ReleaseBuildCommand -Name 'cargo tauri build (Windows host bundle)' -WorkingDirectory $tauriAppRoot -Command @('cargo', 'tauri', 'build', '--ci')
        } else {
            throw 'cargo tauri CLI is required for Windows bundle evidence but is unavailable. Use -SkipBundle for host-binary-only evidence.'
        }
    } else {
        $script:Notes.Add('tauri bundle skipped by flag')
    }

    Start-ReleaseBuildStep -Name 'collect Windows artifacts'
    $expected = Resolve-ExpectedWindowsArtifacts -RepoRoot $script:RepoRoot
    $anyPresent = $false
    $missingRequired = $false

    if ($DryRun) {
        foreach ($item in $expected | Select-Object -First 2) {
            $rel = Get-RelativeReleasePath -RepoRoot $script:RepoRoot -FullPath $item.Path
            $script:Artifacts.Add((New-ReleaseArtifactRecord -RelativePath $rel -SizeBytes 0 -Sha256 $null -Kind $item.Kind -Status 'skipped'))
        }
        $script:Notes.Add('artifact hashing skipped in dry-run')
        $buildStatus = 'dry-run'
    } else {
        # Always require a fresh main release binary for the current run SHA.
        $mainExe = Join-Path $script:RepoRoot 'target\release\storyforge.exe'
        if (Add-PresentArtifact -FullPath $mainExe -Kind 'windows-exe' -NotBeforeUtc $buildStartedUtc -RequireFresh) {
            $anyPresent = $true
        } else {
            $missingRequired = $true
            $alreadyRecorded = @($script:Artifacts | Where-Object {
                $_.kind -eq 'windows-exe' -and $_.relative_path -eq (Get-RelativeReleasePath -RepoRoot $script:RepoRoot -FullPath $mainExe)
            }).Count -gt 0
            if (-not $alreadyRecorded) {
                $script:Artifacts.Add((New-ReleaseArtifactRecord -RelativePath (Get-RelativeReleasePath -RepoRoot $script:RepoRoot -FullPath $mainExe) -SizeBytes 0 -Sha256 $null -Kind 'windows-exe' -Status 'missing'))
            }
        }

        foreach ($item in $expected) {
            if ($item.Path -eq $mainExe) { continue }
            if (Test-Path -LiteralPath $item.Path -PathType Leaf) {
                # Optional bundle/installer artifacts only count when fresh for this run.
                if (Add-PresentArtifact -FullPath $item.Path -Kind $item.Kind -NotBeforeUtc $buildStartedUtc -RequireFresh) {
                    $anyPresent = $true
                }
            }
        }

        if ($bundleRequested) {
            $freshBundle = @($script:Artifacts | Where-Object { $_.status -eq 'present' -and $_.kind -in @('windows-msi', 'windows-nsis') })
            if ($freshBundle.Count -eq 0) {
                $missingRequired = $true
                $script:Warnings.Add('requested Windows bundle produced no fresh installer artifacts for this run')
            }
        }

        if ($missingRequired) {
            $buildStatus = 'failed'
        } elseif (-not $anyPresent) {
            $buildStatus = 'failed'
        }
    }

    Start-ReleaseBuildStep -Name 'dependency / SBOM-style inventory'
    $inventory = New-ReleaseDependencyInventory `
        -CargoTomlPath (Join-Path $script:RepoRoot 'Cargo.toml') `
        -PackageLockPath (Join-Path $script:RepoRoot 'frontend\package-lock.json') `
        -PreferCargoTree:$false
    $inventoryPath = Join-Path $runDir 'dependency-inventory.json'
    # Inventory is cheap local metadata and is always written (even in dry-run).
    Write-ReleaseJson -Object $inventory -Path $inventoryPath
    Write-Host ("Wrote inventory: {0}" -f (Get-RelativeReleasePath -RepoRoot $script:RepoRoot -FullPath $inventoryPath))
    $script:Notes.Add(('dependency_inventory_generator={0}' -f $inventory.generator))
    $inventoryEvidence = [pscustomobject]@{
        relative_path = Get-RelativeReleasePath -RepoRoot $runDir -FullPath $inventoryPath
        sha256 = Get-ReleaseFileSha256 -Path $inventoryPath
        component_count = @($inventory.components).Count
        generator = $inventory.generator
    }

    $script:Notes.Add('host-only; GUI acceptance not claimed')
    $script:Notes.Add('android device acceptance not claimed')

    $warningArr = [string[]]@($script:Warnings | ForEach-Object { [string]$_ })
    $noteArr = [string[]]@($script:Notes | ForEach-Object { [string]$_ })
    if ($script:Artifacts.Count -gt 0) {
        $artifactArr = [object[]]$script:Artifacts.ToArray()
    } else {
        $artifactArr = [object[]]@()
    }

    Start-ReleaseBuildStep -Name 'stage evidence subjects and hash sidecars'
    $stagedSubjects = [object[]]@()
    if ($DryRun) {
        Write-Host 'DRY RUN: would stage subjects/ and <artifact>.sha256 sidecars into the evidence directory'
    } else {
        $presentArtifacts = @($script:Artifacts | Where-Object { $_.status -eq 'present' })
        $stagedSubjects = @(Copy-ReleaseEvidenceSubjects -Artifacts $presentArtifacts -EvidenceDir $runDir -RepoRoot $script:RepoRoot)
        foreach ($s in $stagedSubjects) {
            Write-Host ("Staged subject: {0} (source={1}) sha256={2}" -f $s.relative_path, $s.source_relative_path, $s.sha256)
        }
    }
    if ($null -eq $stagedSubjects) { $stagedSubjects = [object[]]@() }

    # Manifest keeps source-tree artifact paths and a separate staged_subjects
    # list for offline verification under subjects/....
    $manifest = New-ReleaseBuildManifest `
        -Commit $identity.commit `
        -Branch $identity.branch `
        -Target 'x86_64-pc-windows-msvc' `
        -ToolVersions $toolVersions `
        -Artifacts $artifactArr `
        -StagedSubjects $stagedSubjects `
        -DependencyInventory $inventoryEvidence `
        -BuildStatus $buildStatus `
        -Warnings $warningArr `
        -Notes $noteArr `
        -RepoRoot $script:RepoRoot

    $manifestPath = Join-Path $runDir 'manifest.json'
    Write-ReleaseJson -Object $manifest -Path $manifestPath
    Write-Host ("Wrote manifest: {0}" -f (Get-RelativeReleasePath -RepoRoot $script:RepoRoot -FullPath $manifestPath))

    Start-ReleaseBuildStep -Name 'manifest schema validation'
    Assert-ReleaseManifestSchema -Manifest $manifest
    Write-Host 'Manifest schema validation passed.'

    Start-ReleaseBuildStep -Name 'provenance attestation'
    # Provenance subjects reference staged evidence-relative paths so the
    # uploaded package can be verified offline without the original build tree.
    $provArtifacts = if ($DryRun) {
        [object[]]@()
    } else {
        [object[]]@($stagedSubjects | ForEach-Object {
            [pscustomobject]@{
                relative_path = $_.relative_path
                sha256        = $_.sha256
                kind          = $_.kind
                size_bytes    = $_.size_bytes
                status        = $_.status
            }
        })
    }
    if ($null -eq $provArtifacts) { $provArtifacts = [object[]]@() }
    $provenance = New-ReleaseProvenance `
        -Commit $identity.commit `
        -Branch $identity.branch `
        -Target 'x86_64-pc-windows-msvc' `
        -Artifacts $provArtifacts `
        -RepoRoot $script:RepoRoot
    $provenancePath = Join-Path $runDir 'provenance.json'
    Write-ReleaseJson -Object $provenance -Path $provenancePath
    Write-Host ("Wrote provenance: {0}" -f (Get-RelativeReleasePath -RepoRoot $script:RepoRoot -FullPath $provenancePath))

    $summaryPath = Join-Path $runDir 'SUMMARY.txt'
    $summaryLines = New-Object System.Collections.Generic.List[string]
    foreach ($line in @(
        'StoryForge Windows release build summary'
        ("commit={0}" -f $identity.commit)
        ("branch={0}" -f $identity.branch)
        ("build_status={0}" -f $buildStatus)
        ("artifacts={0}" -f $script:Artifacts.Count)
        ("warnings={0}" -f $script:Warnings.Count)
        'acceptance.gui=not_claimed'
        'acceptance.android_device=not_claimed'
        '--- warnings ---'
    )) { $summaryLines.Add([string](Protect-ReleasePath -Text $line -RepoRoot $script:RepoRoot)) }
    foreach ($w in $script:Warnings) { $summaryLines.Add([string](Protect-ReleasePath -Text $w -RepoRoot $script:RepoRoot)) }
    $summaryLines.Add('--- notes ---')
    foreach ($n in $script:Notes) { $summaryLines.Add([string](Protect-ReleasePath -Text $n -RepoRoot $script:RepoRoot)) }
    Set-Content -LiteralPath $summaryPath -Value $summaryLines.ToArray() -Encoding utf8

    Start-ReleaseBuildStep -Name 'retention cleanup'
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
        Write-Host ("Retention complete; keep={0}" -f $KeepRuns)
    }

    Write-Host ''
    $exitCode = Get-ReleaseProcessExitCode -BuildStatus $buildStatus
    if ($exitCode -ne 0) {
        Write-Host ("Windows release build FAILED (fail-closed) status={0}." -f $buildStatus) -ForegroundColor Red
        exit $exitCode
    }

    Write-Host ("Windows release build finished with status={0}." -f $buildStatus) -ForegroundColor Green
    Write-Host 'Reminder: host build success is not GUI/device PASS.' -ForegroundColor Yellow
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
