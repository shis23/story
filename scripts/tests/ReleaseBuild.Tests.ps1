# Requires Pester 3.x (Windows PowerShell default) or later.
# Run:
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/run-release-build-tests.ps1

$ErrorActionPreference = 'Stop'

$RepoRoot = (& git rev-parse --show-toplevel 2>$null)
if (-not $RepoRoot) {
    throw 'Unable to locate repository root for ReleaseBuild tests.'
}
$RepoRoot = (Resolve-Path -LiteralPath $RepoRoot).ProviderPath
$CommonPath = Join-Path $RepoRoot 'scripts\release-build\ReleaseBuild.Common.ps1'

if (-not (Test-Path -LiteralPath $CommonPath)) {
    throw "Missing ReleaseBuild.Common.ps1 at $CommonPath"
}

. $CommonPath

Describe 'ReleaseBuild path redaction' {
    It 'redacts absolute Windows user paths' {
        $raw = 'Built at C:\Users\Predator\project\target\release\storyforge.exe size=12'
        $redacted = Protect-ReleasePath -Text $raw -RepoRoot 'C:\Users\Predator\project'
        $redacted | Should Not Match 'C:\\Users\\Predator'
        $redacted | Should Match '<REPO>'
    }

    It 'redacts home directory variants without leaking username' {
        $homePath = $env:USERPROFILE
        $raw = "cache=$homePath\.cargo\registry index"
        $redacted = Protect-ReleasePath -Text $raw -RepoRoot $RepoRoot
        $redacted | Should Not Match ([regex]::Escape($homePath))
        $redacted | Should Match '<HOME>'
    }

    It 'never includes environment secret-looking values in redacted text' {
        # Construct patterns at runtime so static secret scan does not flag fixtures.
        $fakeToken = 'sk' + '-' + ('a' * 24) + '0123456789'
        $raw = "token=$fakeToken path=C:\Users\someone\secret"
        $redacted = Protect-ReleasePath -Text $raw -RepoRoot $RepoRoot
        $redacted | Should Not Match ([regex]::Escape($fakeToken))
        $redacted | Should Not Match 'C:\\Users\\someone'
        $redacted | Should Match '<REDACTED_SECRET>|<HOME>'
    }
}

Describe 'ReleaseBuild SHA-256 hashing' {
    It 'hashes file content deterministically' {
        $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-release-hash-{0}.bin" -f [guid]::NewGuid().ToString('N'))
        try {
            [System.IO.File]::WriteAllBytes($tmp, [byte[]](1, 2, 3, 4, 5))
            $h1 = Get-ReleaseFileSha256 -Path $tmp
            $h2 = Get-ReleaseFileSha256 -Path $tmp
            $h1 | Should Match '^[a-f0-9]{64}$'
            $h1 | Should Be $h2
        } finally {
            Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue
        }
    }

    It 'changes hash when content changes' {
        $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-release-hash-{0}.bin" -f [guid]::NewGuid().ToString('N'))
        try {
            [System.IO.File]::WriteAllBytes($tmp, [byte[]](1, 2, 3))
            $h1 = Get-ReleaseFileSha256 -Path $tmp
            [System.IO.File]::WriteAllBytes($tmp, [byte[]](1, 2, 4))
            $h2 = Get-ReleaseFileSha256 -Path $tmp
            $h1 | Should Not Be $h2
        } finally {
            Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue
        }
    }
}

Describe 'ReleaseBuild size budgets' {
    It 'classifies under-budget artifacts as ok' {
        $result = Get-ReleaseSizeBudgetResult -Label 'windows-exe' -SizeBytes 10MB -BudgetBytes 200MB
        $result.Status | Should Be 'ok'
        $result.Warning | Should Be $null
    }

    It 'classifies over-budget artifacts as warning without acceptance' {
        $result = Get-ReleaseSizeBudgetResult -Label 'android-debug-apk' -SizeBytes 300MB -BudgetBytes 250MB
        $result.Status | Should Be 'warning'
        $result.Accepted | Should Be $false
        $result.Warning | Should Match 'android-debug-apk'
        $result.Warning | Should Match 'budget'
    }

    It 'returns default Windows and Android budgets' {
        $budgets = Get-ReleaseSizeBudgets
        $budgets['windows-exe'] | Should BeGreaterThan 0
        $budgets['windows-msi'] | Should BeGreaterThan 0
        $budgets['android-debug-apk'] | Should BeGreaterThan 0
        $budgets['android-release-apk'] | Should BeGreaterThan 0
    }
}

