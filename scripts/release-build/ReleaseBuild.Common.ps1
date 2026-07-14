<#
.SYNOPSIS
Shared helpers for StoryForge release build and Android host pipelines.

.DESCRIPTION
Pure PowerShell helpers used by run-release-build.ps1 and unit tests.
Produces redacted paths, SHA-256 digests, size budgets, manifests, warning
reports, APK inspection summaries, dependency inventory fallbacks, and
retention cleanup selection. Never launches a GUI or touches devices.
#>

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

function Format-ReleaseCommand {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Command
    )

    return ($Command | ForEach-Object {
        if ($_ -match '\s') {
            '"' + ($_ -replace '"', '\"') + '"'
        } else {
            $_
        }
    }) -join ' '
}

function Protect-ReleasePath {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$Text,

        [Parameter(Mandatory = $false)]
        [string]$RepoRoot
    )

    if ([string]::IsNullOrEmpty($Text)) {
        return $Text
    }

    $result = $Text

    # Redact common secret-looking tokens first.
    $secretPatterns = @(
        'sk-[A-Za-z0-9_-]{20,}',
        'AKIA[0-9A-Z]{16}',
        'xox[baprs]-[0-9A-Za-z-]{10,}',
        '(?i)\bauthorization\s*:\s*(bearer|basic|token)?\s*[^\s''"`,;]{8,}',
        '(?i)\bbearer\s+[A-Za-z0-9._~+/=-]{8,}',
        '(?i)(api[_-]?key|secret|token|password|passwd|credential|authorization)\s*[:=]\s*[''"]?[^\s''"]{12,}'
    )
    foreach ($pattern in $secretPatterns) {
        $result = [regex]::Replace($result, $pattern, '<REDACTED_SECRET>')
    }

    if (-not [string]::IsNullOrWhiteSpace($RepoRoot)) {
        $repoVariants = @(
            $RepoRoot,
            ($RepoRoot -replace '\\', '/'),
            ($RepoRoot -replace '/', '\')
        ) | Select-Object -Unique

        foreach ($variant in $repoVariants) {
            if ([string]::IsNullOrWhiteSpace($variant)) { continue }
            $result = $result.Replace($variant, '<REPO>')
        }
    }

    $userHome = $env:USERPROFILE
    if (-not [string]::IsNullOrWhiteSpace($userHome)) {
        $homeVariants = @(
            $userHome,
            ($userHome -replace '\\', '/'),
            ($userHome -replace '/', '\')
        ) | Select-Object -Unique
        foreach ($variant in $homeVariants) {
            if ([string]::IsNullOrWhiteSpace($variant)) { continue }
            $result = $result.Replace($variant, '<HOME>')
        }
    }

    # Generic absolute Windows user path redaction.
    $result = [regex]::Replace($result, '(?i)[A-Z]:\\Users\\[^\\/\s"''`]+', '<HOME>')
    $result = [regex]::Replace($result, '(?i)/Users/[^/\s"''`]+', '<HOME>')
    $result = [regex]::Replace($result, '(?i)/home/[^/\s"''`]+', '<HOME>')

    return $result
}

function Protect-ReleaseObject {
    param(
        [AllowNull()]
        [object]$Value,

        [string]$RepoRoot
    )

    if ($null -eq $Value) { return $null }
    if ($Value -is [string]) {
        return (Protect-ReleasePath -Text ([string]$Value) -RepoRoot $RepoRoot)
    }
    if ($Value -is [System.Collections.IDictionary]) {
        $safe = [ordered]@{}
        foreach ($key in $Value.Keys) {
            $safe[[string]$key] = Protect-ReleaseObject -Value $Value[$key] -RepoRoot $RepoRoot
        }
        return $safe
    }
    if ($Value -is [pscustomobject]) {
        $safe = [ordered]@{}
        foreach ($property in $Value.PSObject.Properties) {
            $safe[$property.Name] = Protect-ReleaseObject -Value $property.Value -RepoRoot $RepoRoot
        }
        return [pscustomobject]$safe
    }
    if ($Value -is [System.Collections.IEnumerable]) {
        $items = @($Value | ForEach-Object { Protect-ReleaseObject -Value $_ -RepoRoot $RepoRoot })
        return ,$items
    }
    return $Value
}

function Get-ReleaseSafeErrorDetails {
    param(
        [Parameter(Mandatory = $true)]
        [object]$ErrorRecord,

        [string]$RepoRoot
    )

    $message = if ($ErrorRecord.PSObject.Properties.Name -contains 'Exception' -and
        $null -ne $ErrorRecord.Exception -and
        $ErrorRecord.Exception.PSObject.Properties.Name -contains 'Message') {
        [string]$ErrorRecord.Exception.Message
    } else {
        [string]$ErrorRecord
    }
    $stack = if ($ErrorRecord.PSObject.Properties.Name -contains 'ScriptStackTrace') {
        [string]$ErrorRecord.ScriptStackTrace
    } else { '' }
    $position = ''
    if ($ErrorRecord.PSObject.Properties.Name -contains 'InvocationInfo' -and
        $null -ne $ErrorRecord.InvocationInfo -and
        $ErrorRecord.InvocationInfo.PSObject.Properties.Name -contains 'PositionMessage') {
        $position = [string]$ErrorRecord.InvocationInfo.PositionMessage
    }

    return [pscustomobject]@{
        message = Protect-ReleasePath -Text $message -RepoRoot $RepoRoot
        stack = Protect-ReleasePath -Text $stack -RepoRoot $RepoRoot
        position = Protect-ReleasePath -Text $position -RepoRoot $RepoRoot
    }
}

function Get-ReleaseFileSha256 {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Cannot hash missing file: $Path"
    }

    $hash = Get-FileHash -LiteralPath $Path -Algorithm SHA256
    return $hash.Hash.ToLowerInvariant()
}

function Get-ReleaseSizeBudgets {
    # Soft budgets: overage produces warnings, never silent acceptance.
    return [ordered]@{
        'windows-exe'          = 80MB
        'windows-msi'          = 120MB
        'windows-nsis'         = 120MB
        'android-debug-apk'    = 280MB
        'android-release-apk'  = 60MB
    }
}

function Get-ReleaseSizeBudgetResult {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Label,

        [Parameter(Mandatory = $true)]
        [long]$SizeBytes,

        [Parameter(Mandatory = $true)]
        [long]$BudgetBytes
    )

    if ($SizeBytes -le $BudgetBytes) {
        return [pscustomobject]@{
            Label      = $Label
            SizeBytes  = $SizeBytes
            BudgetBytes = $BudgetBytes
            Status     = 'ok'
            Accepted   = $false
            Warning    = $null
        }
    }

    return [pscustomobject]@{
        Label       = $Label
        SizeBytes   = $SizeBytes
        BudgetBytes = $BudgetBytes
        Status      = 'warning'
        Accepted    = $false
        Warning     = ("size budget warning: {0} is {1} bytes, budget {2} bytes; not treated as acceptance" -f $Label, $SizeBytes, $BudgetBytes)
    }
}

function New-ReleaseArtifactRecord {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RelativePath,

        [Parameter(Mandatory = $true)]
        [long]$SizeBytes,

        [AllowNull()]
        [string]$Sha256,

        [Parameter(Mandatory = $true)]
        [string]$Kind,

        [Parameter(Mandatory = $true)]
        [ValidateSet('present', 'missing', 'skipped')]
        [string]$Status
    )

    return [pscustomobject]@{
        relative_path = $RelativePath -replace '\\', '/'
        size_bytes    = $SizeBytes
        sha256        = $Sha256
        kind          = $Kind
        status        = $Status
    }
}

