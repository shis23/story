# Release runner readiness: local preflight, offline evidence verifier,
# workflow static governance, and host-only vs explicit bundle/APK intent.
# Run via scripts/tests/run-release-build-tests.ps1 (includes this file).

$ErrorActionPreference = 'Stop'

$RepoRoot = (& git rev-parse --show-toplevel 2>$null)
if (-not $RepoRoot) {
    throw 'Unable to locate repository root for runner readiness tests.'
}
$RepoRoot = (Resolve-Path -LiteralPath $RepoRoot).ProviderPath
$CommonPath = Join-Path $RepoRoot 'scripts\release-build\ReleaseBuild.Common.ps1'

if (-not (Test-Path -LiteralPath $CommonPath)) {
    throw "Missing ReleaseBuild.Common.ps1 at $CommonPath"
}

. $CommonPath

function New-SyntheticEvidencePackage {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [switch]$OmitSubject,
        [switch]$OmitSidecar,
        [switch]$TamperHash,
        [switch]$BomSidecar,
        [switch]$PathEscape,
        [switch]$UnknownSchema,
        [switch]$SensitiveNote,
        [switch]$MissingInventory,
        [switch]$MissingProvenance,
        [ValidateSet('ok', 'failed', 'partial', 'dry-run')]
        [string]$BuildStatus = 'ok',
        [string]$RemoteCi = 'not_claimed',
        [string]$ProvenanceCommit,
        [string]$ProvenanceBranch,
        [string]$ProvenanceTarget,
        [string[]]$ProvenanceNotes,
        [string[]]$ManifestNotes,
        [switch]$SubjectAsSymlink,
        [switch]$SidecarAsSymlink,
        [switch]$InventoryAsSymlink,
        [switch]$SubjectsDirAsJunction,
        # Real runner topology: artifacts keep source tree paths; staged subjects live under subjects/.
        [switch]$RunnerTopology,
        [string]$SourceRelativePath = 'target/release/storyforge.exe',
        [Nullable[long]]$ForcedSizeBytes = $null,
        [switch]$OmitStagedSubjects,
        [switch]$OmitSizeBytes
    )

    New-Item -ItemType Directory -Force -Path $Root | Out-Null
    $subjectsDir = Join-Path $Root 'subjects\windows-exe'
    if (-not $SubjectsDirAsJunction) {
        New-Item -ItemType Directory -Force -Path $subjectsDir | Out-Null
    }

    # Staged path is always under subjects/ unless PathEscape injects a hostile relative path
    # into the records (file itself is never written at the escape location).
    $stagedRel = if ($PathEscape) { '../outside/storyforge.exe' } else { 'subjects/windows-exe/storyforge.exe' }
    $subjectPath = Join-Path $Root ('subjects\windows-exe\storyforge.exe')
    $outsideDir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-outside-{0}" -f [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force -Path $outsideDir | Out-Null
    $outsideFile = Join-Path $outsideDir 'storyforge.exe'
    [System.IO.File]::WriteAllBytes($outsideFile, [byte[]](9, 9, 9, 9, 9, 9))

    if ($SubjectsDirAsJunction) {
        $subjectsParent = Join-Path $Root 'subjects'
        New-Item -ItemType Directory -Force -Path $subjectsParent | Out-Null
        $null = cmd /c mklink /J "$subjectsDir" "$outsideDir"
        if ($LASTEXITCODE -ne 0) {
            throw "Unable to create subjects directory junction for adversarial test."
        }
    }

    if (-not $OmitSubject -and -not $PathEscape) {
        if ($SubjectAsSymlink) {
            $null = cmd /c mklink "$subjectPath" "$outsideFile"
            if ($LASTEXITCODE -ne 0) {
                throw "Unable to create subject symlink for adversarial test."
            }
        } else {
            [System.IO.File]::WriteAllBytes($subjectPath, [byte[]](1, 2, 3, 4, 5, 6))
        }
    }

    $sha = if ((-not $OmitSubject) -and (Test-Path -LiteralPath $subjectPath)) {
        # Hash the real bytes when possible; symlink targets still have content.
        try { Get-ReleaseFileSha256 -Path $subjectPath } catch { 'a' * 64 }
    } else {
        'a' * 64
    }
    $reportedSha = if ($TamperHash) { 'b' * 64 } else { $sha }
    $sizeBytes = if ($null -ne $ForcedSizeBytes) { [long]$ForcedSizeBytes } else { 6 }

    if (-not $OmitSidecar -and -not $OmitSubject) {
        $sidecarPath = $subjectPath + '.sha256'
        $content = "{0} *storyforge.exe" -f $reportedSha
        if ($SidecarAsSymlink) {
            $outsideSidecar = Join-Path $outsideDir 'storyforge.exe.sha256'
            $utf8NoBom = New-Object System.Text.UTF8Encoding $false
            [System.IO.File]::WriteAllText($outsideSidecar, $content, $utf8NoBom)
            $null = cmd /c mklink "$sidecarPath" "$outsideSidecar"
            if ($LASTEXITCODE -ne 0) {
                throw "Unable to create sidecar symlink for adversarial test."
            }
        } elseif ($BomSidecar) {
            $utf8Bom = New-Object System.Text.UTF8Encoding $true
            [System.IO.File]::WriteAllText($sidecarPath, $content, $utf8Bom)
        } else {
            $utf8NoBom = New-Object System.Text.UTF8Encoding $false
            [System.IO.File]::WriteAllText($sidecarPath, $content, $utf8NoBom)
        }
    }

    # Source-domain artifact path (real runner topology) vs staged subject path.
    $artifactSourcePath = if ($RunnerTopology) {
        $SourceRelativePath
    } elseif ($PathEscape) {
        $stagedRel
    } else {
        $stagedRel
    }
    $artifact = New-ReleaseArtifactRecord `
        -RelativePath $artifactSourcePath `
        -SizeBytes $sizeBytes `
        -Sha256 $reportedSha `
        -Kind 'windows-exe' `
        -Status 'present'

    $schemaVersion = if ($UnknownSchema) { 99 } else { 1 }
    $notes = @('host-only; GUI acceptance not claimed')
    if ($SensitiveNote) {
        $notes += ('token=sk-' + ('z' * 24))
    }
    if ($null -ne $ManifestNotes) {
        $notes = @($ManifestNotes)
    }

    $defaultCommit = 'abcdef0123456789abcdef0123456789abcdef01'
    $stagedSubject = $null
    if (-not $OmitStagedSubjects) {
        $stagedSubject = New-ReleaseStagedSubjectRecord `
            -StagedRelativePath $stagedRel `
            -SourceRelativePath $artifactSourcePath `
            -SizeBytes $sizeBytes `
            -Sha256 $reportedSha `
            -Kind 'windows-exe' `
            -Status 'present' `
            -HashSidecar ($stagedRel + '.sha256')
        if ($OmitSizeBytes -and $null -ne $stagedSubject) {
            $stagedSubject = [pscustomobject]@{
                relative_path = $stagedSubject.relative_path
                source_relative_path = $stagedSubject.source_relative_path
                sha256 = $stagedSubject.sha256
                kind = $stagedSubject.kind
                status = $stagedSubject.status
                hash_sidecar = $stagedSubject.hash_sidecar
            }
        }
    }

    $manifest = [pscustomobject]@{
        schema_version = $schemaVersion
        generated_at_utc = '2026-07-15T00:00:00Z'
        commit = $defaultCommit
        branch = 'codex/release-runner-readiness'
        target = 'x86_64-pc-windows-msvc'
        tool_versions = [pscustomobject]@{ rustc = '1.0'; cargo = '1.0'; node = '20'; npm = '10' }
        artifacts = @($artifact)
        staged_subjects = if ($OmitStagedSubjects) { @() } else { @($stagedSubject) }
        dependency_inventory = if ($MissingInventory) {
            $null
        } else {
            [pscustomobject]@{
                relative_path = 'dependency-inventory.json'
                sha256 = ('c' * 64)
                component_count = 1
                generator = 'fallback'
            }
        }
        build_status = $BuildStatus
        warnings = @()
        notes = $notes
        acceptance = [pscustomobject]@{
            gui = 'not_claimed'
            android_device = 'not_claimed'
            host_build = $BuildStatus
            remote_ci = $RemoteCi
        }
    }

    if (-not $MissingInventory) {
        $inventory = [pscustomobject]@{
            generator = 'fallback'
            components = @([pscustomobject]@{ name = 'storyforge'; version = '0.0.0' })
        }
        $invPath = Join-Path $Root 'dependency-inventory.json'
        if ($InventoryAsSymlink) {
            $outsideInv = Join-Path $outsideDir 'dependency-inventory.json'
            Write-ReleaseJson -Object $inventory -Path $outsideInv
            $null = cmd /c mklink "$invPath" "$outsideInv"
            if ($LASTEXITCODE -ne 0) {
                throw "Unable to create inventory symlink for adversarial test."
            }
            $manifest.dependency_inventory.sha256 = Get-ReleaseFileSha256 -Path $outsideInv
        } else {
            Write-ReleaseJson -Object $inventory -Path $invPath
            $manifest.dependency_inventory.sha256 = Get-ReleaseFileSha256 -Path $invPath
        }
    }

    Write-ReleaseJson -Object $manifest -Path (Join-Path $Root 'manifest.json')

    if (-not $MissingProvenance) {
        $provNotes = if ($null -ne $ProvenanceNotes) {
            @($ProvenanceNotes)
        } else {
            @('Unsigned host provenance attestation for release evidence only.')
        }
        $provSubject = if ($null -ne $stagedSubject) {
            [pscustomobject]@{
                relative_path = $stagedSubject.relative_path
                sha256 = $stagedSubject.sha256
                kind = $stagedSubject.kind
                size_bytes = if ($OmitSizeBytes) { $null } else { $stagedSubject.size_bytes }
                status = $stagedSubject.status
            }
        } else {
            [pscustomobject]@{
                relative_path = $stagedRel
                sha256 = $reportedSha
                kind = 'windows-exe'
                size_bytes = if ($OmitSizeBytes) { $null } else { $sizeBytes }
                status = 'present'
            }
        }
        if ($OmitSizeBytes) {
            $provSubject = [pscustomobject]@{
                relative_path = $provSubject.relative_path
                sha256 = $provSubject.sha256
                kind = $provSubject.kind
                status = $provSubject.status
            }
        }
        $prov = [pscustomobject]@{
            schema_version = $schemaVersion
            generated_at_utc = '2026-07-15T00:00:00Z'
            commit = if ($ProvenanceCommit) { $ProvenanceCommit } else { $defaultCommit }
            branch = if ($ProvenanceBranch) { $ProvenanceBranch } else { 'codex/release-runner-readiness' }
            target = if ($ProvenanceTarget) { $ProvenanceTarget } else { 'x86_64-pc-windows-msvc' }
            subjects = @($provSubject)
            notes = $provNotes
        }
        Write-ReleaseJson -Object $prov -Path (Join-Path $Root 'provenance.json')
    }

    return $Root
}

function Invoke-VerifyReleaseEvidenceCli {
    param(
        [Parameter(Mandatory = $true)][string]$EvidenceDir,
        [switch]$AllowDryRun
    )

    $scriptPath = Join-Path $RepoRoot 'scripts\verify-release-evidence.ps1'
    $allArgs = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $scriptPath, '-EvidenceDir', $EvidenceDir)
    if ($AllowDryRun) { $allArgs += '-AllowDryRun' }
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $output = & powershell.exe @allArgs 2>&1
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previous
    }
    return [pscustomobject]@{
        ExitCode = $code
        Output = @($output | ForEach-Object { "$_" })
    }
}

