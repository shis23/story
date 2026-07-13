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
        '(?i)(api[_-]?key|secret|token|password|passwd)\s*[:=]\s*[''"]?[^\s''"]{12,}'
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
        [string[]]$Notes = @()
    )

    $safeTools = [ordered]@{}
    foreach ($key in ($ToolVersions.Keys | Sort-Object)) {
        $safeTools[[string]$key] = Protect-ReleasePath -Text ([string]$ToolVersions[$key]) -RepoRoot $null
    }

    return [pscustomobject]@{
        schema_version = 1
        generated_at_utc = (Get-Date).ToUniversalTime().ToString('o')
        commit         = $Commit
        branch         = $Branch
        target         = $Target
        tool_versions  = $safeTools
        artifacts      = @($Artifacts)
        build_status   = $BuildStatus
        warnings       = @($Warnings)
        notes          = @($Notes)
        acceptance     = [pscustomobject]@{
            gui            = 'not_claimed'
            android_device = 'not_claimed'
            host_build     = $BuildStatus
        }
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
        if ($normalized -match '(?i)sqlite') {
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
                $metaJson = & cargo metadata --format-version 1 --no-deps 2>$null
                if ($LASTEXITCODE -eq 0 -and $metaJson) {
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

function Get-ReleaseRetentionCleanupTargets {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Root,

        [Parameter(Mandatory = $true)]
        [int]$Keep
    )

    if ($Keep -lt 0) {
        throw 'Keep must be >= 0'
    }

    if (-not (Test-Path -LiteralPath $Root -PathType Container)) {
        return @()
    }

    $dirs = @(Get-ChildItem -LiteralPath $Root -Directory | Sort-Object LastWriteTimeUtc -Descending)
    if ($dirs.Count -le $Keep) {
        return @()
    }

    return @($dirs | Select-Object -Skip $Keep)
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

    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $dir = Join-Path $root ("{0}-{1}" -f $Prefix, $stamp)
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
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

    if ($pathFull.StartsWith($rootFull, [System.StringComparison]::OrdinalIgnoreCase)) {
        $rel = $pathFull.Substring($rootFull.Length).TrimStart('\', '/')
        return ($rel -replace '\\', '/')
    }

    return (Protect-ReleasePath -Text $pathFull -RepoRoot $RepoRoot)
}