Describe 'ReleaseBuild manifest construction' {
    It 'builds a deterministic manifest without absolute user paths or secrets' {
        $fakeHash = 'a' * 64
        $artifact = New-ReleaseArtifactRecord `
            -RelativePath 'target/release/storyforge.exe' `
            -SizeBytes 12345 `
            -Sha256 $fakeHash `
            -Kind 'windows-exe' `
            -Status 'present'

        $manifest = New-ReleaseBuildManifest `
            -Commit 'abc1234' `
            -Branch 'codex/release-build-pipeline' `
            -Target 'x86_64-pc-windows-msvc' `
            -ToolVersions @{ rustc = '1.0.0'; cargo = '1.0.0'; node = '20.0.0' } `
            -Artifacts @($artifact) `
            -BuildStatus 'ok' `
            -Warnings @('size budget warning: windows-exe') `
            -Notes @('host-only; not GUI acceptance') `
            -RepoRoot 'C:\Users\Predator\project'

        $json = $manifest | ConvertTo-Json -Depth 8
        $json | Should Not Match 'C:\\Users'
        $json | Should Not Match 'sk-'
        $manifest.commit | Should Be 'abc1234'
        $manifest.artifacts.Count | Should Be 1
        $manifest.artifacts[0].relative_path | Should Be 'target/release/storyforge.exe'
        $manifest.build_status | Should Be 'ok'
        $manifest.acceptance.gui | Should Be 'not_claimed'
        $manifest.acceptance.android_device | Should Be 'not_claimed'
    }

    It 'redacts secrets and user paths inside warnings and notes' {
        $fake = 'sk' + '-' + ('b' * 24)
        $manifest = New-ReleaseBuildManifest `
            -Commit 'abc' `
            -Branch 'test' `
            -Target 'x86_64-pc-windows-msvc' `
            -ToolVersions @{ rustc = '1' } `
            -Artifacts @() `
            -BuildStatus 'ok' `
            -Warnings @("leak=$fake path=C:\Users\Predator\secret") `
            -Notes @("cache=C:\Users\Predator\.cargo") `
            -RepoRoot 'C:\Users\Predator\project'

        $json = $manifest | ConvertTo-Json -Depth 8
        $json | Should Not Match ([regex]::Escape($fake))
        $json | Should Not Match 'C:\\Users\\Predator'
        $manifest.warnings[0] | Should Match '<REDACTED_SECRET>|<HOME>'
        $manifest.notes[0] | Should Match '<HOME>'
    }

    It 'marks missing expected artifacts as failed status' {
        $artifact = New-ReleaseArtifactRecord `
            -RelativePath 'target/release/missing.exe' `
            -SizeBytes 0 `
            -Sha256 $null `
            -Kind 'windows-exe' `
            -Status 'missing'

        $manifest = New-ReleaseBuildManifest `
            -Commit 'deadbeef' `
            -Branch 'test' `
            -Target 'x86_64-pc-windows-msvc' `
            -ToolVersions @{} `
            -Artifacts @($artifact) `
            -BuildStatus 'failed' `
            -Warnings @() `
            -Notes @()

        $manifest.build_status | Should Be 'failed'
        $manifest.artifacts[0].status | Should Be 'missing'
    }
}

