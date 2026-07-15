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
        [switch]$SubjectsDirAsJunction
    )

    New-Item -ItemType Directory -Force -Path $Root | Out-Null
    $subjectsDir = Join-Path $Root 'subjects\windows-exe'
    if (-not $SubjectsDirAsJunction) {
        New-Item -ItemType Directory -Force -Path $subjectsDir | Out-Null
    }

    $subjectRel = 'subjects/windows-exe/storyforge.exe'
    $subjectPath = Join-Path $Root ($subjectRel -replace '/', '\')
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

    if (-not $OmitSubject) {
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

    $artifactPath = if ($PathEscape) { '../outside/storyforge.exe' } else { $subjectRel }
    $artifact = New-ReleaseArtifactRecord `
        -RelativePath $artifactPath `
        -SizeBytes 6 `
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

    $manifest = [pscustomobject]@{
        schema_version = $schemaVersion
        generated_at_utc = '2026-07-15T00:00:00Z'
        commit = 'abc1234'
        branch = 'codex/release-runner-readiness'
        target = 'x86_64-pc-windows-msvc'
        tool_versions = [pscustomobject]@{ rustc = '1.0'; cargo = '1.0'; node = '20'; npm = '10' }
        artifacts = @($artifact)
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
        $prov = [pscustomobject]@{
            schema_version = $schemaVersion
            generated_at_utc = '2026-07-15T00:00:00Z'
            commit = if ($ProvenanceCommit) { $ProvenanceCommit } else { 'abc1234' }
            branch = if ($ProvenanceBranch) { $ProvenanceBranch } else { 'codex/release-runner-readiness' }
            target = if ($ProvenanceTarget) { $ProvenanceTarget } else { 'x86_64-pc-windows-msvc' }
            subjects = @(
                [pscustomobject]@{
                    relative_path = $artifactPath
                    sha256 = $reportedSha
                    kind = 'windows-exe'
                    size_bytes = 6
                    status = 'present'
                }
            )
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
                $result.remote_ci_claim | Should Be $claim
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
