# Release CI evidence helpers: provenance, hash files, archive integrity,
# manifest schema validation, and workflow YAML checks.
# Run via scripts/tests/run-release-build-tests.ps1 (includes this file).

$ErrorActionPreference = 'Stop'

$RepoRoot = (& git rev-parse --show-toplevel 2>$null)
if (-not $RepoRoot) {
    throw 'Unable to locate repository root for CI evidence tests.'
}
$RepoRoot = (Resolve-Path -LiteralPath $RepoRoot).ProviderPath
$CommonPath = Join-Path $RepoRoot 'scripts\release-build\ReleaseBuild.Common.ps1'

if (-not (Test-Path -LiteralPath $CommonPath)) {
    throw "Missing ReleaseBuild.Common.ps1 at $CommonPath"
}

. $CommonPath

Describe 'ReleaseBuild provenance generation' {
    It 'builds a provenance record referencing artifacts by sha256 without secrets or paths' {
        $artifacts = @(
            (New-ReleaseArtifactRecord -RelativePath 'target/release/storyforge.exe' -SizeBytes 100 -Sha256 ('a' * 64) -Kind 'windows-exe' -Status 'present')
        )
        $prov = New-ReleaseProvenance `
            -Commit 'abc1234' `
            -Branch 'codex/release-ci-evidence' `
            -Target 'x86_64-pc-windows-msvc' `
            -Artifacts $artifacts `
            -RepoRoot 'C:\Users\Predator\project'

        $prov.schema_version | Should Be 1
        $prov.commit | Should Be 'abc1234'
        $prov.subjects.Count | Should Be 1
        $prov.subjects[0].sha256 | Should Be ('a' * 64)
        $prov.subjects[0].relative_path | Should Be 'target/release/storyforge.exe'
        # Provenance must declare it is an unsigned host attestation, not SLSA.
        ($prov.notes -join ' ') | Should Match 'unsigned|host'
        ($prov.notes -join ' ') | Should Match 'not.*SLSA|not a SLSA'
        $json = $prov | ConvertTo-Json -Depth 10
        $json | Should Not Match 'C:\\Users'
    }

    It 'redacts secrets from the provenance notes and commit fields' {
        $fake = 'sk' + '-' + ('p' * 24)
        $prov = New-ReleaseProvenance `
            -Commit "leak-$fake" `
            -Branch 'test' `
            -Target 'x86_64-pc-windows-msvc' `
            -Artifacts @() `
            -RepoRoot 'C:\Users\Predator\repo'
        $json = $prov | ConvertTo-Json -Depth 10
        $json | Should Not Match ([regex]::Escape($fake))
    }
}

Describe 'ReleaseBuild hash file writer' {
    It 'writes a <basename>.sha256 sidecar file with the expected format' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-hash-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $dir | Out-Null
        try {
            $artifactPath = Join-Path $dir 'storyforge.exe'
            [System.IO.File]::WriteAllBytes($artifactPath, [byte[]](1, 2, 3, 4))
            $hashFile = Write-ReleaseHashFile -ArtifactPath $artifactPath
            Test-Path -LiteralPath $hashFile | Should Be $true
            $hashFile | Should Be ($artifactPath + '.sha256')
            $content = (Get-Content -LiteralPath $hashFile -Raw).Trim()
            $expectedHash = Get-ReleaseFileSha256 -Path $artifactPath
            # Format: <hash> *<basename>  (SHA-256 SUM format)
            $content | Should Match ('^' + [regex]::Escape($expectedHash))
            $content | Should Match 'storyforge\.exe$'
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed when the artifact does not exist' {
        $missing = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-missing-{0}.bin" -f [guid]::NewGuid().ToString('N'))
        { Write-ReleaseHashFile -ArtifactPath $missing } | Should Throw
    }
}