function New-ReleaseBuildManifest {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Commit,

        [Parameter(Mandatory = $true)]
        [string]$Branch,

        [Parameter(Mandatory = $true)]
        [string]$Target,

        [Parameter(Mandatory = $true)]
        [System.Collections.IDictionary]$ToolVersions,

        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [AllowNull()]
        [object[]]$Artifacts = @(),

        [Parameter(Mandatory = $true)]
        [ValidateSet('ok', 'failed', 'partial', 'dry-run')]
        [string]$BuildStatus,

        [AllowEmptyCollection()]
        [string[]]$Warnings = @(),

        [AllowEmptyCollection()]
        [string[]]$Notes = @(),

        [AllowNull()]
        [object]$DependencyInventory = $null,

        [string]$RepoRoot
    )

    $safeTools = [ordered]@{}
    foreach ($key in ($ToolVersions.Keys | Sort-Object)) {
        $safeTools[[string]$key] = Protect-ReleasePath -Text ([string]$ToolVersions[$key]) -RepoRoot $RepoRoot
    }

    $safeWarnings = @($Warnings | ForEach-Object {
        Protect-ReleasePath -Text ([string]$_) -RepoRoot $RepoRoot
    })
    $safeNotes = @($Notes | ForEach-Object {
        Protect-ReleasePath -Text ([string]$_) -RepoRoot $RepoRoot
    })

    $manifest = [pscustomobject]@{
        schema_version = 1
        generated_at_utc = (Get-Date).ToUniversalTime().ToString('o')
        commit         = $Commit
        branch         = $Branch
        target         = $Target
        tool_versions  = $safeTools
        artifacts      = @($Artifacts)
        dependency_inventory = $DependencyInventory
        build_status   = $BuildStatus
        warnings       = $safeWarnings
        notes          = $safeNotes
        acceptance     = [pscustomobject]@{
            gui            = 'not_claimed'
            android_device = 'not_claimed'
            host_build     = $BuildStatus
        }
    }
    return (Protect-ReleaseObject -Value $manifest -RepoRoot $RepoRoot)
}

function Test-ReleaseStatusIsSuccess {
    param(
        [Parameter(Mandatory = $true)]
        [string]$BuildStatus
    )

    return ($BuildStatus -eq 'ok' -or $BuildStatus -eq 'dry-run')
}

function Get-ReleaseProcessExitCode {
    param(
        [Parameter(Mandatory = $true)]
        [string]$BuildStatus
    )

    if (Test-ReleaseStatusIsSuccess -BuildStatus $BuildStatus) {
        return 0
    }
    return 1
}

function Test-ReleaseArtifactIsFresh {
    param(
        [Parameter(Mandatory = $true)]
        [System.IO.FileInfo]$FileInfo,

        [Parameter(Mandatory = $true)]
        [datetime]$NotBeforeUtc
    )

    $writeUtc = $FileInfo.LastWriteTimeUtc
    # Allow small filesystem timestamp skew.
    return ($writeUtc -ge $NotBeforeUtc.AddSeconds(-2))
}

function Find-ReleaseSecretPatternFindings {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$Text
    )

    if ([string]::IsNullOrEmpty($Text)) {
        return @()
    }

    $rules = @(
        @{ Name = 'private key block'; Pattern = '-----BEGIN (RSA|DSA|EC|OPENSSH|PGP) PRIVATE KEY-----' },
        @{ Name = 'AWS access key id'; Pattern = 'AKIA[0-9A-Z]{16}' },
        @{ Name = 'OpenAI-style API key'; Pattern = 'sk-[A-Za-z0-9_-]{20,}' },
        @{ Name = 'Slack token'; Pattern = 'xox[baprs]-[0-9A-Za-z-]{10,}' },
        @{ Name = 'authorization header'; Pattern = '(?i)(Authorization|X-Api-Key)\s*:\s*(token|Bearer|Basic)?\s*[A-Za-z0-9_./+=-]{20,}' },
        @{ Name = 'secret assignment'; Pattern = '(?i)(api[_-]?key|secret|token|password|passwd|authorization)\s*[:=]\s*[''"][^''"]{16,}[''"]' }
    )

    $findings = @()
    foreach ($rule in $rules) {
        if ([regex]::IsMatch($Text, $rule.Pattern)) {
            $findings += ("secret-pattern:{0}" -f $rule.Name)
        }
    }
    return $findings
}