Describe 'Release runner preflight report' {
    It 'returns a machine-readable host-only ready report without leaking home paths or secrets' {
        $report = Test-ReleaseRunnerPreflight `
            -Profile 'windows-host' `
            -RepoRoot $RepoRoot `
            -ToolPresence @{
                cargo = $true
                rustc = $true
                'npm.cmd' = $true
                node = $true
                python = $true
                pyyaml = $true
                'cargo-tauri' = $false
            }

        $report.schema_version | Should Be 1
        $report.profile | Should Be 'windows-host'
        $report.status | Should Be 'ready'
        $report.ready | Should Be $true
        $report.intents.host_only | Should Be $true
        $report.intents.bundle_authorized | Should Be $false
        $report.intents.apk_authorized | Should Be $false
        $report.claims.gui | Should Be 'not_claimable'
        $report.claims.android_device | Should Be 'not_claimable'
        $report.claims.remote_ci | Should Be 'not_claimable'
        $json = $report | ConvertTo-Json -Depth 8
        $json | Should Not Match 'C:\\Users'
        $json | Should Not Match 'sk-[A-Za-z0-9]{10,}'
        $json | Should Not Match $env:USERPROFILE.Replace('\', '\\')
    }

    It 'fails closed when cargo is missing for windows-host profile' {
        { Test-ReleaseRunnerPreflight `
            -Profile 'windows-host' `
            -RepoRoot $RepoRoot `
            -ToolPresence @{ cargo = $false; rustc = $true; 'npm.cmd' = $true; node = $true; python = $true; pyyaml = $true } `
            -FailClosed } | Should Throw
    }

    It 'fails closed when rustc is missing for windows-host profile' {
        { Test-ReleaseRunnerPreflight `
            -Profile 'windows-host' `
            -RepoRoot $RepoRoot `
            -ToolPresence @{ cargo = $true; rustc = $false; 'npm.cmd' = $true; node = $true; python = $true; pyyaml = $true } `
            -FailClosed } | Should Throw
    }

    It 'fails closed when npm is missing for windows-host profile' {
        { Test-ReleaseRunnerPreflight `
            -Profile 'windows-host' `
            -RepoRoot $RepoRoot `
            -ToolPresence @{ cargo = $true; rustc = $true; 'npm.cmd' = $false; node = $true; python = $true; pyyaml = $true } `
            -FailClosed } | Should Throw
    }

    It 'fails closed when node is missing for windows-host profile' {
        { Test-ReleaseRunnerPreflight `
            -Profile 'windows-host' `
            -RepoRoot $RepoRoot `
            -ToolPresence @{ cargo = $true; rustc = $true; 'npm.cmd' = $true; node = $false; python = $true; pyyaml = $true } `
            -FailClosed } | Should Throw
    }

    It 'fails closed when python is missing for ci-gates workflow syntax profile' {
        { Test-ReleaseRunnerPreflight `
            -Profile 'ci-gates' `
            -RepoRoot $RepoRoot `
            -ToolPresence @{ cargo = $true; rustc = $true; 'npm.cmd' = $true; node = $true; python = $false; pyyaml = $false } `
            -FailClosed } | Should Throw
    }

    It 'fails closed when PyYAML is missing for ci-gates workflow syntax profile' {
        { Test-ReleaseRunnerPreflight `
            -Profile 'ci-gates' `
            -RepoRoot $RepoRoot `
            -ToolPresence @{ cargo = $true; rustc = $true; 'npm.cmd' = $true; node = $true; python = $true; pyyaml = $false } `
            -FailClosed } | Should Throw
    }

    It 'marks needs_explicit_authorization when bundle is requested without tauri-cli' {
        $report = Test-ReleaseRunnerPreflight `
            -Profile 'windows-host' `
            -RequireBundle `
            -RepoRoot $RepoRoot `
            -ToolPresence @{
                cargo = $true
                rustc = $true
                'npm.cmd' = $true
                node = $true
                python = $true
                pyyaml = $true
                'cargo-tauri' = $false
            }

        $report.ready | Should Be $false
        $report.status | Should Match 'missing_dependencies|needs_explicit'
        $report.intents.bundle_authorized | Should Be $true
        ($report.missing -join ' ') | Should Match 'tauri'
        { Test-ReleaseRunnerPreflight `
            -Profile 'windows-host' `
            -RequireBundle `
            -RepoRoot $RepoRoot `
            -ToolPresence @{
                cargo = $true; rustc = $true; 'npm.cmd' = $true; node = $true
                python = $true; pyyaml = $true; 'cargo-tauri' = $false
            } `
            -FailClosed } | Should Throw
    }

    It 'marks needs_explicit_authorization / missing deps when APK is requested without SDK/NDK' {
        $savedNdk = [Environment]::GetEnvironmentVariable('NDK_HOME')
        $savedAndroid = [Environment]::GetEnvironmentVariable('ANDROID_HOME')
        try {
            [Environment]::SetEnvironmentVariable('NDK_HOME', $null)
            [Environment]::SetEnvironmentVariable('ANDROID_HOME', $null)
            $report = Test-ReleaseRunnerPreflight `
                -Profile 'android-host' `
                -RequireApk `
                -RepoRoot $RepoRoot `
                -ToolPresence @{
                    cargo = $true
                    rustc = $true
                    'npm.cmd' = $true
                    node = $true
                    python = $true
                    pyyaml = $true
                }
            $report.ready | Should Be $false
            $report.intents.apk_authorized | Should Be $true
            ($report.missing -join ' ') | Should Match 'ANDROID_HOME|NDK_HOME'
            { Test-ReleaseRunnerPreflight `
                -Profile 'android-host' `
                -RequireApk `
                -RepoRoot $RepoRoot `
                -ToolPresence @{
                    cargo = $true; rustc = $true; 'npm.cmd' = $true; node = $true
                    python = $true; pyyaml = $true
                } `
                -FailClosed } | Should Throw
        } finally {
            [Environment]::SetEnvironmentVariable('NDK_HOME', $savedNdk)
            [Environment]::SetEnvironmentVariable('ANDROID_HOME', $savedAndroid)
        }
    }

    It 'host-only preflight does not require tauri-cli or Android SDK' {
        $report = Test-ReleaseRunnerPreflight `
            -Profile 'windows-host' `
            -RepoRoot $RepoRoot `
            -ToolPresence @{
                cargo = $true
                rustc = $true
                'npm.cmd' = $true
                node = $true
                python = $true
                pyyaml = $true
                'cargo-tauri' = $false
            }
        $report.ready | Should Be $true
        $report.intents.bundle_authorized | Should Be $false
        @($report.missing | Where-Object { $_ -match 'tauri|ANDROID_HOME|NDK_HOME' }).Count | Should Be 0
    }

    It 'report always declares GUI and device evidence are not claimable' {
        $report = Test-ReleaseRunnerPreflight -Profile 'android-host' -RepoRoot $RepoRoot -ToolPresence @{
            cargo = $true; rustc = $true; 'npm.cmd' = $true; node = $true; python = $true; pyyaml = $true
        }
        $report.claims.gui | Should Be 'not_claimable'
        $report.claims.android_device | Should Be 'not_claimable'
        $report.claims.remote_ci | Should Be 'not_claimable'
    }
}

Describe 'Release offline evidence package verifier' {
    It 'accepts a well-formed package with subjects, sidecars, inventory, and provenance' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-ok-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir | Out-Null
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $result.Valid | Should Be $true
            $result.subject_count | Should BeGreaterThan 0
            { Assert-ReleaseEvidencePackage -EvidenceDir $dir } | Should Not Throw
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed when a present subject file is missing' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-nosubj-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir -OmitSubject | Out-Null
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $result.Valid | Should Be $false
            ($result.Errors -join ' ') | Should Match 'subject|missing'
            { Assert-ReleaseEvidencePackage -EvidenceDir $dir } | Should Throw
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed when a hash sidecar is missing' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-noside-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir -OmitSidecar | Out-Null
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $result.Valid | Should Be $false
            ($result.Errors -join ' ') | Should Match 'sidecar|sha256'
            { Assert-ReleaseEvidencePackage -EvidenceDir $dir } | Should Throw
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed when subject hash does not match sidecar/provenance' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-tamper-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir -TamperHash | Out-Null
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $result.Valid | Should Be $false
            ($result.Errors -join ' ') | Should Match 'hash|mismatch|sha256'
            { Assert-ReleaseEvidencePackage -EvidenceDir $dir } | Should Throw
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed when sidecar is UTF-8 BOM encoded' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-bom-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir -BomSidecar | Out-Null
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $result.Valid | Should Be $false
            ($result.Errors -join ' ') | Should Match 'BOM|bom'
            { Assert-ReleaseEvidencePackage -EvidenceDir $dir } | Should Throw
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed on path escape outside the evidence package' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-escape-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir -PathEscape | Out-Null
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $result.Valid | Should Be $false
            ($result.Errors -join ' ') | Should Match 'path|escape|outside|within'
            { Assert-ReleaseEvidencePackage -EvidenceDir $dir } | Should Throw
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed on unknown schema version drift' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-schema-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir -UnknownSchema | Out-Null
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $result.Valid | Should Be $false
            ($result.Errors -join ' ') | Should Match 'schema'
            { Assert-ReleaseEvidencePackage -EvidenceDir $dir } | Should Throw
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed when warnings/notes contain secrets and redacts them from errors' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-secret-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir -SensitiveNote | Out-Null
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $result.Valid | Should Be $false
            ($result.Errors -join ' ') | Should Match 'secret|sensitive|redact'
            ($result.Errors -join ' ') | Should Not Match 'sk-z{10,}'
            ($result.Errors -join ' ') | Should Not Match 'sk-zzzzzzzzzz'
            { Assert-ReleaseEvidencePackage -EvidenceDir $dir } | Should Throw
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed when dependency inventory is missing for build_status=ok' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-inv-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir -MissingInventory | Out-Null
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $result.Valid | Should Be $false
            ($result.Errors -join ' ') | Should Match 'inventory'
            { Assert-ReleaseEvidencePackage -EvidenceDir $dir } | Should Throw
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed when provenance is missing for build_status=ok' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-prov-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir -MissingProvenance | Out-Null
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $result.Valid | Should Be $false
            ($result.Errors -join ' ') | Should Match 'provenance'
            { Assert-ReleaseEvidencePackage -EvidenceDir $dir } | Should Throw
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'rejects subject files that are symlinks/reparse points' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-subj-link-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir -SubjectAsSymlink | Out-Null
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $result.Valid | Should Be $false
            ($result.Errors -join ' ') | Should Match 'reparse|symlink|junction'
            { Assert-ReleaseEvidencePackage -EvidenceDir $dir } | Should Throw
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'rejects sidecar files that are symlinks/reparse points' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-side-link-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir -SidecarAsSymlink | Out-Null
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $result.Valid | Should Be $false
            ($result.Errors -join ' ') | Should Match 'reparse|symlink|junction'
            { Assert-ReleaseEvidencePackage -EvidenceDir $dir } | Should Throw
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'rejects inventory files that are symlinks/reparse points' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-inv-link-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir -InventoryAsSymlink | Out-Null
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $result.Valid | Should Be $false
            ($result.Errors -join ' ') | Should Match 'reparse|symlink|junction|inventory'
            { Assert-ReleaseEvidencePackage -EvidenceDir $dir } | Should Throw
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'rejects subjects directory that is a junction/reparse point' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-subj-junc-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir -SubjectsDirAsJunction | Out-Null
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $result.Valid | Should Be $false
            ($result.Errors -join ' ') | Should Match 'reparse|symlink|junction'
            { Assert-ReleaseEvidencePackage -EvidenceDir $dir } | Should Throw
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'redacts missing EvidenceDir path and secret-shaped input from verifier errors' {
        $homeUser = Join-Path $env:USERPROFILE ("sf-missing-ev-{0}" -f [guid]::NewGuid().ToString('N'))
        $secretish = 'sk-' + ('q' * 24)
        $missing = Join-Path $homeUser $secretish
        $result = Test-ReleaseEvidencePackage -EvidenceDir $missing
        $result.Valid | Should Be $false
        $joined = $result.Errors -join ' '
        $joined | Should Match 'missing|not found|Evidence'
        $joined | Should Not Match ([regex]::Escape($env:USERPROFILE))
        $joined | Should Not Match 'C:\\Users'
        $joined | Should Not Match ([regex]::Escape($secretish))
        $joined | Should Not Match 'sk-q{10,}'
    }

    It 'rejects provenance api_key and Bearer secrets via the generic scanner' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-prov-sec-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            $apiKey = 'api_key=' + '"' + ('x' * 20) + '"'
            $bearer = 'Authorization: Bearer ' + ('B' * 32)
            New-SyntheticEvidencePackage -Root $dir -ProvenanceNotes @($apiKey, $bearer) | Out-Null
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $result.Valid | Should Be $false
            ($result.Errors -join ' ') | Should Match 'secret|sensitive|redact'
            ($result.Errors -join ' ') | Should Not Match ([regex]::Escape(('x' * 20)))
            ($result.Errors -join ' ') | Should Not Match ([regex]::Escape(('B' * 32)))
            { Assert-ReleaseEvidencePackage -EvidenceDir $dir } | Should Throw
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'rejects remote_ci claimed/passed and reports the original claim value' {
        foreach ($claim in @('claimed', 'passed')) {
            $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-rci-{0}-{1}" -f $claim, [guid]::NewGuid().ToString('N'))
            try {
                New-SyntheticEvidencePackage -Root $dir -RemoteCi $claim | Out-Null
                $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
                $result.Valid | Should Be $false
                # Helper returns a controlled form; original claim remains visible when non-secret.
                [string]$result.remote_ci_claim | Should Match ([regex]::Escape($claim))
                [string]$result.remote_ci_claim | Should Match 'out_of_bounds|REDACTED|controlled|invalid'
                $result.remote_ci_claimed | Should Be $true
                ($result.Errors -join ' ') | Should Match 'remote_ci'
                ($result.Errors -join ' ') | Should Match $claim
                { Assert-ReleaseEvidencePackage -EvidenceDir $dir } | Should Throw
            } finally {
                Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
            }
        }
    }

    It 'fails closed when manifest and provenance commit/branch/target disagree' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-ident-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage `
                -Root $dir `
                -ProvenanceCommit 'deadbeef' `
                -ProvenanceBranch 'other-branch' `
                -ProvenanceTarget 'aarch64-linux-android' | Out-Null
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $result.Valid | Should Be $false
            ($result.Errors -join ' ') | Should Match 'commit|branch|target|identity|mismatch'
            { Assert-ReleaseEvidencePackage -EvidenceDir $dir } | Should Throw
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed for partial and failed packages instead of treating them as success' {
        foreach ($status in @('partial', 'failed')) {
            $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-status-{0}-{1}" -f $status, [guid]::NewGuid().ToString('N'))
            try {
                New-SyntheticEvidencePackage -Root $dir -BuildStatus $status -OmitSubject -OmitSidecar -MissingInventory -MissingProvenance | Out-Null
                $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
                $result.Valid | Should Be $false
                $result.build_status | Should Be $status
                ($result.Errors -join ' ') | Should Match $status
                { Assert-ReleaseEvidencePackage -EvidenceDir $dir } | Should Throw
            } finally {
                Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
            }
        }
    }

    It 'P0: rejects claimed.exe in staged subjects while only checked.exe is verified in provenance' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-claimed-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir -RunnerTopology | Out-Null
            $checkedRel = 'subjects/windows-exe/checked.exe'
            $claimedRel = 'subjects/windows-exe/claimed.exe'
            $checkedPath = Join-Path $dir ($checkedRel -replace '/', '\')
            $claimedPath = Join-Path $dir ($claimedRel -replace '/', '\')
            [System.IO.File]::WriteAllBytes($checkedPath, [byte[]](7, 7, 7, 7))
            [System.IO.File]::WriteAllBytes($claimedPath, [byte[]](8, 8, 8, 8))
            $checkedSha = Get-ReleaseFileSha256 -Path $checkedPath
            $claimedSha = Get-ReleaseFileSha256 -Path $claimedPath
            $utf8NoBom = New-Object System.Text.UTF8Encoding $false
            [System.IO.File]::WriteAllText(($checkedPath + '.sha256'), ("{0} *checked.exe" -f $checkedSha), $utf8NoBom)
            [System.IO.File]::WriteAllText(($claimedPath + '.sha256'), ("{0} *claimed.exe" -f $claimedSha), $utf8NoBom)

            $manifest = Get-Content -LiteralPath (Join-Path $dir 'manifest.json') -Raw | ConvertFrom-Json
            $prov = Get-Content -LiteralPath (Join-Path $dir 'provenance.json') -Raw | ConvertFrom-Json
            # Attack: staged subjects claim claimed.exe; provenance only checks checked.exe.
            $manifest.staged_subjects = @(
                [pscustomobject]@{
                    relative_path = $claimedRel
                    source_relative_path = 'target/release/claimed.exe'
                    size_bytes = 4
                    sha256 = $claimedSha
                    kind = 'windows-exe'
                    status = 'present'
                    hash_sidecar = ($claimedRel + '.sha256')
                }
            )
            $prov.subjects = @(
                [pscustomobject]@{
                    relative_path = $checkedRel
                    size_bytes = 4
                    sha256 = $checkedSha
                    kind = 'windows-exe'
                    status = 'present'
                }
            )
            Write-ReleaseJson -Object $manifest -Path (Join-Path $dir 'manifest.json')
            Write-ReleaseJson -Object $prov -Path (Join-Path $dir 'provenance.json')

            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $result.Valid | Should Be $false
            ($result.Errors -join ' ') | Should Match 'binding|exact-set|mismatch|missing|extra'
            ($result.Errors -join ' ') | Should Match 'claimed|checked'
            { Assert-ReleaseEvidencePackage -EvidenceDir $dir } | Should Throw
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'P0: rejects duplicate staged subjects and extra/missing set members for ok packages' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-dup-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir -RunnerTopology | Out-Null
            $rel = 'subjects/windows-exe/storyforge.exe'
            $path = Join-Path $dir ($rel -replace '/', '\')
            $sha = Get-ReleaseFileSha256 -Path $path
            $manifest = Get-Content -LiteralPath (Join-Path $dir 'manifest.json') -Raw | ConvertFrom-Json
            $prov = Get-Content -LiteralPath (Join-Path $dir 'provenance.json') -Raw | ConvertFrom-Json
            $entry = [pscustomobject]@{
                relative_path = $rel
                source_relative_path = 'target/release/storyforge.exe'
                size_bytes = 6
                sha256 = $sha
                kind = 'windows-exe'
                status = 'present'
                hash_sidecar = ($rel + '.sha256')
            }
            $provEntry = [pscustomobject]@{
                relative_path = $rel
                size_bytes = 6
                sha256 = $sha
                kind = 'windows-exe'
                status = 'present'
            }
            $manifest.staged_subjects = @($entry, $entry)
            $prov.subjects = @($provEntry)
            Write-ReleaseJson -Object $manifest -Path (Join-Path $dir 'manifest.json')
            Write-ReleaseJson -Object $prov -Path (Join-Path $dir 'provenance.json')
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $result.Valid | Should Be $false
            ($result.Errors -join ' ') | Should Match 'duplicate|binding|exact-set'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'P0: accepts real runner topology with source path != staged subjects path' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-topo-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir -RunnerTopology -SourceRelativePath 'target/release/storyforge.exe' | Out-Null
            $manifest = Get-Content -LiteralPath (Join-Path $dir 'manifest.json') -Raw | ConvertFrom-Json
            $prov = Get-Content -LiteralPath (Join-Path $dir 'provenance.json') -Raw | ConvertFrom-Json
            [string]$manifest.artifacts[0].relative_path | Should Match 'target/release'
            [string]$manifest.staged_subjects[0].relative_path | Should Match '^subjects/'
            [string]$manifest.staged_subjects[0].source_relative_path | Should Match 'target/release'
            [string]$prov.subjects[0].relative_path | Should Match '^subjects/'
            # Offline verifier must not require source path presence inside evidence dir.
            $sourceInEvidence = Join-Path $dir 'target\release\storyforge.exe'
            Test-Path -LiteralPath $sourceInEvidence | Should Be $false
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            if (-not $result.Valid) {
                throw ("expected topology package valid, errors: {0}" -f ($result.Errors -join '; '))
            }
            $result.Valid | Should Be $true
            $result.subject_count | Should BeGreaterThan 0
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'P0: never treats source-relative artifact paths as offline subject files' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-srcpath-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir -RunnerTopology | Out-Null
            # Plant a decoy at the source path inside evidence; verifier must still use staged subjects only.
            $decoyDir = Join-Path $dir 'target\release'
            New-Item -ItemType Directory -Force -Path $decoyDir | Out-Null
            $decoy = Join-Path $decoyDir 'storyforge.exe'
            [System.IO.File]::WriteAllBytes($decoy, [byte[]](9, 9, 9, 9, 9, 9, 9))
            $utf8NoBom = New-Object System.Text.UTF8Encoding $false
            $decoySha = Get-ReleaseFileSha256 -Path $decoy
            [System.IO.File]::WriteAllText(($decoy + '.sha256'), ("{0} *storyforge.exe" -f $decoySha), $utf8NoBom)

            $manifest = Get-Content -LiteralPath (Join-Path $dir 'manifest.json') -Raw | ConvertFrom-Json
            $prov = Get-Content -LiteralPath (Join-Path $dir 'provenance.json') -Raw | ConvertFrom-Json
            # Hostile: force binding against source path instead of staged subjects.
            $manifest.staged_subjects = @()
            $manifest.artifacts = @(
                [pscustomobject]@{
                    relative_path = 'target/release/storyforge.exe'
                    size_bytes = 7
                    sha256 = $decoySha
                    kind = 'windows-exe'
                    status = 'present'
                }
            )
            $prov.subjects = @(
                [pscustomobject]@{
                    relative_path = 'target/release/storyforge.exe'
                    size_bytes = 7
                    sha256 = $decoySha
                    kind = 'windows-exe'
                    status = 'present'
                }
            )
            Write-ReleaseJson -Object $manifest -Path (Join-Path $dir 'manifest.json')
            Write-ReleaseJson -Object $prov -Path (Join-Path $dir 'provenance.json')
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $result.Valid | Should Be $false
            ($result.Errors -join ' ') | Should Match 'staged|subjects/|source|binding|missing'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'P0: windows/android staging emits staged subject records distinct from source artifacts' {
        $repo = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-stage-topo-{0}" -f [guid]::NewGuid().ToString('N'))
        $evidence = Join-Path $repo 'artifacts\release-build\windows-run'
        $binDir = Join-Path $repo 'target\release'
        New-Item -ItemType Directory -Path $binDir, $evidence -Force | Out-Null
        try {
            $exe = Join-Path $binDir 'storyforge.exe'
            [System.IO.File]::WriteAllBytes($exe, [byte[]](1, 2, 3, 4, 5))
            $sha = Get-ReleaseFileSha256 -Path $exe
            $art = New-ReleaseArtifactRecord -RelativePath 'target/release/storyforge.exe' -SizeBytes 5 -Sha256 $sha -Kind 'windows-exe' -Status 'present'
            # Do not wrap with @() here: Copy-ReleaseEvidenceSubjects already returns object[].
            $staged = Copy-ReleaseEvidenceSubjects -Artifacts @($art) -EvidenceDir $evidence -RepoRoot $repo
            @($staged).Count | Should Be 1
            $record = @($staged)[0]
            [string]$record.relative_path | Should Match '^subjects/'
            [string]$record.source_relative_path | Should Be 'target/release/storyforge.exe'
            @($record.PSObject.Properties | ForEach-Object { $_.Name }) -contains 'size_bytes' | Should Be $true
            [long]$record.size_bytes | Should Be 5
            [string]$record.relative_path | Should Not Be ([string]$record.source_relative_path)
            # Provenance must bind staged path, not source path.
            $prov = New-ReleaseProvenance `
                -Commit 'abcdef0123456789abcdef0123456789abcdef01' `
                -Branch 'test' `
                -Target 'x86_64-pc-windows-msvc' `
                -Artifacts @($record) `
                -RepoRoot $repo
            [string]$prov.subjects[0].relative_path | Should Match '^subjects/'
            [string]$prov.subjects[0].relative_path | Should Not Match '^target/'
        } finally {
            Remove-Item -LiteralPath $repo -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'P0: runner-shape Copy (unary-comma object[]) flattens into manifest.staged_subjects and verifies ok' {
        $repo = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-runner-shape-{0}" -f [guid]::NewGuid().ToString('N'))
        $evidence = Join-Path $repo 'artifacts\release-build\windows-run'
        $binDir = Join-Path $repo 'target\release'
        New-Item -ItemType Directory -Path $binDir, $evidence -Force | Out-Null
        try {
            $exe = Join-Path $binDir 'storyforge.exe'
            [System.IO.File]::WriteAllBytes($exe, [byte[]](1, 2, 3, 4, 5, 6))
            $sha = Get-ReleaseFileSha256 -Path $exe
            $art = New-ReleaseArtifactRecord -RelativePath 'target/release/storyforge.exe' -SizeBytes 6 -Sha256 $sha -Kind 'windows-exe' -Status 'present'

            # Exact runner anti-pattern that nests: @(Copy-...) around unary-comma object[].
            $nested = @(Copy-ReleaseEvidenceSubjects -Artifacts @($art) -EvidenceDir $evidence -RepoRoot $repo)
            # Production helper must flatten to flat records.
            $flat = ConvertTo-ReleaseStagedSubjectArray -InputObject $nested
            @($flat).Count | Should Be 1
            $first = @($flat)[0]
            @($first.PSObject.Properties | ForEach-Object { $_.Name }) -contains 'relative_path' | Should Be $true
            [string]$first.relative_path | Should Match '^subjects/'

            $inv = [pscustomobject]@{
                relative_path = 'dependency-inventory.json'
                sha256 = ('c' * 64)
                component_count = 1
                generator = 'fallback'
            }
            $inventory = [pscustomobject]@{ generator = 'fallback'; components = @([pscustomobject]@{ name = 'storyforge'; version = '0.0.0' }) }
            $invPath = Join-Path $evidence 'dependency-inventory.json'
            Write-ReleaseJson -Object $inventory -Path $invPath
            $inv.sha256 = Get-ReleaseFileSha256 -Path $invPath

            $manifest = New-ReleaseBuildManifest `
                -Commit 'abcdef0123456789abcdef0123456789abcdef01' `
                -Branch 'codex/release-runner-readiness' `
                -Target 'x86_64-pc-windows-msvc' `
                -ToolVersions @{ rustc = '1'; cargo = '1'; node = '20'; npm = '10' } `
                -Artifacts @($art) `
                -StagedSubjects $flat `
                -DependencyInventory $inv `
                -BuildStatus 'ok' `
                -Warnings @() `
                -Notes @('host-only; GUI acceptance not claimed') `
                -RepoRoot $repo
            # staged_subjects must be flat records, not nested arrays.
            @($manifest.staged_subjects).Count | Should Be 1
            $ss0 = @($manifest.staged_subjects)[0]
            @($ss0.PSObject.Properties | ForEach-Object { $_.Name }) -contains 'source_relative_path' | Should Be $true
            [string]$ss0.source_relative_path | Should Be 'target/release/storyforge.exe'

            $prov = New-ReleaseProvenance `
                -Commit 'abcdef0123456789abcdef0123456789abcdef01' `
                -Branch 'codex/release-runner-readiness' `
                -Target 'x86_64-pc-windows-msvc' `
                -Artifacts @($flat) `
                -RepoRoot $repo
            Write-ReleaseJson -Object $manifest -Path (Join-Path $evidence 'manifest.json')
            Write-ReleaseJson -Object $prov -Path (Join-Path $evidence 'provenance.json')

            $result = Test-ReleaseEvidencePackage -EvidenceDir $evidence
            if (-not $result.Valid) {
                throw ("runner-shape ok package should verify, errors: {0}" -f ($result.Errors -join '; '))
            }
            $result.Valid | Should Be $true
            $result.subject_count | Should BeGreaterThan 0
        } finally {
            Remove-Item -LiteralPath $repo -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'P0: rejects present manifest artifacts that have no staged subject/provenance coverage' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-art-uncovered-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir -RunnerTopology | Out-Null
            $manifest = Get-Content -LiteralPath (Join-Path $dir 'manifest.json') -Raw | ConvertFrom-Json
            # Attack: add a second present artifact that is never staged/provenance-bound.
            $extra = [pscustomobject]@{
                relative_path = 'target/release/uncovered.exe'
                size_bytes = 4
                sha256 = ('e' * 64)
                kind = 'windows-exe'
                status = 'present'
            }
            $manifest.artifacts = @($manifest.artifacts) + @($extra)
            Write-ReleaseJson -Object $manifest -Path (Join-Path $dir 'manifest.json')
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $result.Valid | Should Be $false
            ($result.Errors -join ' ') | Should Match 'uncovered|mapping|exact-set|artifact|source|missing|coverage|unmapped'
            ($result.Errors -join ' ') | Should Match 'uncovered\.exe|target/release/uncovered'
            { Assert-ReleaseEvidencePackage -EvidenceDir $dir } | Should Throw
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'P1: each present staged_subject uniquely maps to one present artifact by source fields' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-map-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            # Missing source_relative_path
            New-SyntheticEvidencePackage -Root $dir -RunnerTopology | Out-Null
            $manifest = Get-Content -LiteralPath (Join-Path $dir 'manifest.json') -Raw | ConvertFrom-Json
            $manifest.staged_subjects[0].PSObject.Properties.Remove('source_relative_path')
            Write-ReleaseJson -Object $manifest -Path (Join-Path $dir 'manifest.json')
            $noSource = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $noSource.Valid | Should Be $false
            ($noSource.Errors -join ' ') | Should Match 'source_relative_path|source|mapping|orphan'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }

        $dir2 = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-map2-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            # Hash mismatch vs present artifact
            New-SyntheticEvidencePackage -Root $dir2 -RunnerTopology | Out-Null
            $manifest = Get-Content -LiteralPath (Join-Path $dir2 'manifest.json') -Raw | ConvertFrom-Json
            $manifest.staged_subjects[0].sha256 = ('d' * 64)
            $prov = Get-Content -LiteralPath (Join-Path $dir2 'provenance.json') -Raw | ConvertFrom-Json
            $prov.subjects[0].sha256 = ('d' * 64)
            Write-ReleaseJson -Object $manifest -Path (Join-Path $dir2 'manifest.json')
            Write-ReleaseJson -Object $prov -Path (Join-Path $dir2 'provenance.json')
            $mismatch = Test-ReleaseEvidencePackage -EvidenceDir $dir2
            $mismatch.Valid | Should Be $false
            ($mismatch.Errors -join ' ') | Should Match 'mapping|source|mismatch|inconsistent|sha256|size|kind'
        } finally {
            Remove-Item -LiteralPath $dir2 -Recurse -Force -ErrorAction SilentlyContinue
        }

        $dir3 = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-map3-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            # Two staged subjects claim the same present artifact (duplicate mapping)
            New-SyntheticEvidencePackage -Root $dir3 -RunnerTopology | Out-Null
            $manifest = Get-Content -LiteralPath (Join-Path $dir3 'manifest.json') -Raw | ConvertFrom-Json
            $prov = Get-Content -LiteralPath (Join-Path $dir3 'provenance.json') -Raw | ConvertFrom-Json
            $dupStaged = $manifest.staged_subjects[0]
            $dup2 = [pscustomobject]@{
                relative_path = 'subjects/windows-exe/storyforge-copy.exe'
                source_relative_path = [string]$dupStaged.source_relative_path
                size_bytes = [long]$dupStaged.size_bytes
                sha256 = [string]$dupStaged.sha256
                kind = [string]$dupStaged.kind
                status = 'present'
                hash_sidecar = 'subjects/windows-exe/storyforge-copy.exe.sha256'
            }
            $srcPath = Join-Path $dir3 'subjects\windows-exe\storyforge.exe'
            $copyPath = Join-Path $dir3 'subjects\windows-exe\storyforge-copy.exe'
            Copy-Item -LiteralPath $srcPath -Destination $copyPath -Force
            $utf8NoBom = New-Object System.Text.UTF8Encoding $false
            [System.IO.File]::WriteAllText(($copyPath + '.sha256'), ("{0} *storyforge-copy.exe" -f $dupStaged.sha256), $utf8NoBom)
            $manifest.staged_subjects = @($dupStaged, $dup2)
            $prov.subjects = @(
                [pscustomobject]@{ relative_path = $dupStaged.relative_path; size_bytes = $dupStaged.size_bytes; sha256 = $dupStaged.sha256; kind = $dupStaged.kind; status = 'present' },
                [pscustomobject]@{ relative_path = $dup2.relative_path; size_bytes = $dup2.size_bytes; sha256 = $dup2.sha256; kind = $dup2.kind; status = 'present' }
            )
            Write-ReleaseJson -Object $manifest -Path (Join-Path $dir3 'manifest.json')
            Write-ReleaseJson -Object $prov -Path (Join-Path $dir3 'provenance.json')
            $dup = Test-ReleaseEvidencePackage -EvidenceDir $dir3
            $dup.Valid | Should Be $false
            ($dup.Errors -join ' ') | Should Match 'duplicate|mapping|source'
        } finally {
            Remove-Item -LiteralPath $dir3 -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'P1: rejects sidecar content that is not a full standard sha256sum single line' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-sidecar-gram-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir -RunnerTopology | Out-Null
            $sidecar = Join-Path $dir 'subjects\windows-exe\storyforge.exe.sha256'
            $utf8NoBom = New-Object System.Text.UTF8Encoding $false
            # Hash-only line (missing binary marker and basename) must fail closed.
            $hashOnly = (Get-Content -LiteralPath $sidecar -Raw).Trim().Substring(0, 64)
            [System.IO.File]::WriteAllText($sidecar, $hashOnly, $utf8NoBom)
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $result.Valid | Should Be $false
            ($result.Errors -join ' ') | Should Match 'sidecar|grammar|sum line|sha256sum|format'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }

        $dir2 = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-sidecar-multi-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir2 -RunnerTopology | Out-Null
            $sidecar = Join-Path $dir2 'subjects\windows-exe\storyforge.exe.sha256'
            $utf8NoBom = New-Object System.Text.UTF8Encoding $false
            $line = (Get-Content -LiteralPath $sidecar -Raw).Trim()
            [System.IO.File]::WriteAllText($sidecar, ($line + "`n" + $line), $utf8NoBom)
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir2
            $result.Valid | Should Be $false
            ($result.Errors -join ' ') | Should Match 'sidecar|grammar|single|sum line|format|multi'
        } finally {
            Remove-Item -LiteralPath $dir2 -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'P1: requires non-negative size_bytes and fails closed on size mismatch or missing size' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-size-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir -RunnerTopology -ForcedSizeBytes 6 | Out-Null
            $manifest = Get-Content -LiteralPath (Join-Path $dir 'manifest.json') -Raw | ConvertFrom-Json
            $prov = Get-Content -LiteralPath (Join-Path $dir 'provenance.json') -Raw | ConvertFrom-Json
            # Negative size rejected.
            $manifest.staged_subjects[0].size_bytes = -1
            $prov.subjects[0].size_bytes = -1
            Write-ReleaseJson -Object $manifest -Path (Join-Path $dir 'manifest.json')
            Write-ReleaseJson -Object $prov -Path (Join-Path $dir 'provenance.json')
            $neg = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $neg.Valid | Should Be $false
            ($neg.Errors -join ' ') | Should Match 'size_bytes|size'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }

        $dir2 = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-size2-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir2 -RunnerTopology -OmitSizeBytes | Out-Null
            $missing = Test-ReleaseEvidencePackage -EvidenceDir $dir2
            $missing.Valid | Should Be $false
            ($missing.Errors -join ' ') | Should Match 'size_bytes|size'
        } finally {
            Remove-Item -LiteralPath $dir2 -Recurse -Force -ErrorAction SilentlyContinue
        }

        $dir3 = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-size3-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir3 -RunnerTopology -ForcedSizeBytes 99 | Out-Null
            $mismatch = Test-ReleaseEvidencePackage -EvidenceDir $dir3
            $mismatch.Valid | Should Be $false
            ($mismatch.Errors -join ' ') | Should Match 'size|mismatch'
        } finally {
            Remove-Item -LiteralPath $dir3 -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'P1: recursive scanner fail-closes on depth overflow and scans object keys' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-depth-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir -RunnerTopology | Out-Null
            $manifest = Get-Content -LiteralPath (Join-Path $dir 'manifest.json') -Raw | ConvertFrom-Json
            # Build a deep nest beyond scanner max depth (32). Write with high JSON depth
            # so ConvertTo-Json does not silently truncate the attack surface.
            $node = [pscustomobject]@{ leaf = 'ok' }
            for ($i = 0; $i -lt 40; $i++) {
                $node = [pscustomobject]@{ child = $node }
            }
            $manifest | Add-Member -NotePropertyName deep -NotePropertyValue $node -Force
            $manifestPath = Join-Path $dir 'manifest.json'
            $json = $manifest | ConvertTo-Json -Depth 100
            $utf8NoBom = New-Object System.Text.UTF8Encoding $false
            [System.IO.File]::WriteAllText($manifestPath, $json, $utf8NoBom)
            $deep = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $deep.Valid | Should Be $false
            ($deep.Errors -join ' ') | Should Match 'depth|overflow|max depth|too deep'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }

        $dir2 = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-keyscan-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir2 -RunnerTopology | Out-Null
            # Inject secret-shaped object key via raw JSON so property name is scanned.
            $raw = Get-Content -LiteralPath (Join-Path $dir2 'manifest.json') -Raw
            $raw = $raw.TrimEnd()
            if ($raw.EndsWith('}')) {
                $raw = $raw.Substring(0, $raw.Length - 1) + (', "api-key": "' + ('Z' * 20) + '"}')
            }
            $utf8NoBom = New-Object System.Text.UTF8Encoding $false
            [System.IO.File]::WriteAllText((Join-Path $dir2 'manifest.json'), $raw, $utf8NoBom)
            $keyScan = Test-ReleaseEvidencePackage -EvidenceDir $dir2
            $keyScan.Valid | Should Be $false
            ($keyScan.Errors -join ' ') | Should Match 'secret|sensitive|redact'
            ($keyScan.Errors -join ' ') | Should Not Match ([regex]::Escape(('Z' * 20)))
        } finally {
            Remove-Item -LiteralPath $dir2 -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'rejects empty commit/branch/target and non-SHA commit identity' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-sha-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir | Out-Null
            $manifest = Get-Content -LiteralPath (Join-Path $dir 'manifest.json') -Raw | ConvertFrom-Json
            $prov = Get-Content -LiteralPath (Join-Path $dir 'provenance.json') -Raw | ConvertFrom-Json
            $manifest.commit = 'not-a-sha'
            $manifest.branch = ''
            $manifest.target = ' '
            $prov.commit = 'not-a-sha'
            $prov.branch = ''
            $prov.target = ' '
            Write-ReleaseJson -Object $manifest -Path (Join-Path $dir 'manifest.json')
            Write-ReleaseJson -Object $prov -Path (Join-Path $dir 'provenance.json')
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $result.Valid | Should Be $false
            ($result.Errors -join ' ') | Should Match 'commit|branch|target|SHA|empty'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'recursively scans nested manifest/provenance strings for bare Bearer and unquoted api-key/token' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-recsec-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir | Out-Null
            $manifest = Get-Content -LiteralPath (Join-Path $dir 'manifest.json') -Raw | ConvertFrom-Json
            $prov = Get-Content -LiteralPath (Join-Path $dir 'provenance.json') -Raw | ConvertFrom-Json
            $bearer = 'Bearer ' + ('N' * 28)
            $apiKey = 'api-key: ' + ('K' * 20)
            $token = 'token=' + ('T' * 20)
            $credential = 'credential: ' + ('C' * 20)
            # Nested string fields beyond top-level notes/warnings.
            $manifest | Add-Member -NotePropertyName meta -NotePropertyValue ([pscustomobject]@{
                nested = [pscustomobject]@{ leak = $bearer }
            }) -Force
            $manifest.tool_versions | Add-Member -NotePropertyName debug -NotePropertyValue $apiKey -Force
            $prov | Add-Member -NotePropertyName builder -NotePropertyValue ([pscustomobject]@{
                env = @($token, $credential)
            }) -Force
            Write-ReleaseJson -Object $manifest -Path (Join-Path $dir 'manifest.json')
            Write-ReleaseJson -Object $prov -Path (Join-Path $dir 'provenance.json')
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $result.Valid | Should Be $false
            ($result.Errors -join ' ') | Should Match 'secret|sensitive|redact'
            ($result.Errors -join ' ') | Should Not Match ([regex]::Escape(('N' * 28)))
            ($result.Errors -join ' ') | Should Not Match ([regex]::Escape(('K' * 20)))
            ($result.Errors -join ' ') | Should Not Match ([regex]::Escape(('T' * 20)))
            ($result.Errors -join ' ') | Should Not Match ([regex]::Escape(('C' * 20)))
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'sanitizes remote_ci_claim helper output when the raw claim is secret-shaped' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-ev-rci-sec-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            $secretClaim = 'passed sk-' + ('r' * 24)
            New-SyntheticEvidencePackage -Root $dir -RemoteCi $secretClaim | Out-Null
            $result = Test-ReleaseEvidencePackage -EvidenceDir $dir
            $result.Valid | Should Be $false
            $result.remote_ci_claimed | Should Be $true
            [string]$result.remote_ci_claim | Should Not Match 'sk-r{10,}'
            [string]$result.remote_ci_claim | Should Not Match ([regex]::Escape($secretClaim))
            [string]$result.remote_ci_claim | Should Match 'REDACTED|controlled|out_of_bounds|invalid'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'documents TOCTOU trust model for offline evidence verification' {
        $result = Get-ReleaseEvidenceVerifierTrustModel
        $result.model | Should Match 'TOCTOU|snapshot|open-then-hash|handle'
        $result.notes.Count | Should BeGreaterThan 0
        ($result.notes -join ' ') | Should Match 'reparse|hash|race|trust'
        ($result.notes -join ' ') | Should Not Match 'C:\\Users'
    }
}

Describe 'Release evidence verifier CLI (real process)' {
    It 'CLI exits non-zero for missing EvidenceDir without leaking host paths or secrets' {
        $secretish = 'sk-' + ('c' * 24)
        $missing = Join-Path $env:USERPROFILE ("sf-cli-missing-{0}\{1}" -f [guid]::NewGuid().ToString('N'), $secretish)
        $cli = Invoke-VerifyReleaseEvidenceCli -EvidenceDir $missing
        $cli.ExitCode | Should Not Be 0
        $text = $cli.Output -join "`n"
        $text | Should Not Match 'VERIFICATION PASSED'
        $text | Should Match 'FAILED|ERROR|missing|not found|Evidence'
        $text | Should Not Match ([regex]::Escape($env:USERPROFILE))
        $text | Should Not Match 'C:\\Users\\'
        $text | Should Not Match ([regex]::Escape($secretish))
    }

    It 'CLI fails closed for partial packages and does not print VERIFICATION PASSED' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-cli-partial-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir -BuildStatus 'partial' -OmitSubject -OmitSidecar -MissingInventory -MissingProvenance | Out-Null
            $cli = Invoke-VerifyReleaseEvidenceCli -EvidenceDir $dir
            $cli.ExitCode | Should Not Be 0
            $text = $cli.Output -join "`n"
            $text | Should Not Match 'VERIFICATION PASSED'
            $text | Should Match 'FAILED|fail-closed|partial'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'CLI fails closed for failed packages and does not print VERIFICATION PASSED' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-cli-failed-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir -BuildStatus 'failed' -OmitSubject -OmitSidecar -MissingInventory -MissingProvenance | Out-Null
            $cli = Invoke-VerifyReleaseEvidenceCli -EvidenceDir $dir
            $cli.ExitCode | Should Not Be 0
            $text = $cli.Output -join "`n"
            $text | Should Not Match 'VERIFICATION PASSED'
            $text | Should Match 'FAILED|fail-closed|failed'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'CLI accepts a well-formed ok package with exit 0' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-cli-ok-{0}" -f [guid]::NewGuid().ToString('N'))
        try {
            New-SyntheticEvidencePackage -Root $dir | Out-Null
            $cli = Invoke-VerifyReleaseEvidenceCli -EvidenceDir $dir
            $cli.ExitCode | Should Be 0
            ($cli.Output -join "`n") | Should Match 'VERIFICATION PASSED'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Describe 'Release workflow static governance (runner readiness)' {
    It 'parses both tracked workflows with a real YAML parser engine' {
        $workflowDir = Join-Path $RepoRoot '.gitea\workflows'
        $files = @(Get-ChildItem -LiteralPath $workflowDir -Filter '*.yml' -File)
        @($files).Count | Should BeGreaterThan 0
        foreach ($f in $files) {
            $result = Test-ReleaseWorkflowSyntax -Path $f.FullName
            $result.Valid | Should Be $true
            $result.Engine | Should Match 'pyyaml|node-yaml'
            $result.Engine | Should Not Match 'powershell|none'
        }
    }

    It 'returns exact generic metadata for every parsed workflow job' {
        $ciPath = Join-Path $RepoRoot '.gitea\workflows\ci-gates.yml'
        $ci = Test-ReleaseHostEvidenceVerifierOrder -WorkflowPath $ciPath
        $ci.Engine | Should Match 'pyyaml|node-yaml'
        $ci.jobs.ContainsKey('frontend-gate') | Should Be $true
        $ci.jobs['frontend-gate'].present | Should Be $true
        $ci.jobs['frontend-gate'].runs_on | Should Be 'ubuntu-latest'
        @($ci.jobs['frontend-gate'].steps).Count | Should BeGreaterThan 0

        $windowsPath = Join-Path $RepoRoot '.gitea\workflows\windows-gates.yml'
        $windows = Test-ReleaseHostEvidenceVerifierOrder -WorkflowPath $windowsPath
        $windows.jobs.ContainsKey('secret-scan') | Should Be $true
        $windows.jobs.ContainsKey('pester-release-tests') | Should Be $true
        $windows.jobs['secret-scan'].runs_on | Should Be 'windows-latest'
    }

    It 'asserts fixed action pins, npm ci, secret scan, retention, and host-only defaults' {
        $gov = Assert-ReleaseWorkflowStaticContract -RepoRoot $RepoRoot
        if (-not $gov.Valid) {
            throw ("workflow static contract failed: {0}" -f ($gov.Errors -join '; '))
        }
        $gov.Valid | Should Be $true
        $gov.checks['actions_pinned'] | Should Be $true
        $gov.checks['npm_ci'] | Should Be $true
        $gov.checks['secret_scan'] | Should Be $true
        $gov.checks['artifact_retention'] | Should Be $true
        $gov.checks['host_only_default'] | Should Be $true
        $gov.checks['real_yaml_parser_required'] | Should Be $true
    }

    It 'Linux ci-gates workflow keeps its Linux gates; Windows gates moved to windows-gates.yml' {
        # After the Windows-gates split, ci-gates.yml holds the four Linux
        # jobs only; the PyYAML/parser-backed Windows jobs (workflow-syntax,
        # pester-release-tests, secret-scan) live in windows-gates.yml.
        # Note: ci-gates uses mirror-friendly manual rustup/node setup (not the
        # dtolnay/setup-node actions) for CN reachability; those pinned actions
        # remain present in release-host-evidence.yml and are globally checked.
        $ci = Get-Content -LiteralPath (Join-Path $RepoRoot '.gitea\workflows\ci-gates.yml') -Raw
        # Linux gates still present in ci-gates.yml.
        $ci | Should Match 'npm ci'
        $ci | Should Match 'actions/checkout@v4'
        $ci | Should Match 'cargo clippy'
        $ci | Should Match 'cargo test --workspace'
        # Both Rust jobs install the same native dependency set. Transient
        # archive.ubuntu.com failures must retry the install in-place so apt
        # reuses already downloaded archives instead of restarting the job.
        ([regex]::Matches($ci, 'for attempt in 1 2 3')).Count | Should Be 2
        ([regex]::Matches($ci, 'Acquire::Retries=3')).Count | Should Be 4
        ([regex]::Matches($ci, 'retrying with cached archives')).Count | Should Be 2
        # The PyYAML/parser-backed Windows jobs are NOT in ci-gates.yml anymore.
        $ci | Should Not Match 'PyYAML==6\.0\.2'
        $ci | Should Not Match 'Test-ReleaseWorkflowSyntax'
        # No Windows job remains in ci-gates.yml (a runs-on: windows-latest
        # directive would recreate the blocked-job problem). Comments may still
        # mention windows-latest to document the historical issue.
        $ci | Should Not Match '(?m)^\s*runs-on:\s*windows-latest'
        # ci-gates must allow a controlled manual re-run without a business commit.
        $ci | Should Match 'workflow_dispatch'
    }

    It 'windows-gates workflow carries the parser-backed Windows release gates' {
        $wf = Join-Path $RepoRoot '.gitea\workflows\windows-gates.yml'
        Test-Path -LiteralPath $wf | Should Be $true
        $text = Get-Content -LiteralPath $wf -Raw
        $text | Should Match 'PyYAML==6\.0\.2'
        $text | Should Match 'Test-ReleaseWorkflowSyntax'
        $text | Should Match 'pyyaml\|node-yaml'
        $text | Should Match 'actions/setup-python@v5'
        $text | Should Match 'actions/checkout@v4'
        # Triggered only by manual dispatch or a v* tag, not by every push.
        $text | Should Match 'workflow_dispatch'
        $text | Should Match 'tags:'
        # No job-level if-guard: a false `if` with no Windows runner never
        # reaches `skipped` on Gitea 1.26 and leaves the run non-terminal.
        $text | Should Not Match '(?m)^\s{4}if:\s'
    }

    It 'release-host-evidence defaults host-only and pins upload-artifact retention' {
        $wf = Join-Path $RepoRoot '.gitea\workflows\release-host-evidence.yml'
        $text = Get-Content -LiteralPath $wf -Raw
        $text | Should Match 'default:\s*''true'''
        $text | Should Match 'SkipBundle'
        $text | Should Match 'actions/upload-artifact@v4'
        $text | Should Match 'retention-days:\s*14'
        # Production android job must not pass -BuildApk (comment mentions are fine).
        $text | Should Not Match '(?m)^\s*[^#\r\n]*-BuildApk\b'
        $text | Should Match 'contents:\s*read'
    }

    It 'release-host-evidence both host jobs call full Assert-ReleaseEvidencePackage before upload' {
        $wf = Join-Path $RepoRoot '.gitea\workflows\release-host-evidence.yml'
        $order = Test-ReleaseHostEvidenceVerifierOrder -WorkflowPath $wf
        if (-not $order.Valid) {
            throw ("host evidence verifier order failed: {0}" -f ($order.Errors -join '; '))
        }
        $order.Engine | Should Match 'pyyaml|node-yaml'
        $order.jobs['windows-host-evidence'].HasVerifierBeforeUpload | Should Be $true
        $order.jobs['android-host-evidence'].HasVerifierBeforeUpload | Should Be $true
        $gov = Assert-ReleaseWorkflowStaticContract -RepoRoot $RepoRoot
        if (-not $gov.Valid) {
            throw ("workflow static contract failed: {0}" -f ($gov.Errors -join '; '))
        }
        $gov.checks['full_offline_verifier'] | Should Be $true
    }

    It 'fails closed when android verifier is comment-only (parsed job/step order)' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-wf-comment-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        try {
            $wf = Join-Path $dir 'release-host-evidence.yml'
            @'
name: release-host-evidence
on: workflow_dispatch
jobs:
  windows-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Offline verify Windows evidence package (fail-closed)
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          . .\scripts\release-build\ReleaseBuild.Common.ps1
          $evidenceDir = '${{ steps.evidence.outputs.dir }}'
          $result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
      - name: Upload Windows evidence
        uses: actions/upload-artifact@v4
        with:
          path: ${{ steps.evidence.outputs.dir }}
          retention-days: 14
  android-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Offline verify Android evidence package (fail-closed)
        shell: pwsh
        run: |
          # Assert-ReleaseEvidencePackage -EvidenceDir x
          Write-Host 'comment only'
      - name: Upload Android evidence
        uses: actions/upload-artifact@v4
        with:
          path: ${{ steps.evidence.outputs.dir }}
          retention-days: 14
'@ | Set-Content -LiteralPath $wf -Encoding utf8
            $order = Test-ReleaseHostEvidenceVerifierOrder -WorkflowPath $wf
            $order.Valid | Should Be $false
            ($order.Errors -join ' ') | Should Match 'android-host-evidence|Assert-ReleaseEvidencePackage|before upload|missing'
            $order.jobs['windows-host-evidence'].HasVerifierBeforeUpload | Should Be $true
            $order.jobs['android-host-evidence'].HasVerifierBeforeUpload | Should Be $false
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed when android verifier is after upload-artifact (parsed job/step order)' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-wf-after-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        try {
            $wf = Join-Path $dir 'release-host-evidence.yml'
            @'
name: release-host-evidence
on: workflow_dispatch
jobs:
  windows-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Offline verify Windows evidence package (fail-closed)
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          . .\scripts\release-build\ReleaseBuild.Common.ps1
          $evidenceDir = '${{ steps.evidence.outputs.dir }}'
          $result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
      - name: Upload Windows evidence
        uses: actions/upload-artifact@v4
        with:
          path: ${{ steps.evidence.outputs.dir }}
          retention-days: 14
  android-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Upload Android evidence
        uses: actions/upload-artifact@v4
        with:
          path: ${{ steps.evidence.outputs.dir }}
          retention-days: 14
      - name: Offline verify Android evidence package (fail-closed)
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          . .\scripts\release-build\ReleaseBuild.Common.ps1
          $evidenceDir = '${{ steps.evidence.outputs.dir }}'
          $result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
'@ | Set-Content -LiteralPath $wf -Encoding utf8
            $order = Test-ReleaseHostEvidenceVerifierOrder -WorkflowPath $wf
            $order.Valid | Should Be $false
            ($order.Errors -join ' ') | Should Match 'android-host-evidence|before upload|order|after'
            $order.jobs['android-host-evidence'].HasVerifierBeforeUpload | Should Be $false
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed when windows verifier is missing entirely (parsed job/step order)' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-wf-winmiss-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        try {
            $wf = Join-Path $dir 'release-host-evidence.yml'
            @'
name: release-host-evidence
on: workflow_dispatch
jobs:
  windows-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Upload Windows evidence
        uses: actions/upload-artifact@v4
        with:
          path: ${{ steps.evidence.outputs.dir }}
          retention-days: 14
  android-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Offline verify Android evidence package (fail-closed)
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          . .\scripts\release-build\ReleaseBuild.Common.ps1
          $evidenceDir = '${{ steps.evidence.outputs.dir }}'
          $result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
      - name: Upload Android evidence
        uses: actions/upload-artifact@v4
        with:
          path: ${{ steps.evidence.outputs.dir }}
          retention-days: 14
'@ | Set-Content -LiteralPath $wf -Encoding utf8
            $order = Test-ReleaseHostEvidenceVerifierOrder -WorkflowPath $wf
            $order.Valid | Should Be $false
            ($order.Errors -join ' ') | Should Match 'windows-host-evidence|Assert-ReleaseEvidencePackage|missing|before upload'
            $order.jobs['windows-host-evidence'].HasVerifierBeforeUpload | Should Be $false
            $order.jobs['android-host-evidence'].HasVerifierBeforeUpload | Should Be $true
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed when windows verifier is after upload-artifact (parsed job/step order)' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-wf-winafter-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        try {
            $wf = Join-Path $dir 'release-host-evidence.yml'
            @'
name: release-host-evidence
on: workflow_dispatch
jobs:
  windows-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Upload Windows evidence
        uses: actions/upload-artifact@v4
        with:
          path: ${{ steps.evidence.outputs.dir }}
          retention-days: 14
      - name: Offline verify Windows evidence package (fail-closed)
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          . .\scripts\release-build\ReleaseBuild.Common.ps1
          $evidenceDir = '${{ steps.evidence.outputs.dir }}'
          $result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
  android-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Offline verify Android evidence package (fail-closed)
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          . .\scripts\release-build\ReleaseBuild.Common.ps1
          $evidenceDir = '${{ steps.evidence.outputs.dir }}'
          $result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
      - name: Upload Android evidence
        uses: actions/upload-artifact@v4
        with:
          path: ${{ steps.evidence.outputs.dir }}
          retention-days: 14
'@ | Set-Content -LiteralPath $wf -Encoding utf8
            $order = Test-ReleaseHostEvidenceVerifierOrder -WorkflowPath $wf
            $order.Valid | Should Be $false
            ($order.Errors -join ' ') | Should Match 'windows-host-evidence|before upload|order|after'
            $order.jobs['windows-host-evidence'].HasVerifierBeforeUpload | Should Be $false
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed when both jobs only Write-Host the verifier name before upload (AST command required)' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-wf-writehost-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        try {
            $wf = Join-Path $dir 'release-host-evidence.yml'
            @'
name: release-host-evidence
on: workflow_dispatch
jobs:
  windows-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Fake verifier
        shell: pwsh
        run: |
          Write-Host 'Assert-ReleaseEvidencePackage -EvidenceDir x'
      - name: Upload Windows evidence
        uses: actions/upload-artifact@v4
        with:
          path: x
          retention-days: 14
  android-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Fake verifier
        shell: pwsh
        run: |
          Write-Host "Assert-ReleaseEvidencePackage -EvidenceDir x"
      - name: Upload Android evidence
        uses: actions/upload-artifact@v4
        with:
          path: x
          retention-days: 14
'@ | Set-Content -LiteralPath $wf -Encoding utf8
            $order = Test-ReleaseHostEvidenceVerifierOrder -WorkflowPath $wf
            $order.Valid | Should Be $false
            $order.jobs['windows-host-evidence'].HasVerifierBeforeUpload | Should Be $false
            $order.jobs['android-host-evidence'].HasVerifierBeforeUpload | Should Be $false
            ($order.Errors -join ' ') | Should Match 'Assert-ReleaseEvidencePackage|missing|command|invoke|AST|executable'
            # AST helper must not treat Write-Host string args as a real call.
            $fake = Test-ReleaseRunInvokesCommand -ScriptText "Write-Host 'Assert-ReleaseEvidencePackage -EvidenceDir x'" -CommandName 'Assert-ReleaseEvidencePackage'
            $fake | Should Be $false
            $real = Test-ReleaseRunInvokesCommand -ScriptText 'Assert-ReleaseEvidencePackage -EvidenceDir x' -CommandName 'Assert-ReleaseEvidencePackage'
            $real | Should Be $true
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed when verifier name only appears in assignments or string literals (AST command required)' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-wf-assign-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        try {
            $wf = Join-Path $dir 'release-host-evidence.yml'
            @'
name: release-host-evidence
on: workflow_dispatch
jobs:
  windows-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Fake verifier assignment
        shell: pwsh
        run: |
          $cmd = 'Assert-ReleaseEvidencePackage -EvidenceDir x'
          Write-Output $cmd
      - name: Upload Windows evidence
        uses: actions/upload-artifact@v4
        with:
          path: x
          retention-days: 14
  android-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Fake verifier assignment
        shell: pwsh
        run: |
          $name = "Assert-ReleaseEvidencePackage"
          "call $name"
      - name: Upload Android evidence
        uses: actions/upload-artifact@v4
        with:
          path: x
          retention-days: 14
'@ | Set-Content -LiteralPath $wf -Encoding utf8
            $order = Test-ReleaseHostEvidenceVerifierOrder -WorkflowPath $wf
            $order.Valid | Should Be $false
            $order.jobs['windows-host-evidence'].HasVerifierBeforeUpload | Should Be $false
            $order.jobs['android-host-evidence'].HasVerifierBeforeUpload | Should Be $false
            ($order.Errors -join ' ') | Should Match 'Assert-ReleaseEvidencePackage|missing|command|invoke|AST|executable'
            $assign = Test-ReleaseRunInvokesCommand -ScriptText "`$cmd = 'Assert-ReleaseEvidencePackage -EvidenceDir x'; Write-Output `$cmd" -CommandName 'Assert-ReleaseEvidencePackage'
            $assign | Should Be $false
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed when verifier is only inside unreachable if/function/try contexts (top-level reachable required)' {
        # Direct AST unit checks for control-flow unreachable shapes.
        (Test-ReleaseRunInvokesCommand -ScriptText 'if ($false) { Assert-ReleaseEvidencePackage -EvidenceDir x }' -CommandName 'Assert-ReleaseEvidencePackage') | Should Be $false
        (Test-ReleaseRunInvokesCommand -ScriptText 'function NotRun { Assert-ReleaseEvidencePackage -EvidenceDir x }' -CommandName 'Assert-ReleaseEvidencePackage') | Should Be $false
        (Test-ReleaseRunInvokesCommand -ScriptText 'try { throw "x" } catch { Assert-ReleaseEvidencePackage -EvidenceDir x }' -CommandName 'Assert-ReleaseEvidencePackage') | Should Be $false
        (Test-ReleaseRunInvokesCommand -ScriptText 'foreach ($i in 1) { Assert-ReleaseEvidencePackage -EvidenceDir x }' -CommandName 'Assert-ReleaseEvidencePackage') | Should Be $false
        (Test-ReleaseRunInvokesCommand -ScriptText '& { Assert-ReleaseEvidencePackage -EvidenceDir x }' -CommandName 'Assert-ReleaseEvidencePackage') | Should Be $false
        # Top-level direct call and top-level assignment RHS remain accepted.
        (Test-ReleaseRunInvokesCommand -ScriptText 'Assert-ReleaseEvidencePackage -EvidenceDir x' -CommandName 'Assert-ReleaseEvidencePackage') | Should Be $true
        (Test-ReleaseRunInvokesCommand -ScriptText '$result = Assert-ReleaseEvidencePackage -EvidenceDir x' -CommandName 'Assert-ReleaseEvidencePackage') | Should Be $true
        # Flat production-like setup (assignment + dot-source + assignment RHS) is accepted.
        $flatOk = @'
$ErrorActionPreference = "Stop"
. .\scripts\release-build\ReleaseBuild.Common.ps1
$evidenceDir = "x"
$result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
Write-Host "ok"
'@
        (Test-ReleaseRunInvokesCommand -ScriptText $flatOk -CommandName 'Assert-ReleaseEvidencePackage') | Should Be $true
        # Early return/exit before a top-level call is not a guaranteed execution path for the call.
        (Test-ReleaseRunInvokesCommand -ScriptText "return`nAssert-ReleaseEvidencePackage -EvidenceDir x" -CommandName 'Assert-ReleaseEvidencePackage') | Should Be $false
        (Test-ReleaseRunInvokesCommand -ScriptText "exit 0`nAssert-ReleaseEvidencePackage -EvidenceDir x" -CommandName 'Assert-ReleaseEvidencePackage') | Should Be $false
        # Conditional pre-verifier transfer also fails closed (not just bare return/exit/throw).
        (Test-ReleaseRunInvokesCommand -ScriptText "if (`$true) { return }`nAssert-ReleaseEvidencePackage -EvidenceDir x" -CommandName 'Assert-ReleaseEvidencePackage') | Should Be $false
        (Test-ReleaseRunInvokesCommand -ScriptText "if (`$true) { exit 0 }`nAssert-ReleaseEvidencePackage -EvidenceDir x" -CommandName 'Assert-ReleaseEvidencePackage') | Should Be $false
        (Test-ReleaseRunInvokesCommand -ScriptText "if (`$true) { throw 'x' }`nAssert-ReleaseEvidencePackage -EvidenceDir x" -CommandName 'Assert-ReleaseEvidencePackage') | Should Be $false
        (Test-ReleaseRunInvokesCommand -ScriptText "switch (1) { default { return } }`nAssert-ReleaseEvidencePackage -EvidenceDir x" -CommandName 'Assert-ReleaseEvidencePackage') | Should Be $false
        # Arbitrary pre-verifier commands (non-dot-source) fail closed.
        (Test-ReleaseRunInvokesCommand -ScriptText "Write-Host 'setup'`nAssert-ReleaseEvidencePackage -EvidenceDir x" -CommandName 'Assert-ReleaseEvidencePackage') | Should Be $false

        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-wf-unreach-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        try {
            $wf = Join-Path $dir 'release-host-evidence.yml'
            @'
name: release-host-evidence
on: workflow_dispatch
jobs:
  windows-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Unreachable if
        shell: pwsh
        run: |
          if ($false) { Assert-ReleaseEvidencePackage -EvidenceDir x }
      - name: Upload Windows evidence
        uses: actions/upload-artifact@v4
        with:
          path: x
          retention-days: 14
  android-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Uninvoked function
        shell: pwsh
        run: |
          function NotRun { Assert-ReleaseEvidencePackage -EvidenceDir x }
      - name: Upload Android evidence
        uses: actions/upload-artifact@v4
        with:
          path: x
          retention-days: 14
'@ | Set-Content -LiteralPath $wf -Encoding utf8
            $order = Test-ReleaseHostEvidenceVerifierOrder -WorkflowPath $wf
            $order.Valid | Should Be $false
            $order.jobs['windows-host-evidence'].HasVerifierBeforeUpload | Should Be $false
            $order.jobs['android-host-evidence'].HasVerifierBeforeUpload | Should Be $false
            ($order.Errors -join ' ') | Should Match 'Assert-ReleaseEvidencePackage|missing|top-level|reachable|command|executable'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed when pre-verifier if-return/exit/throw makes the top-level call non-guaranteed' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-wf-ifxfer-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        try {
            $wf = Join-Path $dir 'release-host-evidence.yml'
            @'
name: release-host-evidence
on: workflow_dispatch
jobs:
  windows-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Conditional return before verifier
        shell: pwsh
        run: |
          if ($true) { return }
          Assert-ReleaseEvidencePackage -EvidenceDir x
      - name: Upload Windows evidence
        uses: actions/upload-artifact@v4
        with:
          path: x
          retention-days: 14
  android-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Conditional exit before verifier
        shell: pwsh
        run: |
          if ($true) { exit 0 }
          $result = Assert-ReleaseEvidencePackage -EvidenceDir x
      - name: Upload Android evidence
        uses: actions/upload-artifact@v4
        with:
          path: x
          retention-days: 14
'@ | Set-Content -LiteralPath $wf -Encoding utf8
            $order = Test-ReleaseHostEvidenceVerifierOrder -WorkflowPath $wf
            $order.Valid | Should Be $false
            $order.jobs['windows-host-evidence'].HasVerifierBeforeUpload | Should Be $false
            $order.jobs['android-host-evidence'].HasVerifierBeforeUpload | Should Be $false
            ($order.Errors -join ' ') | Should Match 'Assert-ReleaseEvidencePackage|missing|control|reachable|executable|command'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed when verifier step uses shell bash despite CommandAst-shaped run text' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-wf-bashshell-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        try {
            $wf = Join-Path $dir 'release-host-evidence.yml'
            @'
name: release-host-evidence
on: workflow_dispatch
jobs:
  windows-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Offline verify Windows evidence package (fail-closed)
        shell: bash
        run: |
          $ErrorActionPreference = 'Stop'
          . .\scripts\release-build\ReleaseBuild.Common.ps1
          $evidenceDir = '${{ steps.evidence.outputs.dir }}'
          $result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
          true
      - name: Upload Windows evidence
        uses: actions/upload-artifact@v4
        with:
          path: ${{ steps.evidence.outputs.dir }}
          retention-days: 14
  android-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Offline verify Android evidence package (fail-closed)
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          . .\scripts\release-build\ReleaseBuild.Common.ps1
          $evidenceDir = '${{ steps.evidence.outputs.dir }}'
          $result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
      - name: Upload Android evidence
        uses: actions/upload-artifact@v4
        with:
          path: ${{ steps.evidence.outputs.dir }}
          retention-days: 14
'@ | Set-Content -LiteralPath $wf -Encoding utf8
            $order = Test-ReleaseHostEvidenceVerifierOrder -WorkflowPath $wf
            $order.Valid | Should Be $false
            $order.jobs['windows-host-evidence'].HasVerifierBeforeUpload | Should Be $false
            $order.jobs['android-host-evidence'].HasVerifierBeforeUpload | Should Be $true
            ($order.Errors -join ' ') | Should Match 'shell|pwsh|bash'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed when verifier step sets continue-on-error true' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-wf-coe-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        try {
            $wf = Join-Path $dir 'release-host-evidence.yml'
            @'
name: release-host-evidence
on: workflow_dispatch
jobs:
  windows-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Offline verify Windows evidence package (fail-closed)
        shell: pwsh
        continue-on-error: true
        run: |
          $ErrorActionPreference = 'Stop'
          . .\scripts\release-build\ReleaseBuild.Common.ps1
          $evidenceDir = '${{ steps.evidence.outputs.dir }}'
          $result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
      - name: Upload Windows evidence
        uses: actions/upload-artifact@v4
        with:
          path: ${{ steps.evidence.outputs.dir }}
          retention-days: 14
  android-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Offline verify Android evidence package (fail-closed)
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          . .\scripts\release-build\ReleaseBuild.Common.ps1
          $evidenceDir = '${{ steps.evidence.outputs.dir }}'
          $result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
      - name: Upload Android evidence
        uses: actions/upload-artifact@v4
        with:
          path: ${{ steps.evidence.outputs.dir }}
          retention-days: 14
'@ | Set-Content -LiteralPath $wf -Encoding utf8
            $order = Test-ReleaseHostEvidenceVerifierOrder -WorkflowPath $wf
            $order.Valid | Should Be $false
            $order.jobs['windows-host-evidence'].HasVerifierBeforeUpload | Should Be $false
            $order.jobs['android-host-evidence'].HasVerifierBeforeUpload | Should Be $true
            ($order.Errors -join ' ') | Should Match 'continue-on-error'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed when upload uses if always after a controlled verifier' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-wf-ifalways-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        try {
            $wf = Join-Path $dir 'release-host-evidence.yml'
            @'
name: release-host-evidence
on: workflow_dispatch
jobs:
  windows-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Offline verify Windows evidence package (fail-closed)
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          . .\scripts\release-build\ReleaseBuild.Common.ps1
          $evidenceDir = '${{ steps.evidence.outputs.dir }}'
          $result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
      - name: Upload Windows evidence
        if: always()
        uses: actions/upload-artifact@v4
        with:
          path: ${{ steps.evidence.outputs.dir }}
          retention-days: 14
  android-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Offline verify Android evidence package (fail-closed)
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          . .\scripts\release-build\ReleaseBuild.Common.ps1
          $evidenceDir = '${{ steps.evidence.outputs.dir }}'
          $result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
      - name: Upload Android evidence
        uses: actions/upload-artifact@v4
        with:
          path: ${{ steps.evidence.outputs.dir }}
          retention-days: 14
'@ | Set-Content -LiteralPath $wf -Encoding utf8
            $order = Test-ReleaseHostEvidenceVerifierOrder -WorkflowPath $wf
            $order.Valid | Should Be $false
            $order.jobs['windows-host-evidence'].HasVerifierBeforeUpload | Should Be $false
            $order.jobs['android-host-evidence'].HasVerifierBeforeUpload | Should Be $true
            ($order.Errors -join ' ') | Should Match 'always\(|if:'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed when verifier allows dry-run or forges common.ps1 source or path binding' {
        # Unit: script contract rejects AllowDryRun / wrong dotsource / wrong EvidenceDir.
        $good = @'
$ErrorActionPreference = 'Stop'
. .\scripts\release-build\ReleaseBuild.Common.ps1
$evidenceDir = '${{ steps.evidence.outputs.dir }}'
$result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
'@
        $cGood = Test-ReleaseVerifierStepScriptContract -ScriptText $good
        $cGood.Valid | Should Be $true

        $cDry = Test-ReleaseVerifierStepScriptContract -ScriptText @'
. .\scripts\release-build\ReleaseBuild.Common.ps1
Assert-ReleaseEvidencePackage -EvidenceDir '${{ steps.evidence.outputs.dir }}' -AllowDryRun
'@
        $cDry.Valid | Should Be $false
        ($cDry.Errors -join ' ') | Should Match 'AllowDryRun'

        $cForge = Test-ReleaseVerifierStepScriptContract -ScriptText @'
. .\scripts\evil\Fake.Common.ps1
Assert-ReleaseEvidencePackage -EvidenceDir '${{ steps.evidence.outputs.dir }}'
'@
        $cForge.Valid | Should Be $false
        ($cForge.Errors -join ' ') | Should Match 'dot-source|ReleaseBuild\.Common\.ps1'

        $cPath = Test-ReleaseVerifierStepScriptContract -ScriptText @'
. .\scripts\release-build\ReleaseBuild.Common.ps1
Assert-ReleaseEvidencePackage -EvidenceDir 'artifacts/other'
'@
        $cPath.Valid | Should Be $false
        ($cPath.Errors -join ' ') | Should Match 'EvidenceDir|steps\.evidence\.outputs\.dir'

        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-wf-bind-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        try {
            $wf = Join-Path $dir 'release-host-evidence.yml'
            @'
name: release-host-evidence
on: workflow_dispatch
jobs:
  windows-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Offline verify Windows evidence package (fail-closed)
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          . .\scripts\evil\Fake.Common.ps1
          $evidenceDir = '${{ steps.evidence.outputs.dir }}'
          $result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
      - name: Upload Windows evidence
        uses: actions/upload-artifact@v4
        with:
          path: ${{ steps.evidence.outputs.dir }}
          retention-days: 14
  android-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Offline verify Android evidence package (fail-closed)
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          . .\scripts\release-build\ReleaseBuild.Common.ps1
          $evidenceDir = '${{ steps.evidence.outputs.dir }}'
          $result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir -AllowDryRun
      - name: Upload Android evidence
        uses: actions/upload-artifact@v4
        with:
          path: some/other/path
          retention-days: 14
'@ | Set-Content -LiteralPath $wf -Encoding utf8
            $order = Test-ReleaseHostEvidenceVerifierOrder -WorkflowPath $wf
            $order.Valid | Should Be $false
            $order.jobs['windows-host-evidence'].HasVerifierBeforeUpload | Should Be $false
            $order.jobs['android-host-evidence'].HasVerifierBeforeUpload | Should Be $false
            ($order.Errors -join ' ') | Should Match 'dot-source|AllowDryRun|path|EvidenceDir|steps\.evidence\.outputs\.dir'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed on verifier or upload conditions and non-boolean continue-on-error values' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-wf-control-meta-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        try {
            $wf = Join-Path $dir 'release-host-evidence.yml'
            @'
name: release-host-evidence
on: workflow_dispatch
jobs:
  windows-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Offline verify Windows evidence package (fail-closed)
        shell: pwsh
        if: ${{ false }}
        run: |
          $ErrorActionPreference = 'Stop'
          . .\scripts\release-build\ReleaseBuild.Common.ps1
          $evidenceDir = '${{ steps.evidence.outputs.dir }}'
          $result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
      - name: Upload Windows evidence
        continue-on-error: false
        uses: actions/upload-artifact@v4
        with:
          path: ${{ steps.evidence.outputs.dir }}
  android-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Offline verify Android evidence package (fail-closed)
        shell: pwsh
        continue-on-error: 'true'
        run: |
          $ErrorActionPreference = 'Stop'
          . .\scripts\release-build\ReleaseBuild.Common.ps1
          $evidenceDir = '${{ steps.evidence.outputs.dir }}'
          $result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
      - name: Upload Android evidence
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          path: ${{ steps.evidence.outputs.dir }}
'@ | Set-Content -LiteralPath $wf -Encoding utf8
            $order = Test-ReleaseHostEvidenceVerifierOrder -WorkflowPath $wf
            $order.Valid | Should Be $false
            $order.jobs['windows-host-evidence'].HasVerifierBeforeUpload | Should Be $false
            $order.jobs['android-host-evidence'].HasVerifierBeforeUpload | Should Be $false
            ($order.Errors -join ' ') | Should Match 'if|continue-on-error|condition|failure'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'rejects suffix-spoofed or extra dot-source and dynamic EvidenceDir rebinds' {
        $suffixSpoof = @'
$ErrorActionPreference = 'Stop'
. .\evil\scripts\release-build\ReleaseBuild.Common.ps1
$evidenceDir = '${{ steps.evidence.outputs.dir }}'
$result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
'@
        $suffixResult = Test-ReleaseVerifierStepScriptContract -ScriptText $suffixSpoof
        $suffixResult.Valid | Should Be $false
        ($suffixResult.Errors -join ' ') | Should Match 'dot-source|ReleaseBuild\.Common\.ps1|exact'

        $extraSource = @'
$ErrorActionPreference = 'Stop'
. .\scripts\release-build\ReleaseBuild.Common.ps1
. .\evil\override.ps1
$evidenceDir = '${{ steps.evidence.outputs.dir }}'
$result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
'@
        $extraSourceResult = Test-ReleaseVerifierStepScriptContract -ScriptText $extraSource
        $extraSourceResult.Valid | Should Be $false
        ($extraSourceResult.Errors -join ' ') | Should Match 'dot-source|extra|only'

        $dynamicRebind = @'
$ErrorActionPreference = 'Stop'
. .\scripts\release-build\ReleaseBuild.Common.ps1
$evidenceDir = '${{ steps.evidence.outputs.dir }}'
$evidenceDir = $env:UNTRUSTED_EVIDENCE_DIR
$result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
'@
        $dynamicRebindResult = Test-ReleaseVerifierStepScriptContract -ScriptText $dynamicRebind
        $dynamicRebindResult.Valid | Should Be $false
        ($dynamicRebindResult.Errors -join ' ') | Should Match 'EvidenceDir|bound|dynamic|steps\.evidence\.outputs\.dir'
    }

    It 'requires exactly one pinned upload-artifact v4 step per host job' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-wf-extra-upload-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        try {
            $wf = Join-Path $dir 'release-host-evidence.yml'
            @'
name: release-host-evidence
on: workflow_dispatch
jobs:
  windows-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Offline verify Windows evidence package (fail-closed)
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          . .\scripts\release-build\ReleaseBuild.Common.ps1
          $evidenceDir = '${{ steps.evidence.outputs.dir }}'
          $result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
      - name: Upload verified Windows evidence
        uses: actions/upload-artifact@v4
        with:
          path: ${{ steps.evidence.outputs.dir }}
      - name: Upload extra unverified Windows evidence
        if: always()
        uses: actions/upload-artifact@v4
        with:
          path: artifacts/unverified
  android-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Offline verify Android evidence package (fail-closed)
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          . .\scripts\release-build\ReleaseBuild.Common.ps1
          $evidenceDir = '${{ steps.evidence.outputs.dir }}'
          $result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
      - name: Upload Android evidence with wrong action pin
        uses: actions/upload-artifact@v3
        with:
          path: ${{ steps.evidence.outputs.dir }}
'@ | Set-Content -LiteralPath $wf -Encoding utf8
            $order = Test-ReleaseHostEvidenceVerifierOrder -WorkflowPath $wf
            $order.Valid | Should Be $false
            $order.jobs['windows-host-evidence'].HasVerifierBeforeUpload | Should Be $false
            $order.jobs['android-host-evidence'].HasVerifierBeforeUpload | Should Be $false
            ($order.Errors -join ' ') | Should Match 'exactly one|upload-artifact@v4|extra|pinned|v4'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'rejects command-bearing setup and post-verification mutation statements' {
        $commandBearingSetup = @'
$ErrorActionPreference = 'Stop'
. .\scripts\release-build\ReleaseBuild.Common.ps1
$ignored = Invoke-Expression 'function Assert-ReleaseEvidencePackage { return @{ subject_count = 0 } }'
$evidenceDir = '${{ steps.evidence.outputs.dir }}'
$result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
'@
        $setupResult = Test-ReleaseVerifierStepScriptContract -ScriptText $commandBearingSetup
        $setupResult.Valid | Should Be $false
        ($setupResult.Errors -join ' ') | Should Match 'setup|assignment|command|flat'

        $postVerificationMutation = @'
$ErrorActionPreference = 'Stop'
. .\scripts\release-build\ReleaseBuild.Common.ps1
$evidenceDir = '${{ steps.evidence.outputs.dir }}'
$result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
Remove-Item -LiteralPath $evidenceDir -Force -Recurse
'@
        $postResult = Test-ReleaseVerifierStepScriptContract -ScriptText $postVerificationMutation
        $postResult.Valid | Should Be $false
        ($postResult.Errors -join ' ') | Should Match 'after Assert|terminal|post-verification|only'
    }

    It 'rejects command substitution hidden inside post-verification logging' {
        $postVerificationSubexpression = @'
$ErrorActionPreference = 'Stop'
. .\scripts\release-build\ReleaseBuild.Common.ps1
$evidenceDir = '${{ steps.evidence.outputs.dir }}'
$result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
Write-Host "$(Remove-Item -LiteralPath $evidenceDir -Force -Recurse)"
'@
        $result = Test-ReleaseVerifierStepScriptContract -ScriptText $postVerificationSubexpression
        $result.Valid | Should Be $false
        ($result.Errors -join ' ') | Should Match 'after Assert|terminal|post-verification|no statements'
    }

    It 'rejects abbreviated AllowDryRun parameters that PowerShell would bind at runtime' {
        $abbreviatedSwitch = @'
$ErrorActionPreference = 'Stop'
. .\scripts\release-build\ReleaseBuild.Common.ps1
$evidenceDir = '${{ steps.evidence.outputs.dir }}'
$result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir -AllowD
'@
        $result = Test-ReleaseVerifierStepScriptContract -ScriptText $abbreviatedSwitch
        $result.Valid | Should Be $false
        ($result.Errors -join ' ') | Should Match 'AllowDryRun|AllowD|dry-run'
    }

    It 'accepts only the exact controlled verifier invocation grammar' {
        $subexpressionArgument = @'
$ErrorActionPreference = 'Stop'
. .\scripts\release-build\ReleaseBuild.Common.ps1
$evidenceDir = '${{ steps.evidence.outputs.dir }}'
$result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir -Verbose:$(Remove-Item -LiteralPath $evidenceDir -Force -Recurse)
'@
        $subexpressionResult = Test-ReleaseVerifierStepScriptContract -ScriptText $subexpressionArgument
        $subexpressionResult.Valid | Should Be $false
        ($subexpressionResult.Errors -join ' ') | Should Match 'exact|EvidenceDir|argument|parameter|grammar'

        $abbreviatedEvidenceDir = @'
$ErrorActionPreference = 'Stop'
. .\scripts\release-build\ReleaseBuild.Common.ps1
$evidenceDir = '${{ steps.evidence.outputs.dir }}'
$result = Assert-ReleaseEvidencePackage -Evidence $evidenceDir
'@
        $abbreviatedResult = Test-ReleaseVerifierStepScriptContract -ScriptText $abbreviatedEvidenceDir
        $abbreviatedResult.Valid | Should Be $false
        ($abbreviatedResult.Errors -join ' ') | Should Match 'exact|EvidenceDir|parameter|grammar'
    }

    It 'rejects hidden PowerShell preambles that can execute before the controlled verifier' {
        $usingModule = @'
using module .\evil.psm1
$ErrorActionPreference = 'Stop'
. .\scripts\release-build\ReleaseBuild.Common.ps1
$evidenceDir = '${{ steps.evidence.outputs.dir }}'
$result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
'@
        $usingResult = Test-ReleaseVerifierStepScriptContract -ScriptText $usingModule
        $usingResult.Valid | Should Be $false
        ($usingResult.Errors -join ' ') | Should Match 'using|preamble|bare|module'

        $requiresModule = @'
#requires -Modules evil
$ErrorActionPreference = 'Stop'
. .\scripts\release-build\ReleaseBuild.Common.ps1
$evidenceDir = '${{ steps.evidence.outputs.dir }}'
$result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
'@
        $requiresResult = Test-ReleaseVerifierStepScriptContract -ScriptText $requiresModule
        $requiresResult.Valid | Should Be $false
        ($requiresResult.Errors -join ' ') | Should Match 'requires|preamble|bare|module'

        $paramBlock = @'
param([string]$Ignored)
$ErrorActionPreference = 'Stop'
. .\scripts\release-build\ReleaseBuild.Common.ps1
$evidenceDir = '${{ steps.evidence.outputs.dir }}'
$result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
'@
        $paramResult = Test-ReleaseVerifierStepScriptContract -ScriptText $paramBlock
        $paramResult.Valid | Should Be $false
        ($paramResult.Errors -join ' ') | Should Match 'param|preamble|bare'
    }

    It 'rejects provider-qualified assignments that can shadow the controlled verifier command' {
        $aliasOverride = @'
$ErrorActionPreference = 'Stop'
. .\scripts\release-build\ReleaseBuild.Common.ps1
${alias:Assert-ReleaseEvidencePackage} = 'Write-Output'
$evidenceDir = '${{ steps.evidence.outputs.dir }}'
$result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
'@
        $result = Test-ReleaseVerifierStepScriptContract -ScriptText $aliasOverride
        $result.Valid | Should Be $false
        ($result.Errors -join ' ') | Should Match 'local variable|qualified|scope|provider|assignment'

        $redirectionSubexpression = @'
$ErrorActionPreference = 'Stop'
. .\scripts\release-build\ReleaseBuild.Common.ps1
$evidenceDir = '${{ steps.evidence.outputs.dir }}'
$result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir > $(Set-Item -Path function:Assert-ReleaseEvidencePackage -Value { param($EvidenceDir) [pscustomobject]@{ Valid = $true } }; 'NUL')
'@
        $redirectionResult = Test-ReleaseVerifierStepScriptContract -ScriptText $redirectionSubexpression
        $redirectionResult.Valid | Should Be $false
        ($redirectionResult.Errors -join ' ') | Should Match 'redirection|exact|grammar|EvidenceDir'

        $sideEffectingLhs = @'
$ErrorActionPreference = 'Stop'
. .\scripts\release-build\ReleaseBuild.Common.ps1
$evidenceDir = '${{ steps.evidence.outputs.dir }}'
$PSVersionTable[$(Set-Content -LiteralPath (Join-Path $evidenceDir 'tampered.txt') -Value 'x'; 'audit-key')] = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
'@
        $lhsResult = Test-ReleaseVerifierStepScriptContract -ScriptText $sideEffectingLhs
        $lhsResult.Valid | Should Be $false
        ($lhsResult.Errors -join ' ') | Should Match 'simple variable|local variable|reachable|Assert'
    }

    It 'requires the verifier to run from the repository working directory' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-wf-working-dir-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        try {
            $wf = Join-Path $dir 'release-host-evidence.yml'
            @'
name: release-host-evidence
on: workflow_dispatch
jobs:
  windows-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Offline verify Windows evidence package (fail-closed)
        shell: pwsh
        working-directory: evil
        run: |
          $ErrorActionPreference = 'Stop'
          . .\scripts\release-build\ReleaseBuild.Common.ps1
          $evidenceDir = '${{ steps.evidence.outputs.dir }}'
          $result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
      - name: Upload Windows evidence
        uses: actions/upload-artifact@v4
        with:
          path: ${{ steps.evidence.outputs.dir }}
  android-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Offline verify Android evidence package (fail-closed)
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          . .\scripts\release-build\ReleaseBuild.Common.ps1
          $evidenceDir = '${{ steps.evidence.outputs.dir }}'
          $result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
      - name: Upload Android evidence
        uses: actions/upload-artifact@v4
        with:
          path: ${{ steps.evidence.outputs.dir }}
'@ | Set-Content -LiteralPath $wf -Encoding utf8
            $order = Test-ReleaseHostEvidenceVerifierOrder -WorkflowPath $wf
            $order.Valid | Should Be $false
            $order.jobs['windows-host-evidence'].HasVerifierBeforeUpload | Should Be $false
            ($order.Errors -join ' ') | Should Match 'working-directory|working directory|repository'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'requires the controlled verifier to be immediately followed by its only upload step' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-wf-between-verify-upload-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        try {
            $wf = Join-Path $dir 'release-host-evidence.yml'
            @'
name: release-host-evidence
on: workflow_dispatch
jobs:
  windows-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Offline verify Windows evidence package (fail-closed)
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          . .\scripts\release-build\ReleaseBuild.Common.ps1
          $evidenceDir = '${{ steps.evidence.outputs.dir }}'
          $result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
      - name: Mutate after verification
        shell: pwsh
        run: Remove-Item -LiteralPath '${{ steps.evidence.outputs.dir }}' -Force -Recurse
      - name: Upload Windows evidence
        uses: actions/upload-artifact@v4
        with:
          path: ${{ steps.evidence.outputs.dir }}
  android-host-evidence:
    runs-on: windows-latest
    steps:
      - name: Offline verify Android evidence package (fail-closed)
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          . .\scripts\release-build\ReleaseBuild.Common.ps1
          $evidenceDir = '${{ steps.evidence.outputs.dir }}'
          $result = Assert-ReleaseEvidencePackage -EvidenceDir $evidenceDir
      - name: Upload Android evidence
        uses: actions/upload-artifact@v4
        with:
          path: ${{ steps.evidence.outputs.dir }}
'@ | Set-Content -LiteralPath $wf -Encoding utf8
            $order = Test-ReleaseHostEvidenceVerifierOrder -WorkflowPath $wf
            $order.Valid | Should Be $false
            $order.jobs['windows-host-evidence'].HasVerifierBeforeUpload | Should Be $false
            ($order.Errors -join ' ') | Should Match 'immediately|adjacent|next step|upload'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'rejects job-level tolerance, non-root working-directory defaults, and extra workflow uploads' {
        $base = Get-Content -LiteralPath (Join-Path $RepoRoot '.gitea\workflows\release-host-evidence.yml') -Raw
        $cases = @(
            [pscustomobject]@{
                Name = 'job continue-on-error'
                Text = $base.Replace('    runs-on: windows-latest', "    runs-on: windows-latest`r`n    continue-on-error: true")
                Pattern = 'continue-on-error'
            },
            [pscustomobject]@{
                Name = 'job condition'
                Text = $base.Replace('  windows-host-evidence:', "  windows-host-evidence:`r`n    if: `${{ false }}")
                Pattern = 'job.*if:|if: condition'
            },
            [pscustomobject]@{
                Name = 'job dependency'
                Text = $base.Replace('  windows-host-evidence:', "  windows-host-evidence:`r`n    needs: skipped-prerequisite")
                Pattern = 'needs|dependency'
            },
            [pscustomobject]@{
                Name = 'workflow defaults working-directory'
                Text = ("defaults:`r`n  run:`r`n    working-directory: evil`r`n`r`n" + $base)
                Pattern = 'working-directory|repository'
            },
            [pscustomobject]@{
                Name = 'workflow environment override'
                Text = ("env:`r`n  PATH: C:\evil`r`n`r`n" + $base)
                Pattern = 'workflow env|environment|env'
            },
            [pscustomobject]@{
                Name = 'job default shell wrapper'
                Text = $base.Replace('    runs-on: windows-latest', "    runs-on: windows-latest`r`n    defaults:`r`n      run:`r`n        shell: pwsh -Command `"& {0}; exit 0`"")
                Pattern = 'defaults.run.shell|shell'
            },
            [pscustomobject]@{
                Name = 'job PATH environment override'
                Text = $base.Replace('    runs-on: windows-latest', "    runs-on: windows-latest`r`n    env:`r`n      PATH: C:\evil")
                Pattern = 'host job env|environment|env'
            },
            [pscustomobject]@{
                Name = 'job container override'
                Text = $base.Replace('    runs-on: windows-latest', "    runs-on: windows-latest`r`n    container: attacker/command-shadow:latest")
                Pattern = 'container|services'
            },
            [pscustomobject]@{
                Name = 'producer step environment override'
                Text = $base.Replace('        id: evidence', "        id: evidence`r`n        env:`r`n          PATH: C:\evil")
                Pattern = 'step.*env|env override|environment|duplicate key.*env'
            },
            [pscustomobject]@{
                Name = 'job defaults working-directory'
                Text = $base.Replace('    runs-on: windows-latest', "    runs-on: windows-latest`r`n    defaults:`r`n      run:`r`n        working-directory: evil")
                Pattern = 'working-directory|repository'
            },
            [pscustomobject]@{
                Name = 'extra artifact upload job'
                Text = ($base + @'

  unrelated-artifact:
    runs-on: windows-latest
    steps:
      - name: Upload unverified data
        uses: actions/upload-artifact@v4
        with:
          name: unrelated
          path: artifacts/unverified
'@)
                Pattern = 'no other upload|exactly one|upload-artifact'
            }
        )

        foreach ($case in $cases) {
            $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-wf-governance-{0}" -f [guid]::NewGuid().ToString('N'))
            New-Item -ItemType Directory -Force -Path $dir | Out-Null
            try {
                $wf = Join-Path $dir 'release-host-evidence.yml'
                $case.Text | Set-Content -LiteralPath $wf -Encoding utf8
                $order = Test-ReleaseHostEvidenceVerifierOrder -WorkflowPath $wf
                $order.Valid | Should Be $false
                ($order.Errors -join ' ') | Should Match $case.Pattern
            } finally {
                Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
            }
        }
    }

    It 'requires an unconditional checkout before verification and 14-day retention for each controlled upload' {
        $base = Get-Content -LiteralPath (Join-Path $RepoRoot '.gitea\workflows\release-host-evidence.yml') -Raw
        $cases = @(
            [pscustomobject]@{
                Name = 'checkout pin'
                Text = $base.Replace('uses: actions/checkout@v4', 'uses: actions/checkout@v3')
                Switches = @{ RequireCheckout = $true }
                Pattern = 'checkout|actions/checkout@v4'
            },
            [pscustomobject]@{
                Name = 'checkout condition'
                Text = $base.Replace("        uses: actions/checkout@v4", "        if: `${{ false }}`r`n        uses: actions/checkout@v4")
                Switches = @{ RequireCheckout = $true }
                Pattern = 'checkout|if:'
            },
            [pscustomobject]@{
                Name = 'checkout repository override'
                Text = $base.Replace('          fetch-depth: 0', "          fetch-depth: 0`r`n          repository: attacker/poison")
                Switches = @{ RequireCheckout = $true }
                Pattern = 'repository|checkout|ref|ssh'
            },
            [pscustomobject]@{
                Name = 'checkout unapproved with input'
                Text = $base.Replace('          fetch-depth: 0', "          fetch-depth: 0`r`n          github-server-url: https://attacker.invalid")
                Switches = @{ RequireCheckout = $true }
                Pattern = 'checkout|with|input|github-server-url|unsupported'
            },
            [pscustomobject]@{
                Name = 'checkout case-variant source override'
                Text = $base.Replace('          fetch-depth: 0', "          fetch-depth: 0`r`n          Repository: attacker/poison")
                Switches = @{ RequireCheckout = $true }
                Pattern = 'checkout|with|input|repository|unsupported'
            },
            [pscustomobject]@{
                Name = 'checkout duplicate normalized with key'
                Text = $base.Replace('          fetch-depth: 0', "          fetch-depth: 0`r`n          Fetch-Depth: 1")
                Switches = @{ RequireCheckout = $true }
                Pattern = 'checkout|unique|duplicate|with'
            },
            [pscustomobject]@{
                Name = 'retention'
                Text = $base.Replace('retention-days: 14', 'retention-days: 1')
                Switches = @{ RequireRetentionDays14 = $true }
                Pattern = 'retention-days|14'
            }
        )

        foreach ($case in $cases) {
            $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-wf-production-policy-{0}" -f [guid]::NewGuid().ToString('N'))
            New-Item -ItemType Directory -Force -Path $dir | Out-Null
            try {
                $wf = Join-Path $dir 'release-host-evidence.yml'
                $case.Text | Set-Content -LiteralPath $wf -Encoding utf8
                $splat = [hashtable]$case.Switches
                $order = Test-ReleaseHostEvidenceVerifierOrder -WorkflowPath $wf @splat
                $order.Valid | Should Be $false
                ($order.Errors -join ' ') | Should Match $case.Pattern
            } finally {
                Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
            }
        }
    }

    It 'rejects a mutable non-allowlisted action reference in the production static contract' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-release-contract-actions-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path (Join-Path $dir '.gitea\workflows') | Out-Null
        try {
            # Windows PowerShell 5's Get-Content still performs line splitting
            # after decoding; some mis-decoded multibyte comment bytes can be
            # interpreted as NEL and corrupt indentation. Read the UTF-8 text
            # directly so this fixture preserves the tracked YAML byte shape.
            $ci = [System.IO.File]::ReadAllText(
                (Join-Path $RepoRoot '.gitea\workflows\ci-gates.yml'),
                [System.Text.Encoding]::UTF8
            )
            $hostWorkflow = [System.IO.File]::ReadAllText(
                (Join-Path $RepoRoot '.gitea\workflows\release-host-evidence.yml'),
                [System.Text.Encoding]::UTF8
            )
            $ci | Set-Content -LiteralPath (Join-Path $dir '.gitea\workflows\ci-gates.yml') -Encoding utf8
            ($hostWorkflow + @'

  mutable-action:
    runs-on: windows-latest
    steps:
      - { name: Poison, uses: evil/action@main }
'@) | Set-Content -LiteralPath (Join-Path $dir '.gitea\workflows\release-host-evidence.yml') -Encoding utf8
            $contract = Assert-ReleaseWorkflowStaticContract -RepoRoot $dir
            $contract.Valid | Should Be $false
            $contract.checks['actions_pinned'] | Should Be $false
            ($contract.Errors -join ' ') | Should Match 'allowlist|evil/action@main|Action reference'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'rejects non-string and job-level uses fields instead of silently omitting them from the action allowlist' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-release-contract-uses-shape-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path (Join-Path $dir '.gitea\workflows') | Out-Null
        try {
            $ci = Get-Content -LiteralPath (Join-Path $RepoRoot '.gitea\workflows\ci-gates.yml') -Raw
            $hostWorkflow = Get-Content -LiteralPath (Join-Path $RepoRoot '.gitea\workflows\release-host-evidence.yml') -Raw
            $badHost = $hostWorkflow + @'

  malformed-action-shape:
    runs-on: windows-latest
    steps:
      - name: Non-string uses must not disappear
        uses: false

  reusable-workflow-shape:
    uses: attacker/reusable-workflow@main
'@
            $ci | Set-Content -LiteralPath (Join-Path $dir '.gitea\workflows\ci-gates.yml') -Encoding utf8
            $badHost | Set-Content -LiteralPath (Join-Path $dir '.gitea\workflows\release-host-evidence.yml') -Encoding utf8
            $contract = Assert-ReleaseWorkflowStaticContract -RepoRoot $dir
            $contract.Valid | Should Be $false
            $contract.checks['actions_pinned'] | Should Be $false
            ($contract.Errors -join ' ') | Should Match 'Action reference|Reusable|uses|allowlist'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'requires exact top-level read-only permissions and no job-level override' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-release-contract-permissions-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path (Join-Path $dir '.gitea\workflows') | Out-Null
        try {
            $ci = Get-Content -LiteralPath (Join-Path $RepoRoot '.gitea\workflows\ci-gates.yml') -Raw
            $hostWorkflow = Get-Content -LiteralPath (Join-Path $RepoRoot '.gitea\workflows\release-host-evidence.yml') -Raw
            $badCi = $ci.Replace('  contents: read', "  contents: write`r`n  packages: write")
            $badHost = $hostWorkflow.Replace('    timeout-minutes: 60', "    timeout-minutes: 60`r`n    permissions: write-all")
            $badCi | Set-Content -LiteralPath (Join-Path $dir '.gitea\workflows\ci-gates.yml') -Encoding utf8
            $badHost | Set-Content -LiteralPath (Join-Path $dir '.gitea\workflows\release-host-evidence.yml') -Encoding utf8
            $contract = Assert-ReleaseWorkflowStaticContract -RepoRoot $dir
            $contract.Valid | Should Be $false
            $contract.checks['least_privilege_permissions'] | Should Be $false
            ($contract.Errors -join ' ') | Should Match 'permissions|contents|override|read'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'rejects comment decoys for host-only, npm ci, and secret-scan claims' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-release-contract-comment-decoys-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path (Join-Path $dir '.gitea\workflows') | Out-Null
        try {
            $ci = Get-Content -LiteralPath (Join-Path $RepoRoot '.gitea\workflows\ci-gates.yml') -Raw
            $hostWorkflow = Get-Content -LiteralPath (Join-Path $RepoRoot '.gitea\workflows\release-host-evidence.yml') -Raw
            $badCi = $ci.Replace('npm ci', 'npm install').Replace('pwsh -NoProfile -File scripts/verify-release.ps1 -SecretScanOnly', 'Write-Host skipped') + "`r`n# npm ci ; SecretScanOnly`r`n"
            $badHost = $hostWorkflow.Replace("default: 'true'", "default: 'false'") + "`r`n# default: 'true'; pwsh -NoProfile -File scripts/run-release-build.ps1 -SkipBundle`r`n"
            $badCi | Set-Content -LiteralPath (Join-Path $dir '.gitea\workflows\ci-gates.yml') -Encoding utf8
            $badHost | Set-Content -LiteralPath (Join-Path $dir '.gitea\workflows\release-host-evidence.yml') -Encoding utf8
            $contract = Assert-ReleaseWorkflowStaticContract -RepoRoot $dir
            $contract.Valid | Should Be $false
            $contract.checks['host_only_default'] | Should Be $false
            $contract.checks['npm_ci'] | Should Be $false
            $contract.checks['secret_scan'] | Should Be $false
            ($contract.Errors -join ' ') | Should Match 'host-only|npm ci|secret scan'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'requires a non-conditional fresh evidence producer in the production static contract' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-release-contract-producer-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path (Join-Path $dir '.gitea\workflows') | Out-Null
        try {
            $ci = Get-Content -LiteralPath (Join-Path $RepoRoot '.gitea\workflows\ci-gates.yml') -Raw
            $hostWorkflow = Get-Content -LiteralPath (Join-Path $RepoRoot '.gitea\workflows\release-host-evidence.yml') -Raw
            $badHost = $hostWorkflow.Replace(
                '      - name: Windows host release build',
                "      - name: Windows host release build`r`n        if: `${{ false }}"
            )
            $ci | Set-Content -LiteralPath (Join-Path $dir '.gitea\workflows\ci-gates.yml') -Encoding utf8
            $badHost | Set-Content -LiteralPath (Join-Path $dir '.gitea\workflows\release-host-evidence.yml') -Encoding utf8
            $contract = Assert-ReleaseWorkflowStaticContract -RepoRoot $dir
            $contract.Valid | Should Be $false
            $contract.checks['full_offline_verifier'] | Should Be $false
            ($contract.Errors -join ' ') | Should Match 'fresh evidence producer|producer.*if|producer.*condition'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'binds the controlled producer identity to a fresh OutputDir instead of a latest-directory lookup' {
        $base = Get-Content -LiteralPath (Join-Path $RepoRoot '.gitea\workflows\release-host-evidence.yml') -Raw
        $cases = @(
            [pscustomobject]@{
                Name = 'wrong producer output id'
                Text = $base.Replace('id: evidence', 'id: stale-evidence')
                Pattern = 'id: evidence|fresh evidence producer|producer'
            },
            [pscustomobject]@{
                Name = 'no explicit output directory'
                Text = $base.Replace(' -OutputDir $evidenceDir', '')
                Pattern = 'fresh evidence producer|OutputDir|producer'
            },
            [pscustomobject]@{
                Name = 'string decoy instead of controlled producer invocation'
                Text = $base.Replace(
                    'pwsh -NoProfile -File scripts/run-android-host-pipeline.ps1 -OutputDir $evidenceDir',
                    "Write-Host 'scripts/run-android-host-pipeline.ps1 -OutputDir `$evidenceDir'"
                )
                Pattern = 'fresh evidence producer|run-android-host-pipeline|producer'
            }
        )
        foreach ($case in $cases) {
            $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-release-contract-producer-binding-{0}" -f [guid]::NewGuid().ToString('N'))
            New-Item -ItemType Directory -Force -Path $dir | Out-Null
            try {
                $wf = Join-Path $dir 'release-host-evidence.yml'
                $case.Text | Set-Content -LiteralPath $wf -Encoding utf8
                $order = Test-ReleaseHostEvidenceVerifierOrder -WorkflowPath $wf -RequireFreshProducer
                $order.Valid | Should Be $false
                ($order.Errors -join ' ') | Should Match $case.Pattern
            } finally {
                Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
            }
        }
    }

    It 'rejects PowerShell here-string decoys for executable host-only, npm ci, and secret-scan claims' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-release-contract-here-string-decoys-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path (Join-Path $dir '.gitea\workflows') | Out-Null
        try {
            $ci = Get-Content -LiteralPath (Join-Path $RepoRoot '.gitea\workflows\ci-gates.yml') -Raw
            $hostWorkflow = Get-Content -LiteralPath (Join-Path $RepoRoot '.gitea\workflows\release-host-evidence.yml') -Raw
            $badCi = $ci.Replace('npm ci', 'Write-Host skipped').Replace(
                'pwsh -NoProfile -File scripts/verify-release.ps1 -SecretScanOnly',
                'Write-Host skipped'
            ) + @'

  ast-string-decoy:
    runs-on: windows-latest
    steps:
      - name: text that must not count as commands
        shell: pwsh
        run: |
          @'
          npm ci
          pwsh -NoProfile -File scripts/verify-release.ps1 -SecretScanOnly
          '@
          Write-Host skipped
'@
            $badHost = $hostWorkflow.Replace(
                'pwsh -NoProfile -File scripts/run-release-build.ps1 -SkipBundle',
                'Write-Host skipped'
            ) + @'

  host-ast-string-decoy:
    runs-on: windows-latest
    steps:
      - name: text that must not count as a host build
        shell: pwsh
        run: |
          @'
          pwsh -NoProfile -File scripts/run-release-build.ps1 -SkipBundle
          '@
          Write-Host skipped
'@
            $badCi | Set-Content -LiteralPath (Join-Path $dir '.gitea\workflows\ci-gates.yml') -Encoding utf8
            $badHost | Set-Content -LiteralPath (Join-Path $dir '.gitea\workflows\release-host-evidence.yml') -Encoding utf8
            $contract = Assert-ReleaseWorkflowStaticContract -RepoRoot $dir
            $contract.Valid | Should Be $false
            $contract.checks['host_only_default'] | Should Be $false
            $contract.checks['npm_ci'] | Should Be $false
            $contract.checks['secret_scan'] | Should Be $false
            ($contract.Errors -join ' ') | Should Match 'host-only|npm ci|secret scan'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'does not count conditional or tolerated npm ci and secret-scan steps as release gates' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-release-contract-conditional-gates-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path (Join-Path $dir '.gitea\workflows') | Out-Null
        try {
            # Read directly as UTF-8. Windows PowerShell 5 can otherwise
            # misinterpret multibyte Chinese comment bytes while line-splitting
            # Get-Content input and corrupt the following YAML indentation.
            $ci = [System.IO.File]::ReadAllText(
                (Join-Path $RepoRoot '.gitea\workflows\ci-gates.yml'),
                [System.Text.Encoding]::UTF8
            )
            $hostWorkflow = [System.IO.File]::ReadAllText(
                (Join-Path $RepoRoot '.gitea\workflows\release-host-evidence.yml'),
                [System.Text.Encoding]::UTF8
            )
            # Comments do not contribute executable gates; remove them to keep
            # this adversarial fixture independent of legacy host code pages.
            $ci = [regex]::Replace($ci, '(?m)^[ \t]*#[^\r\n]*(?:\r?\n|$)', '')
            $badCi = $ci.Replace('npm ci', 'Write-Host skipped').Replace(
                'pwsh -NoProfile -File scripts/verify-release.ps1 -SecretScanOnly',
                'Write-Host skipped'
            ) + @'

  conditional-gate-decoy:
    runs-on: windows-latest
    steps:
      - name: Conditional npm ci must not count
        if: ${{ false }}
        shell: pwsh
        run: npm ci
      - name: Tolerated secret scan must not count
        continue-on-error: true
        shell: pwsh
        run: pwsh -NoProfile -File scripts/verify-release.ps1 -SecretScanOnly
'@
            $utf8NoBom = New-Object System.Text.UTF8Encoding $false
            [System.IO.File]::WriteAllText(
                (Join-Path $dir '.gitea\workflows\ci-gates.yml'),
                $badCi,
                $utf8NoBom
            )
            [System.IO.File]::WriteAllText(
                (Join-Path $dir '.gitea\workflows\release-host-evidence.yml'),
                $hostWorkflow,
                $utf8NoBom
            )
            $contract = Assert-ReleaseWorkflowStaticContract -RepoRoot $dir
            $contract.Valid | Should Be $false
            $contract.checks['npm_ci'] | Should Be $false
            $contract.checks['secret_scan'] | Should Be $false
            ($contract.Errors -join ' ') | Should Match 'npm[_ ]ci|secret scan'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'requires the Windows producer default and tag branch to execute the real host-only invocation' {
        $safe = @'
if ($skipBundleInput -eq 'false') {
  pwsh -NoProfile -File scripts/run-release-build.ps1 -OutputDir $evidenceDir
} else {
  pwsh -NoProfile -File scripts/run-release-build.ps1 -SkipBundle -OutputDir $evidenceDir
}
'@
        $unsafeDirectInterpolation = @'
if ('${{ github.event.inputs.skip_bundle }}' -eq 'false') {
  pwsh -NoProfile -File scripts/run-release-build.ps1 -OutputDir $evidenceDir
} else {
  pwsh -NoProfile -File scripts/run-release-build.ps1 -SkipBundle -OutputDir $evidenceDir
}
'@
        $falseBranchDecoy = @'
if ($false) {
  pwsh -NoProfile -File scripts/run-release-build.ps1 -SkipBundle -OutputDir $evidenceDir
} else {
  pwsh -NoProfile -File scripts/run-release-build.ps1 -OutputDir $evidenceDir
}
'@
        $reversedDefault = @'
if ($skipBundleInput -eq 'false') {
  pwsh -NoProfile -File scripts/run-release-build.ps1 -SkipBundle -OutputDir $evidenceDir
} else {
  pwsh -NoProfile -File scripts/run-release-build.ps1 -OutputDir $evidenceDir
}
'@
        $extraBuild = @'
if ($skipBundleInput -eq 'false') {
  pwsh -NoProfile -File scripts/run-release-build.ps1 -OutputDir $evidenceDir
} else {
  pwsh -NoProfile -File scripts/run-release-build.ps1 -SkipBundle -OutputDir $evidenceDir
  pwsh -NoProfile -File scripts/run-release-build.ps1 -OutputDir $evidenceDir
}
'@

        (Test-ReleaseWindowsHostOnlyProducerScriptContract -ScriptText $safe) | Should Be $true
        (Test-ReleaseWindowsHostOnlyProducerScriptContract -ScriptText $unsafeDirectInterpolation) | Should Be $false
        (Test-ReleaseWindowsHostOnlyProducerScriptContract -ScriptText $falseBranchDecoy) | Should Be $false
        (Test-ReleaseWindowsHostOnlyProducerScriptContract -ScriptText $reversedDefault) | Should Be $false
        (Test-ReleaseWindowsHostOnlyProducerScriptContract -ScriptText $extraBuild) | Should Be $false
    }

    It 'requires the Windows producer to validate dispatch input as inert environment data before branching' {
        $lines = @(Get-Content -LiteralPath (Join-Path $RepoRoot '.gitea\workflows\release-host-evidence.yml'))
        $runStart = -1
        $runEnd = -1
        for ($i = 0; $i -lt $lines.Count; $i++) {
            if ($lines[$i] -eq '        run: |' -and $i -gt 60 -and $i -lt 120) {
                $runStart = $i
                continue
            }
            if ($runStart -ge 0 -and $i -gt $runStart -and
                $lines[$i] -eq '      - name: Offline verify Windows evidence package (fail-closed)') {
                $runEnd = $i
                break
            }
        }
        $runStart | Should BeGreaterThan -1
        $runEnd | Should BeGreaterThan $runStart
        $body = @($lines[($runStart + 1)..($runEnd - 1)] |
            Where-Object { $_ -match '^          ' } |
            ForEach-Object { $_.Substring(10) }) -join "`n"
        # Build with the same LF separator used above. Windows PowerShell 5
        # otherwise gives this here-string CRLF endings and Replace becomes a
        # no-op, leaving the guard in the adversarial fixture.
        $inputGuard = @(
            "if (`$skipBundleInput -notin @('', 'true', 'false')) {"
            "  throw 'skip_bundle must be empty, true, or false.'"
            '}'
        ) -join "`n"
        $unsafeInterpolation = $body.Replace(
            '[string]$env:SF_RELEASE_SKIP_BUNDLE_INPUT',
            @'
[string]'${{ github.event.inputs.skip_bundle }}'
'@.Trim()
        )
        $withoutAllowlist = $body.Replace($inputGuard, '')

        (Test-ReleaseEvidenceProducerScriptContract `
            -ScriptText $body `
            -ScriptLeafName 'run-release-build.ps1' `
            -RequireWindowsHostOnly) | Should Be $true
        (Test-ReleaseEvidenceProducerScriptContract `
            -ScriptText $unsafeInterpolation `
            -ScriptLeafName 'run-release-build.ps1' `
            -RequireWindowsHostOnly) | Should Be $false
        (Test-ReleaseEvidenceProducerScriptContract `
            -ScriptText $withoutAllowlist `
            -ScriptLeafName 'run-release-build.ps1' `
            -RequireWindowsHostOnly) | Should Be $false
    }

    It 'rejects pwsh Command-before-File and false switch values' {
        $commandBeforeFile = "pwsh -NoProfile -Command 'exit 0' -File scripts/verify-release.ps1 -SecretScanOnly"
        $falseSecretSwitch = 'pwsh -NoProfile -File scripts/verify-release.ps1 -SecretScanOnly:$false'
        $falseBundleSwitch = @'
if ($skipBundleInput -eq 'false') {
  pwsh -NoProfile -File scripts/run-release-build.ps1 -OutputDir $evidenceDir
} else {
  pwsh -NoProfile -File scripts/run-release-build.ps1 -SkipBundle:$false -OutputDir $evidenceDir
}
'@

        (Test-ReleaseRunContainsPowerShellFileInvocation `
            -ScriptText $commandBeforeFile `
            -ExpectedRelativePath 'scripts/verify-release.ps1' `
            -RequiredParameter 'SecretScanOnly' `
            -TopLevelOnly) | Should Be $false
        (Test-ReleaseRunContainsPowerShellFileInvocation `
            -ScriptText $falseSecretSwitch `
            -ExpectedRelativePath 'scripts/verify-release.ps1' `
            -RequiredParameter 'SecretScanOnly' `
            -TopLevelOnly) | Should Be $false
        (Test-ReleaseWindowsHostOnlyProducerScriptContract -ScriptText $falseBundleSwitch) | Should Be $false
    }

    It 'requires flat terminal command scripts for the npm and secret-scan gates' {
        (Test-ReleaseExactNpmCiGateScript -ScriptText 'npm ci') | Should Be $true
        (Test-ReleaseExactNpmCiGateScript -ScriptText 'npm ci; exit 0') | Should Be $false
        (Test-ReleaseExactNpmCiGateScript -ScriptText "#requires -Modules evil`r`nnpm ci") | Should Be $false
        (Test-ReleaseExactSecretScanGateScript -ScriptText 'pwsh -NoProfile -File scripts/verify-release.ps1 -SecretScanOnly') | Should Be $true
        (Test-ReleaseExactSecretScanGateScript -ScriptText 'pwsh -NoProfile -File scripts/verify-release.ps1 -SecretScanOnly; exit 0') | Should Be $false
        (Test-ReleaseExactSecretScanGateScript -ScriptText 'Set-Location evil; pwsh -NoProfile -File scripts/verify-release.ps1 -SecretScanOnly') | Should Be $false
        (Test-ReleaseExactSecretScanGateScript -ScriptText "using module .\\evil.psm1`r`npwsh -NoProfile -File scripts/verify-release.ps1 -SecretScanOnly") | Should Be $false
    }

    It 'requires producer OutputDir and GITHUB_OUTPUT to bind the same controlled variable' {
        $safe = @'
$ErrorActionPreference = 'Stop'
$evidenceDir = Join-Path $PWD ("artifacts\release-build\android-gitea-" + [guid]::NewGuid().ToString('N'))
if (Test-Path -LiteralPath $evidenceDir) {
  throw 'Refusing to reuse an existing Android evidence directory.'
}
pwsh -NoProfile -File scripts/run-android-host-pipeline.ps1 -OutputDir $evidenceDir
if ($LASTEXITCODE -ne 0) {
  throw "Android host pipeline failed with exit code $LASTEXITCODE."
}
if (-not (Test-Path -LiteralPath (Join-Path $evidenceDir 'manifest.json') -PathType Leaf)) {
  throw 'Android host pipeline did not produce manifest.json in its controlled evidence directory.'
}
"dir=$evidenceDir" | Out-File -FilePath $env:GITHUB_OUTPUT -Append -Encoding utf8
'@
        $oldOutput = $safe.Replace('-OutputDir $evidenceDir', '-OutputDir C:\old-evidence')
        $reboundOutput = $safe.Replace(
            '"dir=$evidenceDir" | Out-File',
            "`$evidenceDir = 'C:\old-evidence'`r`n`"dir=`$evidenceDir`" | Out-File"
        )
        $mutatesHelper = $safe.Replace(
            'pwsh -NoProfile',
            "Set-Content scripts\release-build\ReleaseBuild.Common.ps1 'poison'`r`npwsh -NoProfile"
        )
        $extraAndroidBehavior = $safe.Replace('-OutputDir $evidenceDir', '-BuildApk -OutputDir $evidenceDir')
        $modulePreamble = "using module .\evil.psm1`r`n$($safe)"
        $requiresPreamble = "#requires -Modules evil`r`n$($safe)"
        $exitMask = $safe + "`r`nexit 0"
        $missingNativeExitCheck = $safe.Replace(@'
if ($LASTEXITCODE -ne 0) {
  throw "Android host pipeline failed with exit code $LASTEXITCODE."
}
'@, '')
        $missingFreshDirectoryCheck = $safe.Replace(@'
if (Test-Path -LiteralPath $evidenceDir) {
  throw 'Refusing to reuse an existing Android evidence directory.'
}
'@, '')
        $missingManifestCheck = $safe.Replace(@'
if (-not (Test-Path -LiteralPath (Join-Path $evidenceDir 'manifest.json') -PathType Leaf)) {
  throw 'Android host pipeline did not produce manifest.json in its controlled evidence directory.'
}
'@, '')

        (Test-ReleaseEvidenceProducerScriptContract -ScriptText $safe -ScriptLeafName 'run-android-host-pipeline.ps1') | Should Be $true
        (Test-ReleaseEvidenceProducerScriptContract -ScriptText $oldOutput -ScriptLeafName 'run-android-host-pipeline.ps1') | Should Be $false
        (Test-ReleaseEvidenceProducerScriptContract -ScriptText $reboundOutput -ScriptLeafName 'run-android-host-pipeline.ps1') | Should Be $false
        (Test-ReleaseEvidenceProducerScriptContract -ScriptText $mutatesHelper -ScriptLeafName 'run-android-host-pipeline.ps1') | Should Be $false
        (Test-ReleaseEvidenceProducerScriptContract -ScriptText $extraAndroidBehavior -ScriptLeafName 'run-android-host-pipeline.ps1') | Should Be $false
        (Test-ReleaseEvidenceProducerScriptContract -ScriptText $modulePreamble -ScriptLeafName 'run-android-host-pipeline.ps1') | Should Be $false
        (Test-ReleaseEvidenceProducerScriptContract -ScriptText $requiresPreamble -ScriptLeafName 'run-android-host-pipeline.ps1') | Should Be $false
        (Test-ReleaseEvidenceProducerScriptContract -ScriptText $exitMask -ScriptLeafName 'run-android-host-pipeline.ps1') | Should Be $false
        (Test-ReleaseEvidenceProducerScriptContract -ScriptText $missingFreshDirectoryCheck -ScriptLeafName 'run-android-host-pipeline.ps1') | Should Be $false
        (Test-ReleaseEvidenceProducerScriptContract -ScriptText $missingNativeExitCheck -ScriptLeafName 'run-android-host-pipeline.ps1') | Should Be $false
        (Test-ReleaseEvidenceProducerScriptContract -ScriptText $missingManifestCheck -ScriptLeafName 'run-android-host-pipeline.ps1') | Should Be $false
    }

    It 'rejects executable host steps that can mutate the checkout before the controlled producer' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-host-pre-producer-mutation-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        try {
            $source = Get-Content -LiteralPath (Join-Path $RepoRoot '.gitea\workflows\release-host-evidence.yml') -Raw
            $tampered = $source.Replace(
                '      - name: Install Rust toolchain',
                @'
      - name: Mutate verifier helper before evidence production
        shell: pwsh
        run: Set-Content scripts\release-build\ReleaseBuild.Common.ps1 'poison'

      - name: Install Rust toolchain
'@
            )
            $wf = Join-Path $dir 'release-host-evidence.yml'
            $tampered | Set-Content -LiteralPath $wf -Encoding utf8
            $order = Test-ReleaseHostEvidenceVerifierOrder `
                -WorkflowPath $wf `
                -RequireCheckout `
                -RequireFreshProducer `
                -RequireRetentionDays14
            $order.Valid | Should Be $false
            $order.jobs['windows-host-evidence'].HasVerifierBeforeUpload | Should Be $false
            ($order.Errors -join ' ') | Should Match 'topology|unexpected executable|run step|trusted'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'binds npm and secret gates to the intended checked-out job and working directory' {
        function New-ContractStep {
            param(
                [int]$Index,
                [string]$Uses = '',
                [string]$Run = '',
                [bool]$ShellPresent = $false,
                [string]$Shell = '',
                [bool]$WorkingDirectoryPresent = $false,
                [bool]$EnvPresent = $false
            )
            $usesPresent = -not [string]::IsNullOrWhiteSpace($Uses)
            $runPresent = -not [string]::IsNullOrWhiteSpace($Run)
            return [pscustomobject]@{
                index = $Index; uses = $Uses; run = $Run
                uses_present = $usesPresent; uses_kind = if ($usesPresent) { 'string' } else { 'null' }
                run_present = $runPresent; run_kind = if ($runPresent) { 'string' } else { 'null' }
                if_present = $false; continue_on_error_present = $false
                working_directory_present = $WorkingDirectoryPresent
                shell_present = $ShellPresent; shell_kind = if ($ShellPresent) { 'string' } else { 'null' }; shell = $Shell
                env_present = $EnvPresent
                with_raw_keys = @(); with_keys_unique = $true
            }
        }
        $frontend = [pscustomobject]@{
            present = $true; runs_on_kind = 'string'; runs_on = 'ubuntu-latest'
            if_present = $false; continue_on_error_present = $false; needs_present = $false
            defaults_run_working_directory_present = $true; defaults_run_working_directory_kind = 'string'; defaults_run_working_directory = 'frontend'
            defaults_run_shell_present = $false; env_present = $false; container_present = $false; services_present = $false
            steps = @(
                (New-ContractStep -Index 0 -Uses 'actions/checkout@v4'),
                (New-ContractStep -Index 1 -Uses 'actions/setup-node@v4'),
                (New-ContractStep -Index 2 -Run 'npm ci')
            )
        }
        $secret = [pscustomobject]@{
            present = $true; runs_on_kind = 'string'; runs_on = 'windows-latest'
            if_present = $false; continue_on_error_present = $false; needs_present = $false
            defaults_run_working_directory_present = $false; defaults_run_working_directory_kind = ''; defaults_run_working_directory = ''
            defaults_run_shell_present = $false; env_present = $false; container_present = $false; services_present = $false
            steps = @(
                (New-ContractStep -Index 0 -Uses 'actions/checkout@v4'),
                (New-ContractStep -Index 1 -Run 'pwsh -NoProfile -File scripts/verify-release.ps1 -SecretScanOnly' -ShellPresent $true -Shell 'pwsh')
            )
        }
        $metadata = [pscustomobject]@{
            WorkflowDefaultsRunWorkingDirectoryPresent = $false
            WorkflowDefaultsRunShellPresent = $false
            WorkflowEnvPresent = $false
            jobs = @{ 'frontend-gate' = $frontend; 'secret-scan' = $secret }
        }

        (Test-ReleaseCiGateJobContract -WorkflowMetadata $metadata -Gate 'npm_ci').Valid | Should Be $true
        (Test-ReleaseCiGateJobContract -WorkflowMetadata $metadata -Gate 'secret_scan').Valid | Should Be $true
        $frontend.steps[2].working_directory_present = $true
        (Test-ReleaseCiGateJobContract -WorkflowMetadata $metadata -Gate 'npm_ci').Valid | Should Be $false
        $frontend.steps[2].working_directory_present = $false
        $frontend.steps = @(
            (New-ContractStep -Index 0 -Uses 'actions/checkout@v4'),
            (New-ContractStep -Index 1 -Run "Set-Content package-lock.json 'poison'"),
            (New-ContractStep -Index 2 -Uses 'actions/setup-node@v4'),
            (New-ContractStep -Index 3 -Run 'npm ci')
        )
        (Test-ReleaseCiGateJobContract -WorkflowMetadata $metadata -Gate 'npm_ci').Valid | Should Be $false
        $secret.steps[0].index = 2
        (Test-ReleaseCiGateJobContract -WorkflowMetadata $metadata -Gate 'secret_scan').Valid | Should Be $false
        $secret.steps[0].index = 0
        $metadata.WorkflowDefaultsRunShellPresent = $true
        (Test-ReleaseCiGateJobContract -WorkflowMetadata $metadata -Gate 'npm_ci').Valid | Should Be $false
        $metadata.WorkflowDefaultsRunShellPresent = $false
        $frontend.defaults_run_shell_present = $true
        (Test-ReleaseCiGateJobContract -WorkflowMetadata $metadata -Gate 'npm_ci').Valid | Should Be $false
        $frontend.defaults_run_shell_present = $false
        $secret.steps[1].env_present = $true
        (Test-ReleaseCiGateJobContract -WorkflowMetadata $metadata -Gate 'secret_scan').Valid | Should Be $false
        $secret.steps[1].env_present = $false
        $frontend.container_present = $true
        (Test-ReleaseCiGateJobContract -WorkflowMetadata $metadata -Gate 'npm_ci').Valid | Should Be $false
        $frontend.container_present = $false
        # A step may not carry an allowed action and a command-shaped gate at
        # the same time. Gitea/GitHub action semantics for that ambiguous YAML
        # are not a proof that the run block executes.
        $frontend.steps[2].uses = 'dtolnay/rust-toolchain@stable'
        $frontend.steps[2].uses_present = $true
        (Test-ReleaseCiGateJobContract -WorkflowMetadata $metadata -Gate 'npm_ci').Valid | Should Be $false
        $frontend.steps[2].uses = ''
        $frontend.steps[2].uses_present = $false
        $secret.steps[1].uses = 'dtolnay/rust-toolchain@stable'
        $secret.steps[1].uses_present = $true
        (Test-ReleaseCiGateJobContract -WorkflowMetadata $metadata -Gate 'secret_scan').Valid | Should Be $false
    }

    It 'requires a pinned real YAML parser before parser-backed Pester contracts run' {
        function New-PesterContractStep {
            param(
                [int]$Index,
                [string]$Uses = '',
                [string]$Run = '',
                [bool]$ShellPresent = $false,
                [string]$Shell = ''
            )
            $usesPresent = -not [string]::IsNullOrWhiteSpace($Uses)
            $runPresent = -not [string]::IsNullOrWhiteSpace($Run)
            return [pscustomobject]@{
                index = $Index; uses = $Uses; run = $Run
                uses_present = $usesPresent; uses_kind = if ($usesPresent) { 'string' } else { 'null' }
                run_present = $runPresent; run_kind = if ($runPresent) { 'string' } else { 'null' }
                if_present = $false; continue_on_error_present = $false
                working_directory_present = $false
                shell_present = $ShellPresent; shell_kind = if ($ShellPresent) { 'string' } else { 'null' }; shell = $Shell
                env_present = $false
                with_raw_keys = @(); with_keys_unique = $true
            }
        }
        $pinnedInstall = @'
$ErrorActionPreference = 'Stop'
python -m pip install 'PyYAML==6.0.2'
python -c "import yaml; assert yaml.__version__ == '6.0.2'"
$requiredPester = [version]'4.10.1'
if (-not (Get-Module -ListAvailable Pester | Where-Object { $_.Version -eq $requiredPester })) {
  Install-Module Pester -RequiredVersion $requiredPester -Scope CurrentUser -Force -SkipPublisherCheck -AllowClobber
}
Import-Module Pester -RequiredVersion $requiredPester -Force
if ((Get-Module Pester | Select-Object -First 1).Version -ne $requiredPester) {
  throw "Expected Pester $requiredPester."
}
'@
        $pesterJob = [pscustomobject]@{
            present = $true; runs_on_kind = 'string'; runs_on = 'windows-latest'
            if_present = $false; continue_on_error_present = $false; needs_present = $false
            defaults_run_working_directory_present = $false; defaults_run_shell_present = $false
            env_present = $false; container_present = $false; services_present = $false
            steps = @(
                (New-PesterContractStep -Index 0 -Uses 'actions/checkout@v4'),
                (New-PesterContractStep -Index 1 -Uses 'actions/setup-python@v5'),
                (New-PesterContractStep -Index 2 -Run $pinnedInstall -ShellPresent $true -Shell 'pwsh'),
                (New-PesterContractStep -Index 3 -Run 'pwsh -NoProfile -File scripts/tests/run-release-build-tests.ps1' -ShellPresent $true -Shell 'pwsh'),
                (New-PesterContractStep -Index 4 -Run 'pwsh -NoProfile -File scripts/verify-release.ps1 -SecretScanOnly' -ShellPresent $true -Shell 'pwsh')
            )
        }
        $metadata = [pscustomobject]@{
            WorkflowDefaultsRunWorkingDirectoryPresent = $false
            WorkflowDefaultsRunShellPresent = $false
            WorkflowEnvPresent = $false
            jobs = @{ 'pester-release-tests' = $pesterJob }
        }

        (Test-ReleasePesterYamlParserReadiness -WorkflowMetadata $metadata).Valid | Should Be $true
        $pesterJob.steps[1].uses = 'actions/setup-python@v4'
        (Test-ReleasePesterYamlParserReadiness -WorkflowMetadata $metadata).Valid | Should Be $false
        $pesterJob.steps[1].uses = 'actions/setup-python@v5'
        $pesterJob.if_present = $true
        (Test-ReleasePesterYamlParserReadiness -WorkflowMetadata $metadata).Valid | Should Be $false
        $pesterJob.if_present = $false
        $pesterJob.steps[0].with_raw_keys = @('repository')
        (Test-ReleasePesterYamlParserReadiness -WorkflowMetadata $metadata).Valid | Should Be $false
        $pesterJob.steps[0].with_raw_keys = @()
        $pesterJob.steps[3].env_present = $true
        (Test-ReleasePesterYamlParserReadiness -WorkflowMetadata $metadata).Valid | Should Be $false
        $pesterJob.steps[3].env_present = $false
        # `uses` plus a correct-looking `run` is deliberately rejected: a
        # remote Actions engine may not execute the run block for that step.
        $pesterJob.steps[3].uses = 'dtolnay/rust-toolchain@stable'
        $pesterJob.steps[3].uses_present = $true
        (Test-ReleasePesterYamlParserReadiness -WorkflowMetadata $metadata).Valid | Should Be $false
        $pesterJob.steps[3].uses = ''
        $pesterJob.steps[3].uses_present = $false
        $pesterJob.steps[1].run = "Write-Host 'decoy'"
        $pesterJob.steps[1].run_present = $true
        (Test-ReleasePesterYamlParserReadiness -WorkflowMetadata $metadata).Valid | Should Be $false
    }
}

Describe 'Release dry-run offline rehash readiness' {
    It 'windows dry-run evidence can be offline-verified without claiming remote CI' {
        $outDir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-win-ready-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $outDir | Out-Null
        try {
            $scriptPath = Join-Path $RepoRoot 'scripts\run-release-build.ps1'
            $allArgs = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $scriptPath, '-DryRun', '-SkipBundle', '-OutputDir', $outDir)
            $previous = $ErrorActionPreference
            $ErrorActionPreference = 'Continue'
            try {
                $null = & powershell.exe @allArgs 2>&1
                $code = $LASTEXITCODE
            } finally {
                $ErrorActionPreference = $previous
            }
            $code | Should Be 0
            $manifestPath = Join-Path $outDir 'manifest.json'
            Test-Path -LiteralPath $manifestPath | Should Be $true
            $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
            $manifest.build_status | Should Be 'dry-run'
            $manifest.acceptance.gui | Should Be 'not_claimed'
            # dry-run packages are not full subject packages; verifier must not treat them as remote CI.
            $result = Test-ReleaseEvidencePackage -EvidenceDir $outDir -AllowDryRun
            $result.Valid | Should Be $true
            $result.remote_ci_claimed | Should Be $false
            ($result.Notes -join ' ') | Should Match 'dry-run|not remote'
        } finally {
            Remove-Item -LiteralPath $outDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Describe 'Release runner ops documentation (credential-free)' {
    It 'documents Gitea host runner setup without embedding credentials or tokens' {
        $doc = Join-Path $RepoRoot 'docs\operations\gitea-host-release-runner.md'
        Test-Path -LiteralPath $doc | Should Be $true
        $text = Get-Content -LiteralPath $doc -Raw
        $text | Should Match 'labels'
        $text | Should Match 'permissions'
        $text | Should Match 'retention'
        $text | Should Match 'concurrency'
        $text | Should Match 'workflow_dispatch'
        $text | Should Match 'host-only|SkipBundle'
        $text | Should Match 'fail closed|fail-closed'
        $text | Should Not Match 'gitea_[a-z0-9]{10,}'
        $text | Should Not Match 'eyJ[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{10,}'
        $text | Should Not Match 'sk-[A-Za-z0-9]{20,}'
        $text | Should Not Match 'password\s*=\s*\S+'
        $text | Should Match 'do not|never|without'
    }
}
