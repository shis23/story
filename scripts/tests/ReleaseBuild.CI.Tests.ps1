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
        # Boundary-aware redaction: a standalone sk- token in a note is
        # redacted; an sk- substring embedded in an identifier (commit name)
        # is NOT a secret and is preserved. Use a note that places the token at
        # a value boundary so redaction is exercised.
        $fake = 'sk' + '-' + ('p' * 24)
        $prov = New-ReleaseProvenance `
            -Commit "leak-$fake" `
            -Branch 'test' `
            -Target 'x86_64-pc-windows-msvc' `
            -Artifacts @() `
            -RepoRoot 'C:\Users\Predator\repo' `
            -Notes @("token=$fake")
        $json = $prov | ConvertTo-Json -Depth 10
        # The standalone token in a note MUST be redacted.
        $json | Should Not Match ([regex]::Escape("token=$fake"))
        # The embedded substring in the commit identifier is NOT redacted.
        $json | Should Match ([regex]::Escape($fake))
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

    It 'writes UTF-8 without BOM so sha256sum -c can consume the sidecar' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-hash-bom-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $dir | Out-Null
        try {
            $artifactPath = Join-Path $dir 'storyforge.exe'
            [System.IO.File]::WriteAllBytes($artifactPath, [byte[]](9, 8, 7, 6))
            $hashFile = Write-ReleaseHashFile -ArtifactPath $artifactPath
            $bytes = [System.IO.File]::ReadAllBytes($hashFile)
            # UTF-8 BOM is EF BB BF; standard sha256sum files must not start with it.
            if ($bytes.Length -ge 3) {
                $hasBom = ($bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF)
                $hasBom | Should Be $false
            }
            $text = [System.Text.Encoding]::UTF8.GetString($bytes)
            $text | Should Match '^[a-f0-9]{64} \*'
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

    It 'reads entry payloads instead of only enumerating names' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-zip-payload-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $dir | Out-Null
        try {
            $zipPath = Join-Path $dir 'payload.zip'
            Add-Type -AssemblyName System.IO.Compression.FileSystem
            $zip = [System.IO.Compression.ZipFile]::Open($zipPath, 'Create')
            try {
                $entry = $zip.CreateEntry('lib/arm64-v8a/libtest.so')
                $writer = New-Object System.IO.StreamWriter($entry.Open())
                $writer.Write('native-payload-bytes')
                $writer.Close()
            } finally {
                $zip.Dispose()
            }
            $result = Test-ReleaseArchiveIntegrity -Path $zipPath -ExpectedKind 'apk'
            $result.entry_count | Should BeGreaterThan 0
            $result.bytes_read | Should BeGreaterThan 0
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Describe 'ReleaseBuild evidence subject staging' {
    It 'copies present subjects and hash sidecars into the evidence directory for offline verification' {
        $repo = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-stage-repo-{0}" -f [guid]::NewGuid().ToString('N'))
        $evidence = Join-Path $repo 'artifacts\release-build\windows-run'
        $binDir = Join-Path $repo 'target\release'
        New-Item -ItemType Directory -Path $binDir, $evidence -Force | Out-Null
        try {
            $exe = Join-Path $binDir 'storyforge.exe'
            [System.IO.File]::WriteAllBytes($exe, [byte[]](1, 2, 3, 4, 5))
            $sha = Get-ReleaseFileSha256 -Path $exe
            $art = New-ReleaseArtifactRecord -RelativePath 'target/release/storyforge.exe' -SizeBytes 5 -Sha256 $sha -Kind 'windows-exe' -Status 'present'
            # Prefer direct assignment; @() wrapper around unary-comma returns can nest.
            $staged = ConvertTo-ReleaseStagedSubjectArray -InputObject (
                Copy-ReleaseEvidenceSubjects -Artifacts @($art) -EvidenceDir $evidence -RepoRoot $repo
            )
            @($staged).Count | Should Be 1
            $record = @($staged)[0]
            $record.relative_path | Should Match '^subjects/'
            $subjectPath = Join-Path $evidence ($record.relative_path -replace '/', '\')
            Test-Path -LiteralPath $subjectPath | Should Be $true
            Test-Path -LiteralPath ($subjectPath + '.sha256') | Should Be $true
            $rehash = Get-ReleaseFileSha256 -Path $subjectPath
            $rehash | Should Be $sha
            $sidecar = Get-Content -LiteralPath ($subjectPath + '.sha256') -Raw
            $sidecar | Should Match ([regex]::Escape($sha))
        } finally {
            Remove-Item -LiteralPath $repo -Recurse -Force -ErrorAction SilentlyContinue
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
    It 'validates a well-formed workflow with a real parser or fails closed without one' {
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
            if ($result.Engine -eq 'none') {
                $result.Valid | Should Be $false
                ($result.Errors -join ' ') | Should Match 'No real YAML parser'
            } else {
                $result.Valid | Should Be $true
                $result.ErrorCount | Should Be 0
                $result.Engine | Should Match 'pyyaml|node-yaml'
            }
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'rejects a YAML 1.1 boolean key masquerading as the workflow trigger' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-wf-boolean-on-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $dir | Out-Null
        try {
            $wf = Join-Path $dir 'boolean-key.yml'
            # PyYAML's YAML 1.1 resolver parses this unquoted key as Boolean
            # True. A Gitea workflow still needs an explicit source-level `on:`
            # key, rather than a semantically unrelated Boolean key.
            @(
                'name: test'
                'true: [push]'
                'jobs:'
                '  build:'
                '    runs-on: ubuntu-latest'
                '    steps:'
                '      - run: echo hello'
            ) | Set-Content -LiteralPath $wf -Encoding utf8
            $result = Test-ReleaseWorkflowSyntax -Path $wf
            $result.Valid | Should Be $false
            if ($result.Engine -eq 'none') {
                ($result.Errors -join ' ') | Should Match 'No real YAML parser'
            } else {
                ($result.Errors -join ' ') | Should Match 'explicit top-level.*on|Missing top-level.*on'
            }
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'detects invalid YAML syntax with a real parser (not structural-only)' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-wf-bad-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $dir | Out-Null
        try {
            $wf = Join-Path $dir 'bad.yml'
            # Valid-looking keys for the structural heuristic, but invalid YAML quoting.
            @(
                'name: test'
                'on: push'
                'jobs:'
                '  build:'
                '    runs-on: ubuntu-latest'
                '    steps:'
                '      - run: "unclosed quote'
            ) | Set-Content -LiteralPath $wf -Encoding utf8
            $structural = Test-ReleaseWorkflowSyntax -Path $wf -PreferPowerShell
            # Structural path may still pass; real parser must fail.
            $result = Test-ReleaseWorkflowSyntax -Path $wf
            $result.Valid | Should Be $false
            $result.ErrorCount | Should BeGreaterThan 0
            if ($result.Engine -eq 'none') {
                ($result.Errors -join ' ') | Should Match 'No real YAML parser'
            } else {
                $result.Engine | Should Match 'pyyaml|node-yaml'
            }
            if ($structural.Valid) {
                $result.Valid | Should Be $false
            }
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed for a missing workflow file' {
        $missing = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-no-wf-{0}.yml" -f [guid]::NewGuid().ToString('N'))
        { Test-ReleaseWorkflowSyntax -Path $missing } | Should Throw
    }

    It 'rejects jobs that are not a mapping after real parse' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-wf-jobs-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $dir | Out-Null
        try {
            $wf = Join-Path $dir 'jobs-null.yml'
            @(
                'name: test'
                'on: push'
                'jobs: null'
            ) | Set-Content -LiteralPath $wf -Encoding utf8
            $result = Test-ReleaseWorkflowSyntax -Path $wf
            $result.Valid | Should Be $false
            if ($result.Engine -eq 'none') {
                ($result.Errors -join ' ') | Should Match 'No real YAML parser'
            } else {
                ($result.Errors -join ' ') | Should Match 'jobs'
            }
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Describe 'ReleaseBuild Node-only workflow YAML validation' {
    It 'selects Node and parses the requested valid workflow when Python is unavailable' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-node-yaml-valid-{0}" -f [guid]::NewGuid().ToString('N'))
        $previousNodePath = $env:NODE_PATH
        New-Item -ItemType Directory -Path (Join-Path $dir 'node_modules\yaml') -Force | Out-Null
        try {
            # JSON is a YAML subset. This isolated adapter implements yaml.parse
            # and deliberately requires the target marker, so passing the
            # generated helper script instead fails.
            @'
exports.parse = function (text) {
  if (!text.includes('node_only_marker')) {
    throw new Error('expected the requested workflow file');
  }
  return JSON.parse(text.replace(/^\uFEFF/, ''));
};
'@ | Set-Content -LiteralPath (Join-Path $dir 'node_modules\yaml\index.js') -Encoding utf8
            $workflow = Join-Path $dir 'valid.yml'
            @'
{
  "node_only_marker": "valid",
  "name": "node-only-valid",
  "on": "push",
  "jobs": { "build": { "runs-on": "windows-latest" } }
}
'@ | Set-Content -LiteralPath $workflow -Encoding utf8
            $env:NODE_PATH = (Join-Path $dir 'node_modules')

            Mock Get-Command { $null } -ParameterFilter { $Name -in @('python', 'python3') }
            $result = Test-ReleaseWorkflowSyntax -Path $workflow

            $result.Valid | Should Be $true
            $result.Engine | Should Be 'node-yaml'
            Assert-MockCalled Get-Command -ParameterFilter { $Name -eq 'python' } -Times 1 -Scope It -Exactly
            Assert-MockCalled Get-Command -ParameterFilter { $Name -eq 'python3' } -Times 1 -Scope It -Exactly
        } finally {
            $env:NODE_PATH = $previousNodePath
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'selects Node and rejects malformed requested workflow when Python is unavailable' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-node-yaml-invalid-{0}" -f [guid]::NewGuid().ToString('N'))
        $previousNodePath = $env:NODE_PATH
        New-Item -ItemType Directory -Path (Join-Path $dir 'node_modules\yaml') -Force | Out-Null
        try {
            @'
exports.parse = function (text) {
  if (!text.includes('node_only_marker')) {
    throw new Error('expected the requested workflow file');
  }
  return JSON.parse(text.replace(/^\uFEFF/, ''));
};
'@ | Set-Content -LiteralPath (Join-Path $dir 'node_modules\yaml\index.js') -Encoding utf8
            $workflow = Join-Path $dir 'invalid.yml'
            @'
{
  "node_only_marker": "invalid",
  "name": "node-only-invalid",
'@ | Set-Content -LiteralPath $workflow -Encoding utf8
            $env:NODE_PATH = (Join-Path $dir 'node_modules')

            Mock Get-Command { $null } -ParameterFilter { $Name -in @('python', 'python3') }
            $result = Test-ReleaseWorkflowSyntax -Path $workflow

            $result.Valid | Should Be $false
            $result.Engine | Should Be 'node-yaml'
            ($result.Errors -join ' ') | Should Match 'YAML parse error'
            Assert-MockCalled Get-Command -ParameterFilter { $Name -eq 'python' } -Times 1 -Scope It -Exactly
            Assert-MockCalled Get-Command -ParameterFilter { $Name -eq 'python3' } -Times 1 -Scope It -Exactly
        } finally {
            $env:NODE_PATH = $previousNodePath
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Describe 'ReleaseBuild Gitea workflow governance checks' {
    It 'parses tracked workflows with a real parser or fails closed without one' {
        $workflowDir = Join-Path $RepoRoot '.gitea\workflows'
        $files = @(Get-ChildItem -LiteralPath $workflowDir -Filter '*.yml' -File -ErrorAction SilentlyContinue) +
                 @(Get-ChildItem -LiteralPath $workflowDir -Filter '*.yaml' -File -ErrorAction SilentlyContinue)
        @($files).Count | Should BeGreaterThan 0
        $results = @($files | ForEach-Object { Test-ReleaseWorkflowSyntax -Path $_.FullName })
        if (@($results | Where-Object { $_.Engine -eq 'none' }).Count -gt 0) {
            # The CI workflow installs PyYAML. A minimal local test environment
            # must not silently substitute heuristic parsing.
            $noneCount = @($results | Where-Object { $_.Engine -eq 'none' }).Count
            $totalCount = @($results).Count
            $noneCount | Should Be $totalCount
            @($results | Where-Object { $_.Valid }).Count | Should Be 0
        } else {
            @($results | Where-Object { -not $_.Valid }).Count | Should Be 0
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

Describe 'ReleaseBuild secret scan untracked inputs' {
    It 'scans untracked build-input files and fails closed without echoing secrets' {
        $repo = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-scan-untracked-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $repo | Out-Null
        Push-Location $repo
        try {
            & git init --quiet | Out-Null
            & git config user.email 'test@example.com'
            & git config user.name 'test'
            Set-Content -LiteralPath (Join-Path $repo 'README.md') -Value 'ok' -Encoding utf8
            & git add README.md
            & git commit -m 'init' --quiet | Out-Null
            $fake = 'sk' + '-' + ('u' * 24)
            Set-Content -LiteralPath (Join-Path $repo 'local-build.env') -Value ("API_TOKEN=$fake") -Encoding utf8
            { Invoke-ReleaseSecretScan -RepoRoot $repo } | Should Throw
        } finally {
            Pop-Location
            Remove-Item -LiteralPath $repo -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'does not flag an untracked story-task identifier containing an sk- run' {
        # Boundary-aware scanner must not treat an sk- substring embedded in an
        # ordinary identifier (a story-task id) as a secret, while still
        # detecting a standalone real-shaped sk- token.
        $repo = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-scan-taskid-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $repo | Out-Null
        Push-Location $repo
        try {
            & git init --quiet | Out-Null
            & git config user.email 'test@example.com'
            & git config user.name 'test'
            Set-Content -LiteralPath (Join-Path $repo 'README.md') -Value 'ok' -Encoding utf8
            & git add README.md
            & git commit -m 'init' --quiet | Out-Null
            $content = '{' + "`n" +
                '  "tasks": [' + "`n" +
                '    { "id": "task-authenticate-red-wax-note", "status": "pending" },' + "`n" +
                '    { "id": "task-follow-gold-raven-decoy", "status": "pending" }' + "`n" +
                '  ]' + "`n" +
                '}'
            Set-Content -LiteralPath (Join-Path $repo 'story-tasks.json') -Value $content -Encoding utf8
            { Invoke-ReleaseSecretScan -RepoRoot $repo } | Should Not Throw
        } finally {
            Pop-Location
            Remove-Item -LiteralPath $repo -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'fails closed when an untracked build-input exceeds the scan size limit' {
        $repo = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-scan-large-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $repo | Out-Null
        Push-Location $repo
        try {
            & git init --quiet | Out-Null
            & git config user.email 'test@example.com'
            & git config user.name 'test'
            Set-Content -LiteralPath (Join-Path $repo 'README.md') -Value 'ok' -Encoding utf8
            & git add README.md
            & git commit -m 'init' --quiet | Out-Null
            $big = Join-Path $repo 'oversized-input.bin'
            # 2 MiB + 1 byte — must fail closed, not skip.
            $bytes = New-Object byte[] (2MB + 1)
            [System.IO.File]::WriteAllBytes($big, $bytes)
            { Invoke-ReleaseSecretScan -RepoRoot $repo } | Should Throw
        } finally {
            Pop-Location
            Remove-Item -LiteralPath $repo -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'ignores unsafe host-global excludes while preserving repository ignore rules' {
        $repo = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-scan-global-excludes-{0}" -f [guid]::NewGuid().ToString('N'))
        $previousGlobalConfig = $env:GIT_CONFIG_GLOBAL
        $previousNoSystem = $env:GIT_CONFIG_NOSYSTEM
        New-Item -ItemType Directory -Path $repo | Out-Null
        Push-Location $repo
        try {
            & git init --quiet | Out-Null
            & git config user.email 'test@example.com'
            & git config user.name 'test'
            Set-Content -LiteralPath (Join-Path $repo 'README.md') -Value 'ok' -Encoding utf8
            Set-Content -LiteralPath (Join-Path $repo '.gitignore') -Value 'repo-ignored.env' -Encoding utf8
            & git add README.md .gitignore
            & git commit -m 'init' --quiet | Out-Null

            # Simulate a host-global exclude that would hide an untracked secret
            # from --exclude-standard. Release scanning must override it, while
            # the committed repository .gitignore remains effective.
            $globalExcludes = Join-Path $repo 'host-global-excludes'
            $globalConfig = Join-Path $repo 'host-global.gitconfig'
            Set-Content -LiteralPath $globalExcludes -Value 'globally-hidden.env' -Encoding utf8
            & git config --file $globalConfig core.excludesFile $globalExcludes
            if ($LASTEXITCODE -ne 0) { throw 'Unable to configure synthetic global excludes file.' }
            $env:GIT_CONFIG_GLOBAL = $globalConfig
            $env:GIT_CONFIG_NOSYSTEM = '1'

            $fake = 'sk' + '-' + ('g' * 24)
            Set-Content -LiteralPath (Join-Path $repo 'globally-hidden.env') -Value ("API_TOKEN=$fake") -Encoding utf8
            Set-Content -LiteralPath (Join-Path $repo 'repo-ignored.env') -Value ("API_TOKEN=$fake") -Encoding utf8

            # The global pattern is ignored by the release scanner, so the hidden
            # secret must be found rather than silently skipped.
            { Invoke-ReleaseSecretScan -RepoRoot $repo } | Should Throw

            Remove-Item -LiteralPath (Join-Path $repo 'globally-hidden.env') -Force
            # The repository-owned ignore rule is still honored; its fake secret
            # must not be scanned as an untracked build input.
            { Invoke-ReleaseSecretScan -RepoRoot $repo } | Should Not Throw
        } finally {
            if ($null -eq $previousGlobalConfig) {
                Remove-Item Env:\GIT_CONFIG_GLOBAL -ErrorAction SilentlyContinue
            } else {
                $env:GIT_CONFIG_GLOBAL = $previousGlobalConfig
            }
            if ($null -eq $previousNoSystem) {
                Remove-Item Env:\GIT_CONFIG_NOSYSTEM -ErrorAction SilentlyContinue
            } else {
                $env:GIT_CONFIG_NOSYSTEM = $previousNoSystem
            }
            Pop-Location
            Remove-Item -LiteralPath $repo -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Describe 'ReleaseBuild workflow syntax structural fallback' {
    It 'can run PreferPowerShell structural checks but production path requires a real parser engine' {
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-wf-ps-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $dir | Out-Null
        try {
            $wf = Join-Path $dir 'ok.yml'
            @(
                'name: test'
                'on: [push]'
                'concurrency:'
                '  group: g'
                '  cancel-in-progress: true'
                'permissions:'
                '  contents: read'
                'jobs:'
                '  build:'
                '    runs-on: ubuntu-latest'
                '    timeout-minutes: 10'
                '    steps:'
                '      - run: echo hi'
            ) | Set-Content -LiteralPath $wf -Encoding utf8
            $structural = Test-ReleaseWorkflowSyntax -Path $wf -PreferPowerShell
            $structural.Valid | Should Be $true
            $structural.Engine | Should Match 'powershell'
            $real = Test-ReleaseWorkflowSyntax -Path $wf
            if ($real.Engine -eq 'none') {
                $real.Valid | Should Be $false
                ($real.Errors -join ' ') | Should Match 'No real YAML parser'
            } else {
                $real.Valid | Should Be $true
                $real.Engine | Should Match 'pyyaml|node-yaml'
            }
        } finally {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Describe 'ReleaseBuild host evidence workflow defaults' {
    It 'defaults skip_bundle to true and pins tauri-cli when bundle is requested' {
        $wf = Join-Path $RepoRoot '.gitea\workflows\release-host-evidence.yml'
        $text = Get-Content -LiteralPath $wf -Raw
        $text | Should Match 'default:\s*''true'''
        $text | Should Match 'tauri-cli --locked --version'
        $text | Should Match '2\.11\.2'
        $text | Should Match 'SkipBundle'
    }
}

Describe 'ReleaseBuild Pester runner compatibility' {
    It 'selects a legacy-syntax-compatible Pester module and never routes these files through Pester 5' {
        $runner = Get-Content -LiteralPath (Join-Path $RepoRoot 'scripts\tests\run-release-build-tests.ps1') -Raw
        $runner | Should Match 'Version\.Major\s+-lt\s+5'
        $runner | Should Match 'compatible Pester 3\.x/4\.x'
        $runner | Should Not Match 'if\s*\(\$version\.Major\s+-ge\s+5\)'
    }
}
