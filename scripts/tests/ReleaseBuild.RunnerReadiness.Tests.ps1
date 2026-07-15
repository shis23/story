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

    It 'ci-gates workflow requires PyYAML and does not accept structural-only validation' {
        $wf = Join-Path $RepoRoot '.gitea\workflows\ci-gates.yml'
        $text = Get-Content -LiteralPath $wf -Raw
        $text | Should Match 'PyYAML==6\.0\.2'
        $text | Should Match 'Test-ReleaseWorkflowSyntax'
        $text | Should Match 'pyyaml\|node-yaml'
        $text | Should Match 'npm ci'
        $text | Should Match 'actions/checkout@v4'
        $text | Should Match 'actions/setup-node@v4'
        $text | Should Match 'actions/setup-python@v5'
        $text | Should Match 'dtolnay/rust-toolchain@stable'
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