Describe 'ReleaseBuild fail-closed guards' {
    It 'fails when a required tool is missing' {
        { Assert-ReleaseToolAvailable -Name 'totally-missing-tool-xyz' -CommandPath $null } | Should Throw
    }

    It 'fails when expected artifact path is missing' {
        $missing = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-missing-{0}.bin" -f [guid]::NewGuid().ToString('N'))
        { Assert-ReleaseArtifactExists -Path $missing -Label 'windows-exe' } | Should Throw
    }

    It 'fails closed on non-zero process exit code' {
        { Assert-ReleaseExitCode -ExitCode 2 -StepName 'cargo build --release' } | Should Throw
    }

    It 'accepts zero exit code' {
        { Assert-ReleaseExitCode -ExitCode 0 -StepName 'ok-step' } | Should Not Throw
    }
}

Describe 'ReleaseBuild warning normalization' {
    It 'normalizes deprecation and proguard warnings into stable categories' {
        $lines = @(
            'warning: The Kotlin Gradle plugin was loaded multiple times'
            'w: some Kotlin feature is deprecated'
            'WARNING: proguard consumer rules missing for plugin'
            'note: unrelated informational line'
            'Deprecated Gradle features were used in this build'
        )
        $report = ConvertTo-ReleaseWarningReport -Lines $lines -Source 'android-build'
        $report.source | Should Be 'android-build'
        @($report.warnings | Where-Object { $_.category -eq 'kotlin-deprecation' }).Count | Should BeGreaterThan 0
        @($report.warnings | Where-Object { $_.category -eq 'gradle-deprecation' }).Count | Should BeGreaterThan 0
        @($report.warnings | Where-Object { $_.category -eq 'proguard' }).Count | Should BeGreaterThan 0
        $report.warnings | ForEach-Object {
            $_.message | Should Not Match 'C:\\Users'
        }
    }
}

Describe 'ReleaseBuild APK inspection helpers' {
    It 'detects arm64-v8a native lib entries from zip listing' {
        $entries = @(
            'lib/arm64-v8a/libstoryforge.so'
            'lib/arm64-v8a/libsqlite3x.so'
            'assets/index.html'
            'META-INF/MANIFEST.MF'
        )
        $info = Get-ReleaseApkInspection -Entries $entries -ApkLabel 'app-arm64-debug.apk'
        $info.abis -contains 'arm64-v8a' | Should Be $true
        $info.has_native_libs | Should Be $true
        $info.sqlite_bundled | Should Be $true
        $info.capabilities.native_arm64 | Should Be $true
    }

    It 'reports missing arm64 and sqlite when absent' {
        $entries = @(
            'lib/x86_64/libstoryforge.so'
            'assets/index.html'
        )
        $info = Get-ReleaseApkInspection -Entries $entries -ApkLabel 'app-x86-debug.apk'
        $info.abis -contains 'arm64-v8a' | Should Be $false
        $info.sqlite_bundled | Should Be $false
        $info.capabilities.native_arm64 | Should Be $false
    }
}