function Invoke-ReleaseSecretScan {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RepoRoot
    )

    $pathspecs = @(
        '.',
        ':(exclude)target/**',
        ':(exclude)node_modules/**',
        ':(exclude)frontend/dist/**',
        ':(exclude)frontend/node_modules/**',
        ':(exclude).git/**',
        ':(exclude)artifacts/**'
    )

    $rules = @(
        @{ Name = 'private key block'; Pattern = '-----BEGIN (RSA|DSA|EC|OPENSSH|PGP) PRIVATE KEY-----' },
        @{ Name = 'AWS access key id'; Pattern = 'AKIA[0-9A-Z]{16}' },
        @{ Name = 'OpenAI-style API key'; Pattern = 'sk-[A-Za-z0-9_-]{20,}' },
        @{ Name = 'Slack token'; Pattern = 'xox[baprs]-[0-9A-Za-z-]{10,}' },
        @{ Name = 'authorization header'; Pattern = '(Authorization|X-Api-Key)[[:space:]]*:[[:space:]]*(token|Bearer|Basic)?[[:space:]]*[A-Za-z0-9_./+=-]{20,}' },
        @{ Name = 'secret assignment'; Pattern = '(api[_-]?key|secret|token|password|passwd|authorization)[[:space:]]*[:=][[:space:]]*[''"][^''"]{16,}[''"]' }
    )

    $findings = New-Object System.Collections.Generic.List[string]
    $scanTargets = @(
        @{ Name = 'worktree'; Args = @() },
        @{ Name = 'index'; Args = @('--cached') }
    )

    foreach ($target in $scanTargets) {
        foreach ($rule in $rules) {
            $prevEap = $ErrorActionPreference
            $ErrorActionPreference = 'Continue'
            try {
                $output = & git -C $RepoRoot grep @($target.Args) -n -I -E -e $($rule.Pattern) -- @pathspecs 2>&1
                $exitCode = $LASTEXITCODE
            } finally {
                $ErrorActionPreference = $prevEap
            }

            if ($exitCode -eq 1) { continue }
            if ($exitCode -ne 0) {
                throw "Secret scan failed while running $($target.Name) rule '$($rule.Name)'."
            }

            foreach ($line in @($output)) {
                if ($line -match '^(.+?):([0-9]+):') {
                    $findings.Add(("{0} {1} at {2}:{3}" -f $target.Name, $rule.Name, $Matches[1], $Matches[2]))
                } else {
                    $findings.Add(("{0} {1} at unknown location" -f $target.Name, $rule.Name))
                }
            }
        }
    }

    # Also scan untracked build-input files that may participate in the build
    # but are not yet in the index/worktree tracked set. Fail closed: listing,
    # size, and read failures are errors, not silent skips.
    $excludeDirs = @('target', 'node_modules', 'frontend/dist', 'frontend/node_modules', '.git', 'artifacts')
    $maxUntrackedBytes = 2MB
    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        # A user-level core.excludesFile is outside the release input boundary.
        # Ignore it so an unreadable host-global ignore cannot masquerade as an
        # untracked path or make a clean repository scan fail. Repository-local
        # .gitignore/.git/info/exclude rules remain part of --exclude-standard.
        $untracked = @(& git -C $RepoRoot -c 'core.excludesFile=' ls-files --others --exclude-standard 2>&1)
        $utCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $prevEap
    }
    if ($utCode -ne 0) {
        throw ("Secret scan failed while listing untracked files (git ls-files exit {0})." -f $utCode)
    }
    foreach ($rel in $untracked) {
        if ([string]::IsNullOrWhiteSpace("$rel")) { continue }
        # git may emit non-path diagnostics when mixed with stderr; only scan path-like lines.
        $normalized = ("$rel" -replace '\\', '/')
        if ($normalized -match '^(fatal:|error:|warning:)') {
            throw ("Secret scan failed while listing untracked files: {0}" -f (Protect-ReleasePath -Text $normalized -RepoRoot $RepoRoot))
        }
        $skip = $false
        foreach ($ex in $excludeDirs) {
            if ($normalized -eq $ex -or $normalized.StartsWith("$ex/")) {
                $skip = $true
                break
            }
        }
        if ($skip) { continue }
        $full = Join-Path $RepoRoot ($normalized -replace '/', [System.IO.Path]::DirectorySeparatorChar)
        if (-not (Test-Path -LiteralPath $full -PathType Leaf)) {
            # Directories appear in ls-files only as files; ignore missing leaves that
            # disappeared between listing and open only if path is not a file.
            if (Test-Path -LiteralPath $full) { continue }
            throw ("Secret scan failed: untracked path vanished or is not a readable file: {0}" -f (Protect-ReleasePath -Text $normalized -RepoRoot $RepoRoot))
        }
        $item = Get-Item -LiteralPath $full -ErrorAction Stop
        if ($item.Length -gt $maxUntrackedBytes) {
            throw ("Secret scan failed: untracked build-input '{0}' is {1} bytes (limit {2}). Move it under an excluded path or reduce size so it can be scanned fail-closed." -f (Protect-ReleasePath -Text $normalized -RepoRoot $RepoRoot), $item.Length, $maxUntrackedBytes)
        }
        try {
            $text = [System.IO.File]::ReadAllText($full)
        } catch {
            throw ("Secret scan failed: cannot read untracked build-input '{0}'." -f (Protect-ReleasePath -Text $normalized -RepoRoot $RepoRoot))
        }
        $patternHits = @(Find-ReleaseSecretPatternFindings -Text $text)
        foreach ($hit in $patternHits) {
            # Report rule name + path only; never echo secret values.
            $ruleName = $hit -replace '^secret-pattern:', ''
            $safePath = Protect-ReleasePath -Text $normalized -RepoRoot $RepoRoot
            $findings.Add(("untracked {0} at {1}" -f $ruleName, $safePath))
        }
    }

    if ($findings.Count -gt 0) {
        Write-Host 'Potential secret material found:' -ForegroundColor Red
        $findings | Sort-Object -Unique | ForEach-Object { Write-Host ("  {0}" -f $_) }
        throw 'Secret scan failed. Remove the secret material or replace it with a safe reference before releasing.'
    }

    Write-Host 'OK: secret scan found no matches in Git-tracked or untracked build-input files.'
}

function Get-ReleaseAndroidBuildPathIssues {
    $issues = @()
    foreach ($name in @('ANDROID_HOME', 'NDK_HOME')) {
        $value = [Environment]::GetEnvironmentVariable($name)
        if ([string]::IsNullOrWhiteSpace($value)) {
            $issues += "$name is required for -BuildApk but is not set."
            continue
        }
        if (-not (Test-Path -LiteralPath $value -PathType Container)) {
            $issues += "$name is required for -BuildApk but does not point to an existing directory."
        }
    }
    return @($issues)
}

function Assert-ReleaseAndroidBuildEnvironment {
    $issues = @(Get-ReleaseAndroidBuildPathIssues)
    if (@($issues).Count -gt 0) {
        throw ("Android APK build environment is incomplete:{0}  {1}" -f [Environment]::NewLine, ($issues -join ([Environment]::NewLine + '  ')))
    }
}

function Assert-ReleaseToolAvailable {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,

        [AllowNull()]
        [string]$CommandPath
    )

    if ([string]::IsNullOrWhiteSpace($CommandPath)) {
        throw "Required tool '$Name' is missing."
    }

    if (-not (Test-Path -LiteralPath $CommandPath)) {
        # Allow bare command names resolved on PATH.
        $resolved = Get-Command -Name $CommandPath -ErrorAction SilentlyContinue
        if (-not $resolved) {
            throw "Required tool '$Name' is missing at '$CommandPath'."
        }
    }
}

function Assert-ReleaseArtifactExists {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [string]$Label
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Expected release artifact '$Label' is missing: $Path"
    }
}

function Assert-ReleaseExitCode {
    param(
        [Parameter(Mandatory = $true)]
        [int]$ExitCode,

        [Parameter(Mandatory = $true)]
        [string]$StepName
    )

    if ($ExitCode -ne 0) {
        throw "Step '$StepName' failed with exit code $ExitCode."
    }
}

function ConvertTo-ReleaseWarningReport {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [string[]]$Lines,

        [Parameter(Mandatory = $true)]
        [string]$Source,

        [string]$RepoRoot
    )

    $warningItems = @()

    foreach ($line in $Lines) {
        if ([string]::IsNullOrWhiteSpace($line)) { continue }

        $category = $null
        $lower = $line.ToLowerInvariant()

        if ($lower -match 'proguard') {
            $category = 'proguard'
        } elseif ($lower -match 'kotlin' -and ($lower -match 'deprecat' -or $lower -match 'warning')) {
            $category = 'kotlin-deprecation'
        } elseif ($lower -match 'gradle' -and ($lower -match 'deprecat' -or $lower -match 'deprecated')) {
            $category = 'gradle-deprecation'
        } elseif ($lower -match 'tauri' -and $lower -match 'deprecat') {
            $category = 'tauri-deprecation'
        } elseif ($lower -match 'deprecat') {
            $category = 'deprecation'
        } elseif ($lower -match 'warning') {
            $category = 'warning'
        } else {
            continue
        }

        $repoForRedact = if ($null -eq $RepoRoot) { '' } else { $RepoRoot }
        $message = Protect-ReleasePath -Text $line.Trim() -RepoRoot $repoForRedact
        $warningItems += [pscustomobject]@{
            category = $category
            message  = $message
        }
    }

    return [pscustomobject]@{
        source   = $Source
        count    = @($warningItems).Count
        warnings = @($warningItems)
    }
}

function Get-ReleaseApkInspection {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [string[]]$Entries,

        [Parameter(Mandatory = $true)]
        [string]$ApkLabel
    )

    $abis = New-Object System.Collections.Generic.List[string]
    $hasNative = $false
    $sqlite = $false

    foreach ($entry in $Entries) {
        $normalized = $entry -replace '\\', '/'
        if ($normalized -match '^lib/([^/]+)/') {
            $abi = $Matches[1]
            if (-not ($abis -contains $abi)) {
                $abis.Add($abi)
            }
            $hasNative = $true
        }
        if ($normalized -match '(?i)^lib/[^/]+/(libsqlite3|libsqlcipher)\.so$') {
            $sqlite = $true
        }
    }

    $abiArray = @($abis | Sort-Object)
    $hasArm64 = $abiArray -contains 'arm64-v8a'

    return [pscustomobject]@{
        label            = $ApkLabel
        abis             = $abiArray
        has_native_libs  = $hasNative
        sqlite_bundled   = $sqlite
        capabilities     = [pscustomobject]@{
            native_arm64 = $hasArm64
            sqlite       = $sqlite
        }
    }
}