Describe 'ReleaseBuild archive integrity verification' {
    It 'verifies a valid zip archive entry list without error' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-zip-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $dir | Out-Null
        try {
            $zipPath = Join-Path $dir 'test.zip'
            # Create a real zip with System.IO.Compression.
            Add-Type -AssemblyName System.IO.Compression.FileSystem
            $zip = [System.IO.Compression.ZipFile]::Open($zipPath, 'Create')
            try {
                $entry = $zip.CreateEntry('lib/arm64-v8a/libtest.so')
                $writer = New-Object System.IO.StreamWriter($entry.Open())
                $writer.Write('native')
                $writer.Close()
            } finally {
                $zip.Dispose()
            }
            { Test-ReleaseArchiveIntegrity -Path $zipPath -ExpectedKind 'apk' } | Should Not Throw
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed when the archive is missing' {
        $missing = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-nozip-{0}.zip" -f [guid]::NewGuid().ToString('N'))
        { Test-ReleaseArchiveIntegrity -Path $missing -ExpectedKind 'apk' } | Should Throw
    }

    It 'fails closed for a corrupt zip file' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-corrupt-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $dir | Out-Null
        try {
            $corrupt = Join-Path $dir 'corrupt.zip'
            [System.IO.File]::WriteAllBytes($corrupt, [byte[]](0x50, 0x4B, 0x03, 0x04, 0x00, 0x00, 0x00))
            { Test-ReleaseArchiveIntegrity -Path $corrupt -ExpectedKind 'apk' } | Should Throw
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Describe 'ReleaseBuild manifest schema validation' {
    It 'accepts a well-formed manifest with required fields' {
        $artifact = New-ReleaseArtifactRecord -RelativePath 'target/release/storyforge.exe' -SizeBytes 12 -Sha256 ('a' * 64) -Kind 'windows-exe' -Status 'present'
        $manifest = New-ReleaseBuildManifest `
            -Commit 'abc1234' -Branch 'test' -Target 'x86_64-pc-windows-msvc' `
            -ToolVersions @{ rustc = '1' } -Artifacts @($artifact) `
            -BuildStatus 'ok' -Warnings @() -Notes @()
        { Assert-ReleaseManifestSchema -Manifest $manifest } | Should Not Throw
    }

    It 'rejects a manifest missing required top-level fields' {
        $bad = [pscustomobject]@{ commit = 'abc' }
        { Assert-ReleaseManifestSchema -Manifest $bad } | Should Throw
    }

    It 'rejects a manifest with an invalid build_status' {
        $bad = [pscustomobject]@{
            schema_version = 1
            generated_at_utc = '2026-01-01T00:00:00Z'
            commit = 'abc'
            branch = 'test'
            target = 'x86_64-pc-windows-msvc'
            tool_versions = @{}
            artifacts = @()
            dependency_inventory = $null
            build_status = 'awesome'
            warnings = @()
            notes = @()
            acceptance = [pscustomobject]@{ gui = 'not_claimed'; android_device = 'not_claimed'; host_build = 'awesome' }
        }
        { Assert-ReleaseManifestSchema -Manifest $bad } | Should Throw
    }

    It 'rejects a manifest that claims GUI acceptance' {
        $bad = [pscustomobject]@{
            schema_version = 1
            generated_at_utc = '2026-01-01T00:00:00Z'
            commit = 'abc'
            branch = 'test'
            target = 'x86_64-pc-windows-msvc'
            tool_versions = @{}
            artifacts = @()
            dependency_inventory = $null
            build_status = 'ok'
            warnings = @()
            notes = @()
            acceptance = [pscustomobject]@{ gui = 'accepted'; android_device = 'not_claimed'; host_build = 'ok' }
        }
        { Assert-ReleaseManifestSchema -Manifest $bad } | Should Throw
    }

    It 'rejects an artifact record with a present status but null sha256' {
        $manifest = New-ReleaseBuildManifest `
            -Commit 'abc' -Branch 'test' -Target 'x86_64-pc-windows-msvc' `
            -ToolVersions @{} -Artifacts @(
                (New-ReleaseArtifactRecord -RelativePath 'x.exe' -SizeBytes 0 -Sha256 $null -Kind 'windows-exe' -Status 'present')
            ) `
            -BuildStatus 'ok' -Warnings @() -Notes @()
        { Assert-ReleaseManifestSchema -Manifest $manifest } | Should Throw
    }
}

Describe 'ReleaseBuild workflow YAML validation' {
    It 'validates a well-formed workflow file parses without error' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-wf-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $dir | Out-Null
        try {
            $wf = Join-Path $dir 'test.yml'
            @(
                'name: test'
                'on: [push]'
                'jobs:'
                '  build:'
                '    runs-on: ubuntu-latest'
                '    steps:'
                '      - run: echo hello'
            ) | Set-Content -LiteralPath $wf -Encoding utf8
            $result = Test-ReleaseWorkflowSyntax -Path $wf
            $result.Valid | Should Be $true
            $result.ErrorCount | Should Be 0
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'detects invalid YAML syntax' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-wf-bad-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $dir | Out-Null
        try {
            $wf = Join-Path $dir 'bad.yml'
            @(
                'name: test'
                'on: [push'
                'jobs:'
                '  : bad indent'
                '    - run: oops'
            ) | Set-Content -LiteralPath $wf -Encoding utf8
            $result = Test-ReleaseWorkflowSyntax -Path $wf
            $result.Valid | Should Be $false
            $result.ErrorCount | Should BeGreaterThan 0
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed for a missing workflow file' {
        $missing = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-no-wf-{0}.yml" -f [guid]::NewGuid().ToString('N'))
        { Test-ReleaseWorkflowSyntax -Path $missing } | Should Throw
    }
}

Describe 'ReleaseBuild Gitea workflow governance checks' {
    It 'all tracked workflow files parse without syntax errors' {
        $workflowDir = Join-Path $RepoRoot '.gitea\workflows'
        $files = @(Get-ChildItem -LiteralPath $workflowDir -Filter '*.yml' -File -ErrorAction SilentlyContinue) +
                 @(Get-ChildItem -LiteralPath $workflowDir -Filter '*.yaml' -File -ErrorAction SilentlyContinue)
        @($files).Count | Should BeGreaterThan 0
        foreach ($f in $files) {
            $result = Test-ReleaseWorkflowSyntax -Path $f.FullName
            $result.Valid | Should Be $true
        }
    }

    It 'workflows do not hard-code the workstation CARGO_TARGET_DIR path' {
        $workflowDir = Join-Path $RepoRoot '.gitea\workflows'
        $files = @(Get-ChildItem -LiteralPath $workflowDir -File -ErrorAction SilentlyContinue)
        foreach ($f in $files) {
            $text = Get-Content -LiteralPath $f.FullName -Raw
            $text | Should Not Match 'storyforge-parallel-target'
        }
    }

    It 'workflows use timeouts-minutes on artifact-producing jobs' {
        $workflowDir = Join-Path $RepoRoot '.gitea\workflows'
        $files = @(Get-ChildItem -LiteralPath $workflowDir -File -ErrorAction SilentlyContinue)
        foreach ($f in $files) {
            $text = Get-Content -LiteralPath $f.FullName -Raw
            # Every workflow must mention a timeout somewhere.
            $text | Should Match 'timeout-minutes'
        }
    }

    It 'workflows use concurrency cancellation and least-privilege permissions' {
        $workflowDir = Join-Path $RepoRoot '.gitea\workflows'
        $files = @(Get-ChildItem -LiteralPath $workflowDir -File -ErrorAction SilentlyContinue)
        foreach ($f in $files) {
            $text = Get-Content -LiteralPath $f.FullName -Raw
            $text | Should Match 'concurrency'
            $text | Should Match 'cancel-in-progress'
            $text | Should Match 'contents: read'
        }
    }
}

Describe 'ReleaseBuild retention empty-target guard' {
    It 'Get-ReleaseRetentionCleanupTargets returns empty without error when nothing to delete' {
        $root = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-empty-ret-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $root | Out-Null
        try {
            # Only one run dir; keep=5 means nothing to delete.
            $run = Join-Path $root 'windows-run-01'
            New-Item -ItemType Directory -Path $run | Out-Null
            $targets = Get-ReleaseRetentionCleanupTargets -Root $root -Keep 5 -NamePrefixes @('windows-')
            # Must be a countable array even when empty (not $null).
            @($targets).Count | Should Be 0
        } finally {
            Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}