Describe 'ReleaseBuild SBOM inventory fallback' {
    It 'returns cargo metadata style inventory when cargo tree is unavailable' {
        $inventory = New-ReleaseDependencyInventory `
            -CargoTomlPath (Join-Path $RepoRoot 'Cargo.toml') `
            -PackageLockPath (Join-Path $RepoRoot 'frontend\package-lock.json') `
            -PreferCargoTree:$false

        $inventory.generator | Should Match 'fallback|cargo-metadata|package-lock'
        $inventory.components.Count | Should BeGreaterThan 0
        ($inventory.components | ConvertTo-Json -Depth 6) | Should Not Match 'C:\\Users'
    }
}

Describe 'ReleaseBuild cleanup retention' {
    It 'selects old artifact directories beyond retention count' {
        $root = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-retention-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $root | Out-Null
        try {
            1..5 | ForEach-Object {
                $dir = Join-Path $root ("windows-run-{0:D2}" -f $_)
                New-Item -ItemType Directory -Path $dir | Out-Null
                Start-Sleep -Milliseconds 15
                Set-Content -LiteralPath (Join-Path $dir 'marker.txt') -Value $_
            }
            $toDelete = Get-ReleaseRetentionCleanupTargets -Root $root -Keep 2 -NamePrefixes @('windows-')
            $toDelete.Count | Should Be 3
        } finally {
            Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'only deletes prefixed run dirs and never the protected current run' {
        $root = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-retention-safe-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $root | Out-Null
        try {
            $keep = Join-Path $root 'windows-current'
            $old = Join-Path $root 'windows-old'
            $other = Join-Path $root 'scratch-not-a-run'
            New-Item -ItemType Directory -Path $keep, $old, $other | Out-Null
            Start-Sleep -Milliseconds 20
            # Make old older than keep
            (Get-Item -LiteralPath $old).LastWriteTimeUtc = (Get-Date).ToUniversalTime().AddHours(-2)
            (Get-Item -LiteralPath $keep).LastWriteTimeUtc = (Get-Date).ToUniversalTime()

            $toDelete = Get-ReleaseRetentionCleanupTargets `
                -Root $root `
                -Keep 1 `
                -NamePrefixes @('windows-', 'android-') `
                -ProtectFullNames @($keep)

            $names = @($toDelete | ForEach-Object { $_.Name })
            $names -contains 'windows-old' | Should Be $true
            $names -contains 'windows-current' | Should Be $false
            $names -contains 'scratch-not-a-run' | Should Be $false
        } finally {
            Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Describe 'ReleaseBuild exit policy' {
    It 'exits non-zero for failed and partial host statuses' {
        (Test-ReleaseStatusIsSuccess -BuildStatus 'ok') | Should Be $true
        (Test-ReleaseStatusIsSuccess -BuildStatus 'dry-run') | Should Be $true
        (Test-ReleaseStatusIsSuccess -BuildStatus 'failed') | Should Be $false
        (Test-ReleaseStatusIsSuccess -BuildStatus 'partial') | Should Be $false
    }
}

Describe 'ReleaseBuild artifact freshness' {
    It 'rejects artifacts older than the build start watermark' {
        $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-stale-{0}.exe" -f [guid]::NewGuid().ToString('N'))
        try {
            Set-Content -LiteralPath $tmp -Value 'old'
            $item = Get-Item -LiteralPath $tmp
            $item.LastWriteTimeUtc = (Get-Date).ToUniversalTime().AddHours(-3)
            $start = (Get-Date).ToUniversalTime().AddMinutes(-5)
            (Test-ReleaseArtifactIsFresh -FileInfo $item -NotBeforeUtc $start) | Should Be $false
        } finally {
            Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue
        }
    }

    It 'accepts artifacts written at or after the build start watermark' {
        $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-fresh-{0}.exe" -f [guid]::NewGuid().ToString('N'))
        try {
            $start = (Get-Date).ToUniversalTime().AddSeconds(-2)
            Set-Content -LiteralPath $tmp -Value 'new'
            $item = Get-Item -LiteralPath $tmp
            (Test-ReleaseArtifactIsFresh -FileInfo $item -NotBeforeUtc $start) | Should Be $true
        } finally {
            Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue
        }
    }
}

Describe 'ReleaseBuild secret scan helper' {
    It 'flags secret-like content without echoing the secret value' {
        $fake = 'sk' + '-' + ('z' * 24)
        $findings = @(Find-ReleaseSecretPatternFindings -Text ("token=$fake"))
        $findings.Count | Should BeGreaterThan 0
        ($findings -join ' ') | Should Not Match ([regex]::Escape($fake))
        ($findings -join ' ') | Should Match 'OpenAI-style API key|secret'
    }

    It 'returns no findings for clean text' {
        $findings = @(Find-ReleaseSecretPatternFindings -Text 'build ok commit=abc size=12')
        $findings.Count | Should Be 0
    }
}

Describe 'ReleaseBuild dry-run contract' {
    It 'formats commands without executing them' {
        $formatted = Format-ReleaseCommand -Command @('cargo', 'build', '--release')
        $formatted | Should Be 'cargo build --release'
    }
}