function Assert-ReleaseApkInspection {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Inspection
    )

    if (-not $Inspection.has_native_libs) {
        throw "APK '$($Inspection.label)' contains no native libraries."
    }
    if (-not $Inspection.capabilities.native_arm64) {
        throw "APK '$($Inspection.label)' contains no arm64-v8a native library."
    }
}

function Assert-ReleaseRequiredApkKinds {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [object[]]$Artifacts
    )

    foreach ($kind in @('android-debug-apk', 'android-release-apk')) {
        $present = @($Artifacts | Where-Object {
            $_.kind -eq $kind -and $_.status -eq 'present'
        })
        if ($present.Count -eq 0) {
            throw "Required fresh APK evidence kind '$kind' is missing."
        }
    }
}

function New-ReleaseDependencyInventory {
    param(
        [Parameter(Mandatory = $true)]
        [string]$CargoTomlPath,

        [Parameter(Mandatory = $false)]
        [string]$PackageLockPath,

        [switch]$PreferCargoTree
    )

    $components = New-Object System.Collections.Generic.List[object]
    $generator = 'fallback'

    if ($PreferCargoTree) {
        $cargo = Get-Command cargo -ErrorAction SilentlyContinue
        if ($cargo) {
            try {
                $treeOut = & cargo tree --workspace --prefix none --depth 1 2>$null
                if ($LASTEXITCODE -eq 0 -and $treeOut) {
                    $generator = 'cargo-tree'
                    foreach ($line in $treeOut) {
                        if ($line -match '^(\S+)\s+v([0-9][^\s]*)') {
                            $components.Add([pscustomobject]@{
                                name    = $Matches[1]
                                version = $Matches[2]
                                source  = 'cargo-tree'
                            })
                        }
                    }
                }
            } catch {
                # Fall through to cargo metadata / lock fallback.
            }
        }
    }

    if ($components.Count -eq 0) {
        $cargo = Get-Command cargo -ErrorAction SilentlyContinue
        if ($cargo -and (Test-Path -LiteralPath $CargoTomlPath)) {
            try {
                # Include transitive packages for SBOM-style inventory (not just workspace members).
                $prevEap = $ErrorActionPreference
                $ErrorActionPreference = 'Continue'
                try {
                    $metaJson = & cargo metadata --format-version 1 --manifest-path $CargoTomlPath 2>$null
                    $metaCode = $LASTEXITCODE
                } finally {
                    $ErrorActionPreference = $prevEap
                }
                if ($metaCode -eq 0 -and $metaJson) {
                    $generator = 'cargo-metadata'
                    $meta = $metaJson | ConvertFrom-Json
                    foreach ($pkg in $meta.packages) {
                        $components.Add([pscustomobject]@{
                            name    = $pkg.name
                            version = $pkg.version
                            source  = 'cargo-metadata'
                            license = $(if ($pkg.PSObject.Properties.Name -contains 'license') { $pkg.license } else { $null })
                        })
                    }
                }
            } catch {
                # Fall through to Cargo.lock parse.
            }
        }
    }

    if ($components.Count -eq 0) {
        $lockPath = Join-Path (Split-Path -Parent $CargoTomlPath) 'Cargo.lock'
        if (Test-Path -LiteralPath $lockPath) {
            $generator = 'cargo-lock-fallback'
            $name = $null
            $version = $null
            Get-Content -LiteralPath $lockPath | ForEach-Object {
                if ($_ -match '^name\s*=\s*"([^"]+)"') {
                    $name = $Matches[1]
                } elseif ($_ -match '^version\s*=\s*"([^"]+)"') {
                    $version = $Matches[1]
                    if ($name) {
                        $components.Add([pscustomobject]@{
                            name    = $name
                            version = $version
                            source  = 'cargo-lock'
                        })
                        $name = $null
                        $version = $null
                    }
                } elseif ($_ -match '^\[\[') {
                    $name = $null
                    $version = $null
                }
            }
        }
    }

    if (-not [string]::IsNullOrWhiteSpace($PackageLockPath) -and (Test-Path -LiteralPath $PackageLockPath)) {
        try {
            $lock = Get-Content -LiteralPath $PackageLockPath -Raw | ConvertFrom-Json
            if ($lock.packages) {
                if ($generator -eq 'fallback') {
                    $generator = 'package-lock'
                } else {
                    $generator = "$generator+package-lock"
                }
                foreach ($prop in $lock.packages.PSObject.Properties) {
                    $pkgPath = $prop.Name
                    $pkg = $prop.Value
                    if ([string]::IsNullOrWhiteSpace($pkgPath)) { continue }
                    $pkgName = if ($pkgPath -match 'node_modules/(.+)$') { $Matches[1] } else { $pkgPath }
                    $pkgVersion = $null
                    if ($pkg.PSObject.Properties.Name -contains 'version') {
                        $pkgVersion = $pkg.version
                    }
                    if ($pkgName -and $pkgVersion) {
                        $components.Add([pscustomobject]@{
                            name    = $pkgName
                            version = $pkgVersion
                            source  = 'package-lock'
                        })
                    }
                }
            }
        } catch {
            # Keep cargo inventory if package-lock parse fails.
        }
    }

    if ($components.Count -eq 0) {
        $generator = 'fallback-empty'
        $components.Add([pscustomobject]@{
            name    = 'storyforge'
            version = 'unknown'
            source  = 'fallback'
            note    = 'No local SBOM tooling produced inventory; see Cargo.toml / package-lock.json manually.'
        })
    }

    # De-duplicate by name@version@source
    $unique = @{}
    foreach ($c in $components) {
        $key = '{0}@{1}@{2}' -f $c.name, $c.version, $c.source
        if (-not $unique.ContainsKey($key)) {
            $unique[$key] = $c
        }
    }

    return [pscustomobject]@{
        generator  = $generator
        generated_at_utc = (Get-Date).ToUniversalTime().ToString('o')
        components = @($unique.Values | Sort-Object name, version)
        notes      = @(
            'SBOM-style inventory for host release evidence only.',
            'Not a signed SPDX attestation.',
            'Absolute paths and secrets are excluded by construction.'
        )
    }
}

function Test-ReleasePathWithinRoot {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$Path
    )

    $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd('\', '/')
    $pathFull = [System.IO.Path]::GetFullPath($Path).TrimEnd('\', '/')
    if ($pathFull.Equals($rootFull, [System.StringComparison]::OrdinalIgnoreCase)) {
        return $true
    }
    $prefix = $rootFull + [System.IO.Path]::DirectorySeparatorChar
    return $pathFull.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)
}

function Assert-ReleaseDirectoryNotReparsePoint {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Label
    )

    $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    if (-not $item.PSIsContainer) {
        throw "$Label is not a directory: $Path"
    }
    if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "$Label must not be a junction, symlink, or reparse point: $Path"
    }
    return $item
}

function Get-ReleaseRetentionCleanupTargets {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Root,

        [Parameter(Mandatory = $true)]
        [int]$Keep,

        [string[]]$NamePrefixes = @('windows-', 'android-'),

        [string[]]$ProtectFullNames = @()
    )

    if ($Keep -lt 0) {
        throw 'Keep must be >= 0'
    }

    if (-not (Test-Path -LiteralPath $Root -PathType Container)) {
        return @()
    }

    $rootItem = Assert-ReleaseDirectoryNotReparsePoint -Path $Root -Label 'Retention root'
    $rootFull = [System.IO.Path]::GetFullPath($rootItem.FullName).TrimEnd('\', '/')

    $protected = @{}
    foreach ($p in @($ProtectFullNames)) {
        if ([string]::IsNullOrWhiteSpace($p)) { continue }
        try {
            $protected[[System.IO.Path]::GetFullPath($p)] = $true
        } catch {
            $protected[$p] = $true
        }
    }

    $candidates = @(Get-ChildItem -LiteralPath $rootFull -Directory -ErrorAction Stop | Where-Object {
        $name = $_.Name
        $matched = $false
        foreach ($prefix in @($NamePrefixes)) {
            if ($name.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
                $matched = $true
                break
            }
        }
        if (-not $matched) { return $false }

        # Skip reparse points / junctions / symlinks.
        if (($_.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
            return $false
        }
        return $true
    } | Sort-Object LastWriteTimeUtc -Descending)

    if ($candidates.Count -le $Keep) {
        return @()
    }

    # Keep the newest N matching runs; never delete the protected current run even if it falls outside Keep.
    $toDelete = @($candidates | Select-Object -Skip $Keep | Where-Object {
        -not $protected.ContainsKey($_.FullName)
    })
    return $toDelete
}

function Remove-ReleaseRetentionTargets {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$Targets
    )

    $rootItem = Assert-ReleaseDirectoryNotReparsePoint -Path $Root -Label 'Retention root'
    $rootFull = [System.IO.Path]::GetFullPath($rootItem.FullName).TrimEnd('\', '/')
    foreach ($target in @($Targets)) {
        $targetPath = if ($target -is [System.IO.FileSystemInfo]) { $target.FullName } else { [string]$target }
        if ([string]::IsNullOrWhiteSpace($targetPath)) {
            throw 'Retention target path is empty.'
        }
        $targetFull = [System.IO.Path]::GetFullPath($targetPath).TrimEnd('\', '/')
        if (-not (Test-ReleasePathWithinRoot -Root $rootFull -Path $targetFull)) {
            throw "Retention target escapes the trusted root: $targetFull"
        }
        if (-not ([System.IO.Path]::GetDirectoryName($targetFull)).Equals($rootFull, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Retention target must be a direct child of the trusted root: $targetFull"
        }
        $targetItem = Assert-ReleaseDirectoryNotReparsePoint -Path $targetFull -Label 'Retention target'
        Remove-Item -LiteralPath $targetItem.FullName -Recurse -Force -ErrorAction Stop
        if (Test-Path -LiteralPath $targetItem.FullName) {
            throw "Retention cleanup failed to remove target: $($targetItem.FullName)"
        }
    }
}

function Find-ReleaseRepoRoot {
    $start = (Get-Location).ProviderPath
    try {
        $gitRoot = (& git -C $start rev-parse --show-toplevel 2>$null)
        if ($LASTEXITCODE -eq 0 -and $gitRoot) {
            return (Resolve-Path -LiteralPath $gitRoot).ProviderPath
        }
    } catch {
        # Fall through.
    }

    $dir = Get-Item -LiteralPath $start
    while ($null -ne $dir) {
        $gitMarker = Join-Path $dir.FullName '.git'
        $cargoToml = Join-Path $dir.FullName 'Cargo.toml'
        if ((Test-Path -LiteralPath $gitMarker) -and (Test-Path -LiteralPath $cargoToml)) {
            return $dir.FullName
        }
        $dir = $dir.Parent
    }

    throw "Unable to locate the StoryForge repository root from '$start'."
}

function Get-ReleaseToolVersions {
    $versions = [ordered]@{}

    $pairs = @(
        @{ Name = 'rustc'; Command = @('rustc', '--version') },
        @{ Name = 'cargo'; Command = @('cargo', '--version') },
        @{ Name = 'node'; Command = @('node', '--version') },
        @{ Name = 'npm'; Command = @('npm.cmd', '--version') }
    )

    foreach ($pair in $pairs) {
        try {
            $cmd = Get-Command $pair.Command[0] -ErrorAction SilentlyContinue
            if (-not $cmd) {
                $versions[$pair.Name] = 'missing'
                continue
            }
            $out = & $pair.Command[0] $pair.Command[1..($pair.Command.Count - 1)] 2>$null
            if ($LASTEXITCODE -eq 0 -and $out) {
                $versions[$pair.Name] = (($out | Select-Object -First 1).ToString().Trim())
            } else {
                $versions[$pair.Name] = 'unknown'
            }
        } catch {
            $versions[$pair.Name] = 'error'
        }
    }

    return $versions
}

function Get-ReleaseGitIdentity {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RepoRoot
    )

    $commit = (& git -C $RepoRoot rev-parse --short HEAD 2>$null)
    if ($LASTEXITCODE -ne 0 -or -not $commit) {
        $commit = 'unknown'
    } else {
        $commit = $commit.Trim()
    }

    $branch = (& git -C $RepoRoot rev-parse --abbrev-ref HEAD 2>$null)
    if ($LASTEXITCODE -ne 0 -or -not $branch) {
        $branch = 'unknown'
    } else {
        $branch = $branch.Trim()
    }

    return [pscustomobject]@{
        commit = $commit
        branch = $branch
    }
}

function New-ReleaseRunDirectory {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RepoRoot,

        [string]$Prefix = 'run'
    )

    $root = Join-Path $RepoRoot 'artifacts\release-build'
    if (-not (Test-Path -LiteralPath $root)) {
        New-Item -ItemType Directory -Force -Path $root | Out-Null
    }

    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss-fff'
    $nonce = [guid]::NewGuid().ToString('N').Substring(0, 8)
    $dir = Join-Path $root ("{0}-{1}-{2}" -f $Prefix, $stamp, $nonce)
    New-Item -ItemType Directory -Path $dir -ErrorAction Stop | Out-Null
    return $dir
}

function Write-ReleaseJson {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Object,

        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $json = $Object | ConvertTo-Json -Depth 10
    $dir = Split-Path -Parent $Path
    if (-not (Test-Path -LiteralPath $dir)) {
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
    }
    Set-Content -LiteralPath $Path -Value $json -Encoding utf8
}

function Get-RelativeReleasePath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RepoRoot,

        [Parameter(Mandatory = $true)]
        [string]$FullPath
    )

    $rootFull = [System.IO.Path]::GetFullPath($RepoRoot).TrimEnd('\', '/')
    $pathFull = [System.IO.Path]::GetFullPath($FullPath)

    if (Test-ReleasePathWithinRoot -Root $rootFull -Path $pathFull) {
        $rel = $pathFull.Substring($rootFull.Length).TrimStart('\', '/')
        return ($rel -replace '\\', '/')
    }

    return '<EXTERNAL_PATH>'
}

function Get-ReleaseResultCount {
    param(
        [Parameter(Mandatory = $true)][object]$Result,
        [Parameter(Mandatory = $true)][string[]]$PropertyNames
    )

    foreach ($name in $PropertyNames) {
        if ($Result.PSObject.Properties.Name -contains $name) {
            $value = $Result.$name
            if ($null -eq $value) { continue }
            if ($value -is [System.Collections.IEnumerable] -and -not ($value -is [string])) {
                return @($value).Count
            }
            return [int]$value
        }
    }
    return 0
}

function Assert-ReleasePesterResult {
    param(
        [Parameter(Mandatory = $true)][object]$Result,
        [Parameter(Mandatory = $true)][string]$Label
    )

    if ($null -eq $Result) {
        throw "Pester returned no result for '$Label'."
    }
    $total = Get-ReleaseResultCount -Result $Result -PropertyNames @('TotalCount', 'Total')
    $failed = Get-ReleaseResultCount -Result $Result -PropertyNames @('FailedCount', 'Failed')
    $skipped = Get-ReleaseResultCount -Result $Result -PropertyNames @('SkippedCount', 'Skipped')
    $pending = Get-ReleaseResultCount -Result $Result -PropertyNames @('PendingCount', 'Pending')
    $inconclusive = Get-ReleaseResultCount -Result $Result -PropertyNames @('InconclusiveCount', 'Inconclusive')
    if ($total -le 0) {
        throw "Pester executed zero tests for '$Label'."
    }
    if ($failed -gt 0 -or $skipped -gt 0 -or $pending -gt 0 -or $inconclusive -gt 0) {
        throw "Pester result for '$Label' is not clean: total=$total failed=$failed skipped=$skipped pending=$pending inconclusive=$inconclusive."
    }
}

function New-ReleaseProvenance {
    <#
    .SYNOPSIS
    Builds an unsigned host provenance record for release evidence.

    .DESCRIPTION
    Creates a provenance-style attestation referencing artifacts by sha256
    digest and relative path. This is an unsigned host-only attestation, not a
    SLSA/cosign/in-toto signed attestation. All strings are sanitized.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$Commit,
        [Parameter(Mandatory = $true)][string]$Branch,
        [Parameter(Mandatory = $true)][string]$Target,
        [AllowEmptyCollection()][object[]]$Artifacts = @(),
        [AllowEmptyCollection()][string[]]$Notes = @(),
        [string]$RepoRoot
    )

    if ($null -eq $Artifacts) { $Artifacts = @() }
    $subjects = @()
    foreach ($art in $Artifacts) {
        if ($null -eq $art) { continue }
        $subjects += [pscustomobject]@{
            relative_path = Protect-ReleasePath -Text ([string]$art.relative_path) -RepoRoot $RepoRoot
            sha256        = [string]$art.sha256
            kind          = [string]$art.kind
            size_bytes    = [long]$art.size_bytes
            status        = [string]$art.status
        }
    }

    $baseNotes = @(
        'Unsigned host provenance attestation for release evidence only.',
        'This is not a SLSA, cosign, or in-toto signed attestation.',
        'No signing keys were used; subjects are identified by sha256 digest.'
    )
    $allNotes = @($baseNotes) + @($Notes | ForEach-Object {
        Protect-ReleasePath -Text ([string]$_) -RepoRoot $RepoRoot
    })

    $prov = [pscustomobject]@{
        schema_version   = 1
        generated_at_utc = (Get-Date).ToUniversalTime().ToString('o')
        commit           = $Commit
        branch           = $Branch
        target           = $Target
        subjects         = $subjects
        notes            = $allNotes
    }
    return (Protect-ReleaseObject -Value $prov -RepoRoot $RepoRoot)
}

function Write-ReleaseHashFile {
    <#
    .SYNOPSIS
    Writes a `<file>.sha256` sidecar digest in the standard SUM format.

    .DESCRIPTION
    Writes `<lowercased-sha256> *<basename>` to `<ArtifactPath>.sha256` so that
    `sha256sum -c` style verification tools can consume it. Uses UTF-8 without
    BOM. Fails closed when the artifact does not exist.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$ArtifactPath
    )

    if (-not (Test-Path -LiteralPath $ArtifactPath -PathType Leaf)) {
        throw "Cannot write hash file for missing artifact: $ArtifactPath"
    }

    $hash = Get-ReleaseFileSha256 -Path $ArtifactPath
    $basename = Split-Path -Leaf $ArtifactPath
    $content = "{0} *{1}" -f $hash, $basename
    $hashPath = $ArtifactPath + '.sha256'
    # UTF-8 without BOM: Windows PowerShell's Set-Content -Encoding utf8 emits a BOM,
    # which breaks sha256sum -c on Linux. Write raw UTF-8 bytes instead.
    $utf8NoBom = New-Object System.Text.UTF8Encoding $false
    [System.IO.File]::WriteAllText($hashPath, $content, $utf8NoBom)
    return $hashPath
}

function Test-ReleaseArchiveIntegrity {
    <#
    .SYNOPSIS
    Verifies that a zip-based archive (APK, etc.) can be opened and its payloads read.

    .DESCRIPTION
    Opens the archive with System.IO.Compression, enumerates entries, and reads
    each entry payload into a buffer to confirm the file is not truncated or
    corrupt. Fails closed on missing or unreadable files. Does not accept GUI
    or device evidence. Returns entry_count and bytes_read for evidence.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$ExpectedKind
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Archive integrity check failed: missing $ExpectedKind archive: $Path"
    }

    Add-Type -AssemblyName System.IO.Compression.FileSystem -ErrorAction SilentlyContinue
    try {
        $zip = [System.IO.Compression.ZipFile]::OpenRead($Path)
        $entryCount = 0
        $bytesRead = [long]0
        $buffer = New-Object byte[] 8192
        try {
            foreach ($entry in $zip.Entries) {
                $entryCount += 1
                # Directories have Length 0 and no payload stream of interest.
                if ($entry.Length -le 0 -and $entry.FullName.EndsWith('/')) {
                    continue
                }
                $stream = $null
                try {
                    $stream = $entry.Open()
                    while ($true) {
                        $n = $stream.Read($buffer, 0, $buffer.Length)
                        if ($n -le 0) { break }
                        $bytesRead += $n
                    }
                } finally {
                    if ($null -ne $stream) { $stream.Dispose() }
                }
            }
        } finally {
            $zip.Dispose()
        }
        if ($entryCount -le 0) {
            throw "Archive integrity check failed: $ExpectedKind archive has zero entries: $Path"
        }
        return [pscustomobject]@{
            entry_count = $entryCount
            bytes_read  = $bytesRead
            kind        = $ExpectedKind
            path        = $Path
        }
    } catch {
        $msg = $_.Exception.Message
        throw "Archive integrity check failed for $ExpectedKind ($Path): $msg"
    }
}

function Copy-ReleaseEvidenceSubjects {
    <#
    .SYNOPSIS
    Stages present artifact subjects and hash sidecars into an evidence directory.

    .DESCRIPTION
    Copies each present artifact into `<EvidenceDir>/subjects/<kind>/<basename>`
    and writes a matching `.sha256` sidecar next to it. Returns staged subject
    records with evidence-relative paths so provenance can reference offline-
    verifiable files inside the uploaded package. Fails closed on missing sources.
    #>
    param(
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$Artifacts,
        [Parameter(Mandatory = $true)][string]$EvidenceDir,
        [Parameter(Mandatory = $true)][string]$RepoRoot
    )

    if ($null -eq $Artifacts) { $Artifacts = @() }
    $subjectsRoot = Join-Path $EvidenceDir 'subjects'
    if (-not (Test-Path -LiteralPath $subjectsRoot)) {
        New-Item -ItemType Directory -Force -Path $subjectsRoot | Out-Null
    }

    $staged = New-Object System.Collections.Generic.List[object]
    foreach ($art in $Artifacts) {
        if ($null -eq $art) { continue }
        if ($art.status -ne 'present') { continue }
        if ([string]::IsNullOrWhiteSpace($art.relative_path)) {
            throw 'Cannot stage subject with empty relative_path.'
        }
        if ([string]::IsNullOrWhiteSpace($art.sha256)) {
            throw ("Cannot stage present subject without sha256: {0}" -f $art.relative_path)
        }

        $source = Join-Path $RepoRoot ($art.relative_path -replace '/', [System.IO.Path]::DirectorySeparatorChar)
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
            throw "Cannot stage missing subject source: $($art.relative_path)"
        }

        $kindDir = Join-Path $subjectsRoot ($art.kind -replace '[^A-Za-z0-9._-]', '_')
        if (-not (Test-Path -LiteralPath $kindDir)) {
            New-Item -ItemType Directory -Force -Path $kindDir | Out-Null
        }
        $basename = Split-Path -Leaf $source
        $dest = Join-Path $kindDir $basename
        Copy-Item -LiteralPath $source -Destination $dest -Force
        $hashFile = Write-ReleaseHashFile -ArtifactPath $dest
        $rehash = Get-ReleaseFileSha256 -Path $dest
        if ($rehash -ne $art.sha256) {
            throw ("Staged subject hash mismatch for {0}: expected {1}, got {2}" -f $art.relative_path, $art.sha256, $rehash)
        }

        $relEvidence = Get-RelativeReleasePath -RepoRoot $EvidenceDir -FullPath $dest
        $staged.Add([pscustomobject]@{
            relative_path = $relEvidence
            source_path   = $art.relative_path
            sha256        = $rehash
            kind          = $art.kind
            size_bytes    = [long](Get-Item -LiteralPath $dest).Length
            status        = 'present'
            hash_sidecar  = (Get-RelativeReleasePath -RepoRoot $EvidenceDir -FullPath $hashFile)
        }) | Out-Null
    }
    return ,$staged.ToArray()
}

function Assert-ReleaseManifestSchema {
    <#
    .SYNOPSIS
    Validates that a release manifest object conforms to the expected schema.

    .DESCRIPTION
    Checks required top-level fields, valid build_status values, valid
    acceptance scope (GUI and device must never be 'accepted'), and that
    present artifacts always carry a sha256 digest. Fails closed on violations.
    #>
    param(
        [Parameter(Mandatory = $true)][object]$Manifest
    )

    if ($null -eq $Manifest) {
        throw 'Manifest schema validation failed: manifest is null.'
    }

    $requiredFields = @(
        'schema_version', 'generated_at_utc', 'commit', 'branch', 'target',
        'tool_versions', 'artifacts', 'build_status', 'warnings', 'notes',
        'acceptance'
    )
    foreach ($field in $requiredFields) {
        if ($Manifest.PSObject.Properties.Name -notcontains $field) {
            throw "Manifest schema validation failed: missing required field '$field'."
        }
    }

    $validStatuses = @('ok', 'failed', 'partial', 'dry-run')
    if ($validStatuses -notcontains $Manifest.build_status) {
        throw ("Manifest schema validation failed: invalid build_status '{0}'." -f $Manifest.build_status)
    }

    $acc = $Manifest.acceptance
    if ($null -eq $acc) {
        throw 'Manifest schema validation failed: acceptance block is null.'
    }
    if ($acc.PSObject.Properties.Name -notcontains 'gui' -or
        $acc.PSObject.Properties.Name -notcontains 'android_device') {
        throw 'Manifest schema validation failed: acceptance block missing gui/android_device fields.'
    }
    if ($acc.gui -ne 'not_claimed') {
        throw ("Manifest schema validation failed: GUI acceptance is '{0}' but must be 'not_claimed'." -f $acc.gui)
    }
    if ($acc.android_device -ne 'not_claimed') {
        throw ("Manifest schema validation failed: android_device acceptance is '{0}' but must be 'not_claimed'." -f $acc.android_device)
    }

    foreach ($art in @($Manifest.artifacts)) {
        if ($null -eq $art) { continue }
        if ($art.PSObject.Properties.Name -notcontains 'relative_path' -or
            $art.PSObject.Properties.Name -notcontains 'sha256' -or
            $art.PSObject.Properties.Name -notcontains 'kind' -or
            $art.PSObject.Properties.Name -notcontains 'status') {
            throw 'Manifest schema validation failed: artifact record missing required fields.'
        }
        $validArtStatuses = @('present', 'missing', 'skipped')
        if ($validArtStatuses -notcontains $art.status) {
            throw ("Manifest schema validation failed: invalid artifact status '{0}'." -f $art.status)
        }
        if ($art.status -eq 'present' -and [string]::IsNullOrWhiteSpace($art.sha256)) {
            throw ("Manifest schema validation failed: present artifact '{0}' has no sha256 digest." -f $art.relative_path)
        }
    }
}

function Test-ReleaseWorkflowSyntaxPowerShell {
    <#
    .SYNOPSIS
    Pure-PowerShell structural validation of a Gitea/GitHub Actions workflow YAML.

    .DESCRIPTION
    Heuristic only — not a real YAML parser. Used solely as an emergency fallback
    when explicitly allowed. Production validation must use PyYAML or a Node YAML
    parser.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$Path
    )

    $errors = New-Object System.Collections.Generic.List[string]
    $text = Get-Content -LiteralPath $Path -Raw -ErrorAction Stop
    if ([string]::IsNullOrWhiteSpace($text)) {
        $errors.Add('Workflow file is empty')
        return [pscustomobject]@{
            Valid = $false; ErrorCount = $errors.Count; Errors = @($errors); Engine = 'powershell-structural'
        }
    }

    $openSquare = ([regex]::Matches($text, '\[')).Count
    $closeSquare = ([regex]::Matches($text, '\]')).Count
    if ($openSquare -ne $closeSquare) {
        $errors.Add('Unbalanced square brackets')
    }
    $openCurly = ([regex]::Matches($text, '\{')).Count
    $closeCurly = ([regex]::Matches($text, '\}')).Count
    if ($openCurly -ne $closeCurly) {
        $errors.Add('Unbalanced curly braces')
    }
    if ($text -notmatch '(?m)^jobs\s*:') {
        $errors.Add("Missing top-level 'jobs:' mapping")
    }
    if ($text -notmatch '(?m)^(on|name)\s*:') {
        $errors.Add("Missing top-level 'on:' or 'name:' key")
    }
    if ($text -match '(?m)^\t') {
        $errors.Add('Tab indentation is not allowed in YAML')
    }

    return [pscustomobject]@{
        Valid      = ($errors.Count -eq 0)
        ErrorCount = $errors.Count
        Errors     = @($errors)
        Engine     = 'powershell-structural'
    }
}

function Test-ReleaseWorkflowSyntaxWithPyYaml {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$PythonCommand
    )

    $pyScript = @'
import sys, json
try:
    import yaml
except ImportError:
    print(json.dumps({"valid": False, "error_count": 1, "errors": ["PyYAML is not installed"], "engine": "python-missing-pyyaml"}))
    sys.exit(0)

path = sys.argv[1]
errors = []
try:
    with open(path, "r", encoding="utf-8") as f:
        data = yaml.safe_load(f)
    if data is None:
        errors.append("Workflow file is empty or parsed to null")
    elif not isinstance(data, dict):
        errors.append("Workflow root must be a mapping/dict")
    else:
        jobs = data.get("jobs")
        if not isinstance(jobs, dict):
            errors.append("Top-level 'jobs' must be a mapping after YAML parse")
        if "on" not in data and "name" not in data:
            errors.append("Missing top-level 'on' or 'name' after YAML parse")
except yaml.YAMLError as exc:
    msg = str(exc).replace("\n", " ")
    errors.append("YAML parse error: " + msg[:500])
except Exception as exc:
    errors.append("Unexpected error: " + str(exc)[:500])

result = {"valid": len(errors) == 0, "error_count": len(errors), "errors": errors, "engine": "pyyaml"}
print(json.dumps(result))
'@

    $tmpPy = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-yaml-{0}.py" -f [guid]::NewGuid().ToString('N'))
    try {
        $utf8NoBom = New-Object System.Text.UTF8Encoding $false
        [System.IO.File]::WriteAllText($tmpPy, $pyScript, $utf8NoBom)
        $prevEap = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try {
            $rawOutput = & $PythonCommand $tmpPy $Path 2>&1
            $code = $LASTEXITCODE
        } finally {
            $ErrorActionPreference = $prevEap
        }
        if ($code -ne 0 -or [string]::IsNullOrWhiteSpace(($rawOutput | Out-String))) {
            return [pscustomobject]@{
                Valid = $false
                ErrorCount = 1
                Errors = @("python PyYAML validation exited $code or produced no output")
                Engine = 'pyyaml'
            }
        }
        $lastLine = @($rawOutput | Where-Object { "$_" -match '^\{' })[-1]
        if (-not $lastLine) {
            return [pscustomobject]@{
                Valid = $false
                ErrorCount = 1
                Errors = @('python PyYAML validation produced no JSON output')
                Engine = 'pyyaml'
            }
        }
        $parsed = $lastLine | ConvertFrom-Json
        return [pscustomobject]@{
            Valid      = [bool]$parsed.valid
            ErrorCount = [int]$parsed.error_count
            Errors     = @($parsed.errors)
            Engine     = [string]$parsed.engine
        }
    } finally {
        Remove-Item -LiteralPath $tmpPy -Force -ErrorAction SilentlyContinue
    }
}

function Test-ReleaseWorkflowSyntaxWithNodeYaml {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$NodeCommand
    )

    # Use Node's process.argv only; no shell interpolation of workflow content.
    $js = @'
const fs = require("fs");
// argv[1] is this temporary helper script; argv[2] is the workflow requested
// by the PowerShell caller (`node <tmpJs> <workflowPath>`).
const path = process.argv[2];
function fail(msg) {
  process.stdout.write(JSON.stringify({ valid: false, error_count: 1, errors: [msg], engine: "node-yaml" }));
  process.exit(0);
}
let yaml;
try {
  yaml = require("yaml");
} catch (_) {
  try {
    yaml = require("js-yaml");
  } catch (e2) {
    fail("Neither 'yaml' nor 'js-yaml' Node packages are installed");
  }
}
try {
  const text = fs.readFileSync(path, "utf8");
  const data = yaml.load ? yaml.load(text) : yaml.parse(text);
  const errors = [];
  if (data == null) errors.push("Workflow file is empty or parsed to null");
  else if (typeof data !== "object" || Array.isArray(data)) errors.push("Workflow root must be a mapping/dict");
  else {
    if (!data.jobs || typeof data.jobs !== "object" || Array.isArray(data.jobs)) {
      errors.push("Top-level 'jobs' must be a mapping after YAML parse");
    }
    if (!Object.prototype.hasOwnProperty.call(data, "on") && !Object.prototype.hasOwnProperty.call(data, "name")) {
      errors.push("Missing top-level 'on' or 'name' after YAML parse");
    }
  }
  process.stdout.write(JSON.stringify({ valid: errors.length === 0, error_count: errors.length, errors, engine: "node-yaml" }));
} catch (e) {
  const msg = String(e && e.message ? e.message : e).replace(/\n/g, " ").slice(0, 500);
  process.stdout.write(JSON.stringify({ valid: false, error_count: 1, errors: ["YAML parse error: " + msg], engine: "node-yaml" }));
}
'@

    $tmpJs = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-yaml-{0}.js" -f [guid]::NewGuid().ToString('N'))
    try {
        $utf8NoBom = New-Object System.Text.UTF8Encoding $false
        [System.IO.File]::WriteAllText($tmpJs, $js, $utf8NoBom)
        $prevEap = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try {
            $rawOutput = & $NodeCommand $tmpJs $Path 2>&1
            $code = $LASTEXITCODE
        } finally {
            $ErrorActionPreference = $prevEap
        }
        if ($code -ne 0 -or [string]::IsNullOrWhiteSpace(($rawOutput | Out-String))) {
            return [pscustomobject]@{
                Valid = $false
                ErrorCount = 1
                Errors = @("node YAML validation exited $code or produced no output")
                Engine = 'node-yaml'
            }
        }
        $lastLine = @($rawOutput | Where-Object { "$_" -match '^\{' })[-1]
        if (-not $lastLine) {
            return [pscustomobject]@{
                Valid = $false
                ErrorCount = 1
                Errors = @('node YAML validation produced no JSON output')
                Engine = 'node-yaml'
            }
        }
        $parsed = $lastLine | ConvertFrom-Json
        return [pscustomobject]@{
            Valid      = [bool]$parsed.valid
            ErrorCount = [int]$parsed.error_count
            Errors     = @($parsed.errors)
            Engine     = [string]$parsed.engine
        }
    } finally {
        Remove-Item -LiteralPath $tmpJs -Force -ErrorAction SilentlyContinue
    }
}

function Test-ReleaseWorkflowSyntax {
    <#
    .SYNOPSIS
    Validates that a workflow YAML file parses without syntax errors.

    .DESCRIPTION
    Uses a real YAML parser by default (PyYAML via python, or Node yaml/js-yaml).
    Fails closed when no real parser is available unless -AllowStructuralFallback
    is set. -PreferPowerShell forces the heuristic path (tests only).
    #>
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [switch]$PreferPowerShell,
        [switch]$AllowStructuralFallback
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Workflow file not found: $Path"
    }

    if ($PreferPowerShell) {
        return (Test-ReleaseWorkflowSyntaxPowerShell -Path $Path)
    }

    $pythonCmd = Get-Command python -ErrorAction SilentlyContinue
    if (-not $pythonCmd) {
        $pythonCmd = Get-Command python3 -ErrorAction SilentlyContinue
    }
    if ($pythonCmd) {
        $pyResult = Test-ReleaseWorkflowSyntaxWithPyYaml -Path $Path -PythonCommand $pythonCmd.Source
        if ($pyResult.Engine -ne 'python-missing-pyyaml') {
            return $pyResult
        }
    }

    $nodeCmd = Get-Command node -ErrorAction SilentlyContinue
    if ($nodeCmd) {
        $nodeResult = Test-ReleaseWorkflowSyntaxWithNodeYaml -Path $Path -NodeCommand $nodeCmd.Source
        if ($nodeResult.Errors -notcontains "Neither 'yaml' nor 'js-yaml' Node packages are installed") {
            return $nodeResult
        }
    }

    if ($AllowStructuralFallback) {
        return (Test-ReleaseWorkflowSyntaxPowerShell -Path $Path)
    }

    return [pscustomobject]@{
        Valid      = $false
        ErrorCount = 1
        Errors     = @('No real YAML parser available (need python+PyYAML or node with yaml/js-yaml). Structural-only checks are not accepted for production validation.')
        Engine     = 'none'
    }
}
