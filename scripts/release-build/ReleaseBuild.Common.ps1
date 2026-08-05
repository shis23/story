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
    # Boundary-aware: `sk-` must start at a string boundary or follow a
    # non-word, non-hyphen character so substrings of ordinary identifiers
    # (e.g. story-task id "task-authenticate-red-wax-note") are not matched.
    $secretPatterns = @(
        '(?<![\w-])sk-[A-Za-z0-9_-]{20,}(?![\w-])',
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

function New-ReleaseStagedSubjectRecord {
    <#
    .SYNOPSIS
    Builds a single offline-verifiable staged subject record for evidence packages.

    .DESCRIPTION
    Separates the source tree artifact path (source_relative_path) from the
    evidence-package staged path (relative_path under subjects/). Offline
    verification only opens relative_path inside EvidenceDir.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$StagedRelativePath,
        [Parameter(Mandatory = $true)][string]$SourceRelativePath,
        [Parameter(Mandatory = $true)][long]$SizeBytes,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Sha256,
        [Parameter(Mandatory = $true)][string]$Kind,
        [Parameter(Mandatory = $true)][ValidateSet('present', 'missing', 'skipped')][string]$Status,
        [string]$HashSidecar
    )

    $staged = ($StagedRelativePath -replace '\\', '/').Trim().TrimStart('./')
    while ($staged.Contains('//')) { $staged = $staged -replace '//', '/' }
    $source = ($SourceRelativePath -replace '\\', '/').Trim().TrimStart('./')
    while ($source.Contains('//')) { $source = $source -replace '//', '/' }
    $sidecar = if ([string]::IsNullOrWhiteSpace($HashSidecar)) {
        $staged + '.sha256'
    } else {
        ($HashSidecar -replace '\\', '/').Trim().TrimStart('./')
    }

    return [pscustomobject]@{
        relative_path        = $staged
        source_relative_path = $source
        size_bytes           = [long]$SizeBytes
        sha256               = $Sha256
        kind                 = $Kind
        status               = $Status
        hash_sidecar         = $sidecar
    }
}

function Test-ReleaseStagedSubjectPath {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$RelativePath)
    $rel = ([string]$RelativePath -replace '\\', '/').Trim().TrimStart('./')
    while ($rel.Contains('//')) { $rel = $rel -replace '//', '/' }
    return [bool]($rel -match '^(?i)subjects/')
}

function ConvertTo-ReleaseStagedSubjectArray {
    <#
    .SYNOPSIS
    Flattens runner-shaped staged subject results into a flat object[] of records.

    .DESCRIPTION
    Copy-ReleaseEvidenceSubjects returns a unary-comma object[]. Wrapping that
    return with @(...) can nest arrays. This helper unwraps nested arrays and
    yields a flat object[] of staged subject records for manifest/provenance.
    #>
    param(
        [AllowNull()][object]$InputObject
    )

    $flat = New-Object System.Collections.Generic.List[object]
    function Add-Flattened {
        param([AllowNull()][object]$Node)
        if ($null -eq $Node) { return }
        if ($Node -is [string] -or $Node -is [ValueType]) { return }
        # Treat objects that already look like staged records as leaves.
        if ($Node -is [pscustomobject] -or $Node -is [System.Management.Automation.PSObject]) {
            $names = @($Node.PSObject.Properties | ForEach-Object { $_.Name })
            if ($names -contains 'relative_path' -or $names -contains 'source_relative_path' -or $names -contains 'sha256') {
                $flat.Add($Node) | Out-Null
                return
            }
        }
        if ($Node -is [System.Collections.IEnumerable] -and -not ($Node -is [string])) {
            foreach ($item in @($Node)) {
                Add-Flattened -Node $item
            }
            return
        }
    }
    Add-Flattened -Node $InputObject
    # Unary comma preserves empty and single-element object[] across function returns.
    return ,([object[]]@($flat.ToArray()))
}

function Test-ReleaseSha256SumLine {
    <#
    .SYNOPSIS
    Validates a single standard sha256sum line: "<64-hex> *basename".
    #>
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text,
        [string]$ExpectedBasename
    )

    if ([string]::IsNullOrWhiteSpace($Text)) {
        return [pscustomobject]@{ Valid = $false; Hash = $null; Basename = $null; Reason = 'empty' }
    }
    # Exactly one non-empty line (allow a single trailing newline after TrimEnd of outer reader).
    $normalized = $Text -replace "`r`n", "`n" -replace "`r", "`n"
    if ($normalized.Contains("`n")) {
        $parts = @($normalized -split "`n" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
        if ($parts.Count -ne 1) {
            return [pscustomobject]@{ Valid = $false; Hash = $null; Basename = $null; Reason = 'multi-line' }
        }
        $normalized = $parts[0]
    }
    $normalized = $normalized.Trim()
    if ($normalized -notmatch '^(?i)([0-9a-f]{64}) \*(.+)$') {
        return [pscustomobject]@{ Valid = $false; Hash = $null; Basename = $null; Reason = 'grammar' }
    }
    $hash = $Matches[1].ToLowerInvariant()
    $base = $Matches[2]
    if ($base -match '[\\/]' -or $base -match '\s') {
        return [pscustomobject]@{ Valid = $false; Hash = $hash; Basename = $base; Reason = 'basename' }
    }
    if (-not [string]::IsNullOrWhiteSpace($ExpectedBasename) -and $base -ne $ExpectedBasename) {
        return [pscustomobject]@{ Valid = $false; Hash = $hash; Basename = $base; Reason = 'basename-mismatch' }
    }
    return [pscustomobject]@{ Valid = $true; Hash = $hash; Basename = $base; Reason = 'ok' }
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

        [AllowEmptyCollection()]
        [AllowNull()]
        [object[]]$StagedSubjects = @(),

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

    $flatStaged = ConvertTo-ReleaseStagedSubjectArray -InputObject $StagedSubjects

    $manifest = [pscustomobject]@{
        schema_version = 1
        generated_at_utc = (Get-Date).ToUniversalTime().ToString('o')
        commit         = $Commit
        branch         = $Branch
        target         = $Target
        tool_versions  = $safeTools
        artifacts      = @($Artifacts)
        # Offline-verifiable staged subjects (subjects/...). Source tree paths stay on artifacts.
        staged_subjects = $flatStaged
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
        # Boundary-aware: a real key starts at a string boundary or after a
        # non-word, non-hyphen separator (space, quote, =, :, {, comma, ...).
        # Substrings of ordinary identifiers such as "task-authenticate-red-wax-note"
        # or "task-follow-gold-raven-decoy" must not match.
        @{ Name = 'OpenAI-style API key'; Pattern = '(?<![\w-])sk-[A-Za-z0-9_-]{20,}(?![\w-])' },
        # Short OpenAI-style keys (12-19 chars after "sk-", proxy relays
        # issuing truncated keys); must contain an uppercase letter to
        # discriminate real keys from lowercase test values. Gate 8 review
        # P1-2. Kept in sync with Invoke-ReleaseSecretScan.
        @{ Name = 'OpenAI-style API key (short)'; Pattern = '(?<![\w-])sk-(?=[A-Za-z0-9_-]{12,19}(?![\w-]))(?=[A-Za-z0-9_-]*[A-Z])[A-Za-z0-9_-]{12,19}' },
        @{ Name = 'Slack token'; Pattern = 'xox[baprs]-[0-9A-Za-z-]{10,}' },
        @{ Name = 'authorization header'; Pattern = '(?i)(Authorization|X-Api-Key)\s*:\s*(token|Bearer|Basic)?\s*[A-Za-z0-9_./+=-]{20,}' },
        # Bare Bearer tokens without an Authorization: prefix.
        @{ Name = 'bare bearer token'; Pattern = '(?i)\bBearer\s+[A-Za-z0-9._~+/=-]{16,}' },
        # Quoted secret assignment: real credential bound to a key. Excludes
        # word-substring keys, Rust struct-literal conversions
        # (`secret: "...".into()`), angle-bracket placeholders, English-phrase
        # values, and sentinel/placeholder markers (incl. NOT-REAL test
        # fixtures and storyforge-secret:v1: SecretRefs) so test fixtures/docs
        # are not flagged. Kept in sync with Invoke-ReleaseSecretScan.
        @{ Name = 'secret assignment'; Pattern = '(?i)(?<![A-Za-z0-9_])(api[_-]?key|secret|token|password|passwd|authorization|credential)\s*[:=]\s*[''"](?!(?:<[^>]+>|[^''"]*\s[^''"]*\s|[^''"]{0,60}(?:secret_should|should_be_|placeholder|\bexample\b|normalized|_test_|dummy|redacted|changeme|xxxxx|sf_secret_|earlyfact|legacy-|embed-|process-only|from-shell|not[_-]?real|storyforge-secret:v1:|\bsecret[a-z_-]*(?:token|key|value|string|word|phrase)\b|\balso-secret\b|\b(?:fake|test|sample|dummy)-token\b)))(?=[^''"]{16,}[''"])(?:[^''"]{16,})[''"](?!\s*\.(?:into|to_string|to_owned|as_str)\s*\()' },
        # Unquoted api-key / token / credential assignments or colon forms.
        # Key must be a standalone word; excludes Rust struct-literal
        # conversions so fixtures like `api_key: value.into()` are not flagged,
        # and StoryForge SecretRef values (the "-secret:" substring of
        # "storyforge-secret:v1:…" must not read as a `secret:` assignment —
        # its value part starts with "v1:"; real keys never do).
        @{ Name = 'unquoted secret assignment'; Pattern = '(?i)(?<![A-Za-z0-9_])(api[_-]?key|token|password|passwd|secret|credential)\s*[:=]\s*(?!v1:)[^\s''"]{16,}(?!\s*\.(?:into|to_string|to_owned|as_str)\s*\()' }
    )

    $findings = @()
    foreach ($rule in $rules) {
        if ([regex]::IsMatch($Text, $rule.Pattern)) {
            $findings += ("secret-pattern:{0}" -f $rule.Name)
        }
    }
    return $findings
}

function Get-ReleaseObjectStringLeaves {
    <#
    .SYNOPSIS
    Recursively collects string leaves and object keys from JSON-like objects for secret scanning.

    .DESCRIPTION
    Walks values and property/dictionary keys. Depth is bounded; callers must treat
    DepthExceeded=$true as fail-closed (scanner trust boundary overflow).
    #>
    param(
        [AllowNull()][object]$Value,
        [int]$Depth = 0,
        [int]$MaxDepth = 32
    )

    $leaves = New-Object System.Collections.Generic.List[string]
    $script:ReleaseObjectLeafDepthExceeded = $false
    function Walk-ObjectLeaves {
        param([AllowNull()][object]$Node, [int]$Level)
        if ($null -eq $Node) { return }
        if ($Level -gt $MaxDepth) {
            $script:ReleaseObjectLeafDepthExceeded = $true
            return
        }
        if ($Node -is [string]) {
            $leaves.Add([string]$Node) | Out-Null
            return
        }
        # Skip non-string scalars (bool/int/long/decimal/datetime).
        if ($Node -is [ValueType]) { return }
        if ($Node -is [System.Collections.IDictionary]) {
            foreach ($key in @($Node.Keys)) {
                $keyText = [string]$key
                $leaves.Add($keyText) | Out-Null
                $val = $Node[$key]
                if ($val -is [string] -or $val -is [ValueType]) {
                    # Also scan key=value / key: value forms so secret-shaped keys are caught.
                    $leaves.Add(('{0}={1}' -f $keyText, $val)) | Out-Null
                    $leaves.Add(('{0}: {1}' -f $keyText, $val)) | Out-Null
                }
                Walk-ObjectLeaves -Node $val -Level ($Level + 1)
            }
            return
        }
        if ($Node -is [pscustomobject]) {
            foreach ($property in @($Node.PSObject.Properties)) {
                $keyText = [string]$property.Name
                $leaves.Add($keyText) | Out-Null
                $val = $property.Value
                if ($val -is [string] -or $val -is [ValueType]) {
                    $leaves.Add(('{0}={1}' -f $keyText, $val)) | Out-Null
                    $leaves.Add(('{0}: {1}' -f $keyText, $val)) | Out-Null
                }
                Walk-ObjectLeaves -Node $val -Level ($Level + 1)
            }
            return
        }
        if ($Node -is [System.Collections.IEnumerable]) {
            foreach ($item in @($Node)) {
                Walk-ObjectLeaves -Node $item -Level ($Level + 1)
            }
        }
    }
    Walk-ObjectLeaves -Node $Value -Level $Depth
    return [pscustomobject]@{
        Leaves = [string[]]@($leaves.ToArray())
        DepthExceeded = [bool]$script:ReleaseObjectLeafDepthExceeded
        MaxDepth = [int]$MaxDepth
    }
}

function Test-ReleaseGitCommitSha {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Commit)
    # Strict git SHA: 7-40 lowercase/uppercase hex (short or full object id).
    return [bool]([regex]::IsMatch([string]$Commit, '^[0-9a-fA-F]{7,40}$'))
}

function Get-ReleaseEvidenceSubjectBindingKey {
    param([Parameter(Mandatory = $true)][object]$Entry)

    $rel = ([string]$Entry.relative_path) -replace '\\', '/'
    $rel = $rel.Trim().TrimStart('./')
    while ($rel.Contains('//')) { $rel = $rel -replace '//', '/' }
    $kind = ([string]$Entry.kind).Trim().ToLowerInvariant()
    $status = ([string]$Entry.status).Trim().ToLowerInvariant()
    $sha = ([string]$Entry.sha256).Trim().ToLowerInvariant()
    if ($Entry.PSObject.Properties.Name -notcontains 'size_bytes' -or $null -eq $Entry.size_bytes) {
        throw 'size_bytes is required for exact-set subject binding.'
    }
    $size = [long]$Entry.size_bytes
    if ($size -lt 0) {
        throw 'size_bytes must be non-negative for exact-set subject binding.'
    }
    return ('{0}|{1}|{2}|{3}|{4}' -f $rel.ToLowerInvariant(), $kind, $status, $sha, $size)
}

function Get-ReleaseEvidenceVerifierTrustModel {
    <#
    .SYNOPSIS
    Declares the offline verifier TOCTOU / reparse trust model.
    #>
    return [pscustomobject]@{
        model = 'open-then-hash with reparse rejection (TOCTOU-aware, not a sealed snapshot handle)'
        notes = @(
            'Verifier rejects junction/symlink/reparse points on evidence roots, subjects, sidecars, inventory, and ancestor path segments before hashing.',
            'Hashing uses open-then-hash of the validated path; this is not a single durable OS file handle snapshot across the whole package.',
            'TOCTOU race trust model: the package directory is assumed immutable for the duration of verification; concurrent writers are out of trust boundary.',
            'Canonical path checks bound subjects inside EvidenceDir after reparse rejection to reduce path-escape races.',
            'Operators must treat mutable or shared evidence directories as untrusted; copy to a private immutable tree before verification when in doubt.'
        )
    }
}

function Invoke-ReleaseSecretScan {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RepoRoot,
        # 额外扫描根（如 evidence 目录，repo 外）。只扫文本文件，跳过
        # SQLite/图片/构建产物等二进制；规则与 repo 侧一致（Gate 8 审查
        # P1-2：evidence 曾藏真实 key 且不在扫描域）。
        [string[]]$EvidenceRoots = @()
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
        # Boundary-aware OpenAI-style key: must start at a string boundary or
        # after a non-word, non-hyphen separator so substrings of ordinary
        # identifiers (e.g. "task-authenticate-red-wax-note") are not matched.
        @{ Name = 'OpenAI-style API key'; Pattern = '(?<![\w-])sk-[A-Za-z0-9_-]{20,}(?![\w-])' },
        # Short OpenAI-style keys (12-19 chars after "sk-", e.g. proxy relays
        # issuing truncated keys). Must contain an uppercase letter to
        # discriminate real high-entropy keys from lowercase test values
        # (sk-super-secret / sk-smth-1234). Gate 8 review P1-2: the real
        # proxy key (sk-BX…, 16 chars) is invisible to the {20,} rule.
        @{ Name = 'OpenAI-style API key (short)'; Pattern = '(?<![\w-])sk-(?=[A-Za-z0-9_-]{12,19}(?![\w-]))(?=[A-Za-z0-9_-]*[A-Z])[A-Za-z0-9_-]{12,19}' },
        @{ Name = 'Slack token'; Pattern = 'xox[baprs]-[0-9A-Za-z-]{10,}' },
        @{ Name = 'authorization header'; Pattern = '(Authorization|X-Api-Key)\s*:\s*(token|Bearer|Basic)?\s*[A-Za-z0-9_./+=-]{20,}' },
        # Secret assignment: a real credential bound to a key. Exclusions keep
        # test fixtures/docs from being flagged WITHOUT weakening real-key
        # detection:
        #   (1) key must be a standalone word (word boundary before it) so a
        #       substring like `Token=` inside manifest content or `myToken=`
        #       does not match;
        #   (2) Rust struct-literal conversions
        #       (`secret: "...".into()/.to_string()/.to_owned()/.as_str()`) are
        #       test fixtures, never config-file secrets;
        #   (3) the quoted value must not be an obvious placeholder: angle-
        #       bracketed `<...>`, a human-readable phrase containing spaces,
        #       or carrying sentinel markers (secret_should, should_be_,
        #       placeholder, example, normalized, _test_, dummy, redacted,
        #       changeme, xxxx, sf_secret_, earlyfact, legacy-, embed-,
        #       process-only, from-shell, NOT-REAL, storyforge-secret:v1:
        #       SecretRef). Real keys are high-entropy opaque strings and
        #       never carry these markers or read like English.
        @{ Name = 'secret assignment'; Pattern = '(?i)(?<![A-Za-z0-9_])(api[_-]?key|secret|token|password|passwd|authorization|credential)\s*[:=]\s*[''"](?!(?:<[^>]+>|[^''"]*\s[^''"]*\s|[^''"]{0,60}(?:secret_should|should_be_|placeholder|\bexample\b|normalized|_test_|dummy|redacted|changeme|xxxxx|sf_secret_|earlyfact|legacy-|embed-|process-only|from-shell|not[_-]?real|storyforge-secret:v1:|\bsecret[a-z_-]*(?:token|key|value|string|word|phrase)\b|\balso-secret\b|\b(?:fake|test|sample|dummy)-token\b)))(?=[^''"]{16,}[''"])(?:[^''"]{16,})[''"](?!\s*\.(?:into|to_string|to_owned|as_str)\s*\()' }
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
                # PCRE2 (-P) is required for the boundary-aware lookbehind/lookahead
                # on the OpenAI-style key pattern. git 2.53 compiles in PCRE2.
                $output = & git -C $RepoRoot grep @($target.Args) -n -I -P -e $($rule.Pattern) -- @pathspecs 2>&1
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

    # Gate 8 审查 P1-2: evidence 目录（repo 外）不在 git grep 域，且真实代理
    # key 长度低于 {20,} 阈值。对每个 evidence root 做文本文件扫描，规则与
    # repo 侧一致（Find-ReleaseSecretPatternFindings）。二进制产物（SQLite、
    # WAL、图片、构建产物、超大日志）跳过——它们不是可读文本输入。
    $evidenceExcludeExts = @('.sqlite3', '-wal', '-shm', '.png', '.jpg', '.jpeg', '.gif', '.webp', '.apk', '.so', '.dll', '.exe', '.zip', '.gz', '.pdf', '.woff', '.woff2', '.ico', '.bin', '.mp4')
    $evidenceExcludeDirSegments = @('card-shell-cache')
    $evidenceMaxBytes = 10MB
    foreach ($evidenceRoot in $EvidenceRoots) {
        if (-not (Test-Path -LiteralPath $evidenceRoot -PathType Container)) {
            throw ("Secret scan failed: evidence root does not exist: {0}" -f $evidenceRoot)
        }
        $evidenceFiles = @(Get-ChildItem -LiteralPath $evidenceRoot -Recurse -File -ErrorAction Stop)
        foreach ($file in $evidenceFiles) {
            $nameLower = $file.Name.ToLowerInvariant()
            $skip = $false
            foreach ($ex in $evidenceExcludeExts) {
                if ($nameLower.EndsWith($ex)) { $skip = $true; break }
            }
            if ($skip) { continue }
            # 第三方 webview 缓存目录（jquery/esm/index.html 等）：非用户输入，
            # unquoted 规则会对其库代码误报。
            $relSegments = $file.FullName.Substring($evidenceRoot.Length).Split([System.IO.Path]::DirectorySeparatorChar)
            foreach ($seg in $relSegments) {
                if ($evidenceExcludeDirSegments -contains $seg) { $skip = $true; break }
            }
            if ($skip) { continue }
            if ($file.Length -gt $evidenceMaxBytes) { continue }
            try {
                $text = [System.IO.File]::ReadAllText($file.FullName)
            } catch {
                # 不可读/二进制内容：跳过，不作为扫描输入。
                continue
            }
            $patternHits = @(Find-ReleaseSecretPatternFindings -Text $text)
            foreach ($hit in $patternHits) {
                # Report rule name + path only; never echo secret values.
                $ruleName = $hit -replace '^secret-pattern:', ''
                $findings.Add(("evidence {0} at {1}" -f $ruleName, $file.FullName))
            }
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

function Assert-ReleasePathNotReparsePoint {
    <#
    .SYNOPSIS
    Rejects files or directories that are junctions, symlinks, or other reparse points.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Label
    )

    if (-not (Test-Path -LiteralPath $Path)) {
        throw "$Label path is missing."
    }
    $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw ("{0} must not be a junction, symlink, or reparse point." -f $Label)
    }
    return $item
}

function Test-ReleaseEvidencePathSafe {
    <#
    .SYNOPSIS
    Ensures a subject/sidecar/inventory path stays inside EvidenceDir without reparse escapes.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$EvidenceRoot,
        [Parameter(Mandatory = $true)][string]$RelativePath,
        [Parameter(Mandatory = $true)][string]$Label
    )

    $rel = [string]$RelativePath
    if ([string]::IsNullOrWhiteSpace($rel)) {
        throw "$Label relative_path is empty."
    }
    if ($rel -match '(^|/|\\)\.\.(/|\\|$)' -or $rel.StartsWith('/') -or $rel -match '^[A-Za-z]:') {
        throw ("{0} path escape rejected." -f $Label)
    }

    $candidate = Join-Path $EvidenceRoot ($rel -replace '/', [System.IO.Path]::DirectorySeparatorChar)
    if (-not (Test-ReleasePathWithinRoot -Root $EvidenceRoot -Path $candidate)) {
        throw ("{0} path is outside the evidence package." -f $Label)
    }

    # Walk every ancestor under EvidenceRoot and reject reparse points so a
    # junctioned subjects/ tree cannot smuggle external content.
    $rootFull = [System.IO.Path]::GetFullPath($EvidenceRoot).TrimEnd('\', '/')
    $full = [System.IO.Path]::GetFullPath($candidate)
    $cursor = $full
    while ($true) {
        if (-not (Test-Path -LiteralPath $cursor)) { break }
        $null = Assert-ReleasePathNotReparsePoint -Path $cursor -Label $Label
        if ($cursor.Equals($rootFull, [System.StringComparison]::OrdinalIgnoreCase)) { break }
        $parent = Split-Path -Parent $cursor
        if ([string]::IsNullOrWhiteSpace($parent) -or $parent -eq $cursor) { break }
        if (-not (Test-ReleasePathWithinRoot -Root $EvidenceRoot -Path $parent)) { break }
        $cursor = $parent
    }

    # Canonical path must still resolve inside the evidence root.
    if (Test-Path -LiteralPath $candidate) {
        $resolved = (Resolve-Path -LiteralPath $candidate).ProviderPath
        if (-not (Test-ReleasePathWithinRoot -Root $EvidenceRoot -Path $resolved)) {
            throw ("{0} canonical path escapes the evidence package." -f $Label)
        }
    }

    return $candidate
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
        $stagedSize = [long](Get-Item -LiteralPath $dest).Length
        $hashSidecarRel = Get-RelativeReleasePath -RepoRoot $EvidenceDir -FullPath $hashFile
        $staged.Add((New-ReleaseStagedSubjectRecord `
            -StagedRelativePath $relEvidence `
            -SourceRelativePath ([string]$art.relative_path) `
            -SizeBytes $stagedSize `
            -Sha256 $rehash `
            -Kind ([string]$art.kind) `
            -Status 'present' `
            -HashSidecar $hashSidecarRel)) | Out-Null
    }
    # Flatten and return object[]. ConvertTo already uses unary-comma return.
    return (ConvertTo-ReleaseStagedSubjectArray -InputObject $staged.ToArray())
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
    if ($text -notmatch '(?m)^(?:\uFEFF)?(?:on|["'']on["''])\s*:') {
        $errors.Add("Missing explicit top-level 'on:' key")
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
import sys, json, re
try:
    import yaml
except ImportError:
    print(json.dumps({"valid": False, "error_count": 1, "errors": ["PyYAML is not installed"], "engine": "python-missing-pyyaml"}))
    sys.exit(0)

path = sys.argv[1]
errors = []
class UniqueKeySafeLoader(yaml.SafeLoader):
    pass
def construct_unique_mapping(loader, node, deep=False):
    mapping = {}
    for key_node, value_node in node.value:
        if key_node.tag == "tag:yaml.org,2002:merge" or key_node.value == "<<":
            raise yaml.constructor.ConstructorError(
                "while constructing a mapping", node.start_mark,
                "YAML merge keys are not allowed in workflow files", key_node.start_mark
            )
        key = loader.construct_object(key_node, deep=deep)
        if key in mapping:
            raise yaml.constructor.ConstructorError(
                "while constructing a mapping", node.start_mark,
                "found duplicate key: {0}".format(key), key_node.start_mark
            )
        mapping[key] = loader.construct_object(value_node, deep=deep)
    return mapping
UniqueKeySafeLoader.add_constructor(
    yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG,
    construct_unique_mapping
)
try:
    with open(path, "r", encoding="utf-8") as f:
        source = f.read()
    def has_explicit_root_on_key(text):
        root_indent = None
        for raw_line in text.splitlines():
            line = raw_line.lstrip("\ufeff")
            stripped = line.strip()
            if not stripped or stripped.startswith("#") or stripped in ("---", "{", "}"):
                continue
            key_match = re.match(r"^([ \t]*)(?:\"[^\"\r\n]+\"|'[^'\r\n]+'|[A-Za-z_][A-Za-z0-9_-]*)[ \t]*:", line)
            if not key_match:
                continue
            if root_indent is None:
                root_indent = key_match.group(1)
            on_match = re.match(r"^([ \t]*)(?:on|[\"']on[\"'])[ \t]*:", line)
            if on_match and root_indent == on_match.group(1):
                return True
        return False
    # PyYAML parses YAML 1.1's unquoted `on` as Boolean True. Require the
    # source-level trigger spelling so a `true:` mapping key cannot impersonate
    # a runnable Gitea workflow.
    has_explicit_on_key = has_explicit_root_on_key(source)
    if re.search(r"(?m)^[ \t]*<<[ \t]*:", source):
        raise ValueError("YAML merge keys are not allowed in workflow files")
    if re.search(r"(?:^|[ \t,:\[{])(?:&|\*)[A-Za-z0-9_-]+", source):
        raise ValueError("YAML anchors and aliases are not allowed in workflow files")
    data = yaml.load(source, Loader=UniqueKeySafeLoader)
    if data is None:
        errors.append("Workflow file is empty or parsed to null")
    elif not isinstance(data, dict):
        errors.append("Workflow root must be a mapping/dict")
    else:
        jobs = data.get("jobs")
        if not isinstance(jobs, dict):
            errors.append("Top-level 'jobs' must be a mapping after YAML parse")
        if not has_explicit_on_key:
            errors.append("Missing explicit top-level 'on:' workflow trigger key")
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
function hasExplicitRootOnKey(text) {
  let rootIndent = null;
  for (let line of text.split(/\r?\n/)) {
    line = line.replace(/^\uFEFF/, "");
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#") || trimmed === "---" || trimmed === "{" || trimmed === "}") continue;
    const key = /^([ \t]*)(?:"[^"\r\n]+"|'[^'\r\n]+'|[A-Za-z_][A-Za-z0-9_-]*)[ \t]*:/.exec(line);
    if (!key) continue;
    if (rootIndent === null) rootIndent = key[1];
    const on = /^([ \t]*)(?:on|["']on["'])[ \t]*:/.exec(line);
    if (on && rootIndent === on[1]) return true;
  }
  return false;
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
  // A parser may treat YAML 1.1 `on` as Boolean true. The workflow source must
  // still spell the root trigger key explicitly; `true:` is not a trigger.
  const hasExplicitOnKey = hasExplicitRootOnKey(text);
  if (/(^|\n)[ \t]*<<[ \t]*:/.test(text)) throw new Error("YAML merge keys are not allowed in workflow files");
  if (/(^|[ \t,:\[{])(?:&|\*)[A-Za-z0-9_-]+/.test(text)) throw new Error("YAML anchors and aliases are not allowed in workflow files");
  const data = yaml.load ? yaml.load(text) : yaml.parse(text);
  const errors = [];
  if (data == null) errors.push("Workflow file is empty or parsed to null");
  else if (typeof data !== "object" || Array.isArray(data)) errors.push("Workflow root must be a mapping/dict");
  else {
    if (!data.jobs || typeof data.jobs !== "object" || Array.isArray(data.jobs)) {
      errors.push("Top-level 'jobs' must be a mapping after YAML parse");
    }
    if (!hasExplicitOnKey) {
      errors.push("Missing explicit top-level 'on:' workflow trigger key");
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

function Get-ReleaseRunnerToolPresence {
    <#
    .SYNOPSIS
    Discovers local tools required by release runners and workflow syntax gates.
    #>
    param(
        [hashtable]$Overrides
    )

    $presence = [ordered]@{
        cargo         = $false
        rustc         = $false
        'npm.cmd'     = $false
        node          = $false
        python        = $false
        pyyaml        = $false
        'cargo-tauri' = $false
    }

    if ($null -ne $Overrides) {
        foreach ($key in $Overrides.Keys) {
            $presence[[string]$key] = [bool]$Overrides[$key]
        }
        return [hashtable]$presence
    }

    $presence['cargo'] = [bool](Get-Command cargo -ErrorAction SilentlyContinue)
    $presence['rustc'] = [bool](Get-Command rustc -ErrorAction SilentlyContinue)
    $presence['npm.cmd'] = [bool](Get-Command npm.cmd -ErrorAction SilentlyContinue)
    if (-not $presence['npm.cmd']) {
        $presence['npm.cmd'] = [bool](Get-Command npm -ErrorAction SilentlyContinue)
    }
    $presence['node'] = [bool](Get-Command node -ErrorAction SilentlyContinue)

    $pythonCmd = Get-Command python -ErrorAction SilentlyContinue
    if (-not $pythonCmd) {
        $pythonCmd = Get-Command python3 -ErrorAction SilentlyContinue
    }
    $presence['python'] = [bool]$pythonCmd
    if ($pythonCmd) {
        $prevEap = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try {
            $null = & $pythonCmd.Source -c "import yaml" 2>$null
            $presence['pyyaml'] = ($LASTEXITCODE -eq 0)
        } catch {
            $presence['pyyaml'] = $false
        } finally {
            $ErrorActionPreference = $prevEap
        }
    }

    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $null = & cargo tauri --version 2>$null
        $presence['cargo-tauri'] = ($LASTEXITCODE -eq 0)
    } catch {
        $presence['cargo-tauri'] = $false
    } finally {
        $ErrorActionPreference = $prevEap
    }

    return [hashtable]$presence
}

function Test-ReleaseRunnerPreflight {
    <#
    .SYNOPSIS
    Local fail-closed preflight for Gitea/host release runners.

    .DESCRIPTION
    Produces a machine-readable readiness report distinguishing host-only
    readiness, missing dependencies, and explicit bundle/APK authorization.
    Never claims GUI, device, or remote CI evidence. Paths and secrets are
    redacted from the report.
    #>
    param(
        [Parameter(Mandatory = $true)]
        [ValidateSet('windows-host', 'android-host', 'ci-gates')]
        [string]$Profile,

        [string]$RepoRoot,

        [hashtable]$ToolPresence,

        [switch]$RequireBundle,
        [switch]$RequireApk,
        [switch]$FailClosed
    )

    if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
        $RepoRoot = Find-ReleaseRepoRoot
    } else {
        $RepoRoot = (Resolve-Path -LiteralPath $RepoRoot).ProviderPath
    }

    $tools = Get-ReleaseRunnerToolPresence -Overrides $ToolPresence
    $missing = New-Object System.Collections.Generic.List[string]
    $notes = New-Object System.Collections.Generic.List[string]

    $required = @('cargo', 'rustc', 'npm.cmd', 'node')
    if ($Profile -eq 'ci-gates') {
        $required = @('cargo', 'rustc', 'npm.cmd', 'node', 'python', 'pyyaml')
    }

    foreach ($name in $required) {
        if (-not [bool]$tools[$name]) {
            $missing.Add($name) | Out-Null
        }
    }

    if ($RequireBundle) {
        if (-not [bool]$tools['cargo-tauri']) {
            $missing.Add('cargo-tauri') | Out-Null
        }
    }

    if ($RequireApk) {
        $androidIssues = @(Get-ReleaseAndroidBuildPathIssues)
        foreach ($issue in $androidIssues) {
            if ($issue -match 'ANDROID_HOME') {
                $missing.Add('ANDROID_HOME') | Out-Null
            } elseif ($issue -match 'NDK_HOME') {
                $missing.Add('NDK_HOME') | Out-Null
            } else {
                $missing.Add((Protect-ReleasePath -Text $issue -RepoRoot $RepoRoot)) | Out-Null
            }
        }
        if ($Profile -ne 'android-host') {
            $notes.Add('APK authorization requested outside android-host profile') | Out-Null
        }
    }

    $uniqueMissing = @($missing | Select-Object -Unique)
    $ready = ($uniqueMissing.Count -eq 0)
    $status = if ($ready) {
        'ready'
    } elseif ($RequireBundle -or $RequireApk) {
        if (@($uniqueMissing | Where-Object { $_ -in @('cargo-tauri', 'ANDROID_HOME', 'NDK_HOME') }).Count -gt 0) {
            'needs_explicit_authorization'
        } else {
            'missing_dependencies'
        }
    } else {
        'missing_dependencies'
    }

    $notes.Add('Local preflight only; remote Gitea runner execution is not proven by this report.') | Out-Null
    $notes.Add('GUI acceptance and Android device acceptance are never claimable from this preflight.') | Out-Null
    if (-not $RequireBundle) {
        $notes.Add('Bundle evidence is not authorized; host-only path does not require cargo-tauri.') | Out-Null
    }
    if (-not $RequireApk) {
        $notes.Add('APK evidence is not authorized; host smoke does not require ANDROID_HOME/NDK_HOME.') | Out-Null
    }

    $safeTools = [ordered]@{}
    foreach ($key in @($tools.Keys | Sort-Object)) {
        $safeTools[[string]$key] = [bool]$tools[$key]
    }

    $report = [pscustomobject]@{
        schema_version = 1
        generated_at_utc = (Get-Date).ToUniversalTime().ToString('o')
        profile = $Profile
        ready = $ready
        status = $status
        missing = @($uniqueMissing | ForEach-Object {
            Protect-ReleasePath -Text ([string]$_) -RepoRoot $RepoRoot
        })
        tools = [pscustomobject]$safeTools
        intents = [pscustomobject]@{
            host_only = (-not $RequireBundle -and -not $RequireApk)
            bundle_authorized = [bool]$RequireBundle
            apk_authorized = [bool]$RequireApk
        }
        claims = [pscustomobject]@{
            gui = 'not_claimable'
            android_device = 'not_claimable'
            remote_ci = 'not_claimable'
            host_build = if ($ready -and $Profile -ne 'ci-gates') { 'preflight_ok' } else { 'not_proven' }
        }
        notes = @($notes | ForEach-Object {
            Protect-ReleasePath -Text ([string]$_) -RepoRoot $RepoRoot
        })
    }

    $report = Protect-ReleaseObject -Value $report -RepoRoot $RepoRoot

    if ($FailClosed -and -not $ready) {
        $detail = if ($uniqueMissing.Count -gt 0) { ($uniqueMissing -join ', ') } else { $status }
        throw ("Release runner preflight failed for profile '{0}': {1}" -f $Profile, $detail)
    }

    return $report
}

function Test-ReleaseEvidencePackage {
    <#
    .SYNOPSIS
    Offline-verifies a release evidence directory (manifest, provenance, subjects).

    .DESCRIPTION
    Fail-closed offline verification for evidence packages produced by the host
    runners. Rejects missing subjects/sidecars, hash mismatches, BOM sidecars,
    path escapes, reparse points, unknown schema versions, sensitive notes/warnings,
    identity mismatch, out-of-bounds remote_ci claims, and non-success build_status
    packages. Dry-run packages require -AllowDryRun and never claim remote CI.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$EvidenceDir,
        [switch]$AllowDryRun
    )

    $errors = New-Object System.Collections.Generic.List[string]
    $notes = New-Object System.Collections.Generic.List[string]
    $subjectCount = 0
    $remoteCiClaim = 'absent'
    $remoteCiClaimed = $false
    $buildStatus = 'unknown'
    $evidenceRoot = $null

    function Add-SafeEvidenceError {
        param([Parameter(Mandatory = $true)][string]$Message)
        $safe = Protect-ReleasePath -Text $Message -RepoRoot $evidenceRoot
        # Always strip absolute Windows/Unix home shapes even when RepoRoot is unknown.
        $safe = Protect-ReleasePath -Text $safe
        $errors.Add($safe) | Out-Null
    }

    if (-not (Test-Path -LiteralPath $EvidenceDir -PathType Container)) {
        Add-SafeEvidenceError -Message 'Evidence directory missing.'
        return [pscustomobject]@{
            Valid = $false
            ErrorCount = $errors.Count
            Errors = @($errors)
            Notes = @()
            subject_count = 0
            remote_ci_claim = $remoteCiClaim
            remote_ci_claimed = $false
            build_status = $buildStatus
        }
    }

    try {
        $null = Assert-ReleaseDirectoryNotReparsePoint -Path $EvidenceDir -Label 'Evidence directory'
    } catch {
        Add-SafeEvidenceError -Message $_.Exception.Message
        return [pscustomobject]@{
            Valid = $false
            ErrorCount = $errors.Count
            Errors = @($errors | ForEach-Object { Protect-ReleasePath -Text $_ })
            Notes = @()
            subject_count = 0
            remote_ci_claim = $remoteCiClaim
            remote_ci_claimed = $false
            build_status = $buildStatus
        }
    }

    $evidenceRoot = (Resolve-Path -LiteralPath $EvidenceDir).ProviderPath
    $manifestPath = Join-Path $evidenceRoot 'manifest.json'
    $provPath = Join-Path $evidenceRoot 'provenance.json'

    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        Add-SafeEvidenceError -Message 'manifest.json is missing from the evidence package.'
        return [pscustomobject]@{
            Valid = $false
            ErrorCount = $errors.Count
            Errors = @($errors)
            Notes = @()
            subject_count = 0
            remote_ci_claim = $remoteCiClaim
            remote_ci_claimed = $false
            build_status = $buildStatus
        }
    }

    try {
        $null = Assert-ReleasePathNotReparsePoint -Path $manifestPath -Label 'manifest.json'
        $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    } catch {
        Add-SafeEvidenceError -Message 'manifest.json is not valid JSON or is a reparse point.'
        return [pscustomobject]@{
            Valid = $false
            ErrorCount = $errors.Count
            Errors = @($errors)
            Notes = @()
            subject_count = 0
            remote_ci_claim = $remoteCiClaim
            remote_ci_claimed = $false
            build_status = $buildStatus
        }
    }

    $buildStatus = [string]$manifest.build_status

    try {
        Assert-ReleaseManifestSchema -Manifest $manifest
    } catch {
        Add-SafeEvidenceError -Message $_.Exception.Message
    }

    if ($null -eq $manifest.schema_version -or [int]$manifest.schema_version -ne 1) {
        Add-SafeEvidenceError -Message ("Unknown or unsupported schema_version '{0}'." -f $manifest.schema_version)
    }

    # Identity: non-empty commit/branch/target; commit must be a strict git SHA.
    foreach ($field in @('commit', 'branch', 'target')) {
        $val = [string]$manifest.$field
        if ([string]::IsNullOrWhiteSpace($val)) {
            Add-SafeEvidenceError -Message ("manifest {0} identity is empty." -f $field)
        }
    }
    if (-not [string]::IsNullOrWhiteSpace([string]$manifest.commit) -and -not (Test-ReleaseGitCommitSha -Commit ([string]$manifest.commit))) {
        Add-SafeEvidenceError -Message 'manifest commit is not a strict git SHA format.'
    }

    # remote_ci claim is explicit; raw value never returned unsanitized.
    $rawRemoteCiClaim = 'absent'
    if ($manifest.PSObject.Properties.Name -contains 'acceptance' -and $null -ne $manifest.acceptance) {
        $acc = $manifest.acceptance
        if ($acc.PSObject.Properties.Name -contains 'remote_ci') {
            $rawRemoteCiClaim = [string]$acc.remote_ci
        }
    }
    $allowedRemoteCi = @('not_claimed', 'not_claimable', 'absent', '')
    if (-not [string]::IsNullOrWhiteSpace($rawRemoteCiClaim) -and $allowedRemoteCi -notcontains $rawRemoteCiClaim) {
        $remoteCiClaimed = $true
        $secretHits = @(Find-ReleaseSecretPatternFindings -Text $rawRemoteCiClaim)
        if ($secretHits.Count -gt 0 -or $rawRemoteCiClaim -match 'sk-[A-Za-z0-9_-]{10,}') {
            $remoteCiClaim = 'out_of_bounds_REDACTED'
            Add-SafeEvidenceError -Message 'remote_ci claim is out of bounds and secret-shaped; value redacted from helper output.'
        } else {
            $controlled = Protect-ReleasePath -Text $rawRemoteCiClaim -RepoRoot $evidenceRoot
            $controlled = Protect-ReleasePath -Text $controlled
            # Bound length and character set so helper output stays controlled.
            if ($controlled.Length -gt 64) { $controlled = $controlled.Substring(0, 64) }
            $remoteCiClaim = 'out_of_bounds:' + $controlled
            Add-SafeEvidenceError -Message ("remote_ci claim '{0}' is out of bounds; only not_claimed/not_claimable are allowed." -f $controlled)
        }
    } else {
        $remoteCiClaimed = $false
        if ([string]::IsNullOrWhiteSpace($rawRemoteCiClaim)) {
            $remoteCiClaim = 'absent'
        } else {
            $remoteCiClaim = $rawRemoteCiClaim
        }
    }

    # partial/failed packages are never offline-success; fail closed with explicit status.
    if ($buildStatus -eq 'partial' -or $buildStatus -eq 'failed') {
        Add-SafeEvidenceError -Message ("build_status '{0}' is fail-closed; offline verification does not accept partial/failed packages as success." -f $buildStatus)
    }

    # Recursive generic secret scan over every string leaf and object key in the manifest.
    $manifestSecretFindings = New-Object System.Collections.Generic.List[string]
    $manifestLeafWalk = Get-ReleaseObjectStringLeaves -Value $manifest
    if ($manifestLeafWalk.DepthExceeded) {
        Add-SafeEvidenceError -Message ("manifest object graph exceeds scanner max depth ({0}); fail-closed depth overflow." -f $manifestLeafWalk.MaxDepth)
    }
    foreach ($leaf in @($manifestLeafWalk.Leaves)) {
        if ($null -eq $leaf) { continue }
        $text = [string]$leaf
        if ([string]::IsNullOrWhiteSpace($text)) { continue }
        foreach ($hit in @(Find-ReleaseSecretPatternFindings -Text $text)) {
            $manifestSecretFindings.Add([string]$hit) | Out-Null
        }
    }
    if ($manifestSecretFindings.Count -gt 0) {
        Add-SafeEvidenceError -Message 'Sensitive secret-like content found in manifest object graph; package rejected (values redacted).'
    }

    $isDryRun = ($buildStatus -eq 'dry-run')
    if ($isDryRun) {
        if (-not $AllowDryRun) {
            Add-SafeEvidenceError -Message 'dry-run evidence package rejected without -AllowDryRun; dry-run is not remote CI evidence.'
        } else {
            $notes.Add('Accepted as local dry-run evidence only; not remote CI, not GUI, not device acceptance.') | Out-Null
        }
    }

    if ($buildStatus -eq 'ok') {
        if (-not (Test-Path -LiteralPath $provPath -PathType Leaf)) {
            Add-SafeEvidenceError -Message 'provenance.json is missing from the evidence package.'
        }
        $invMeta = $manifest.dependency_inventory
        if ($null -eq $invMeta -or [string]::IsNullOrWhiteSpace([string]$invMeta.relative_path)) {
            Add-SafeEvidenceError -Message 'dependency inventory metadata is missing from the manifest for build_status=ok.'
        } else {
            try {
                $invPath = Test-ReleaseEvidencePathSafe `
                    -EvidenceRoot $evidenceRoot `
                    -RelativePath ([string]$invMeta.relative_path) `
                    -Label 'dependency inventory'
                if (-not (Test-Path -LiteralPath $invPath -PathType Leaf)) {
                    Add-SafeEvidenceError -Message 'dependency inventory file is missing from the evidence package.'
                } elseif (-not [string]::IsNullOrWhiteSpace([string]$invMeta.sha256)) {
                    $invHash = Get-ReleaseFileSha256 -Path $invPath
                    if ($invHash -ne ([string]$invMeta.sha256).ToLowerInvariant()) {
                        Add-SafeEvidenceError -Message 'dependency inventory sha256 mismatch.'
                    }
                }
            } catch {
                Add-SafeEvidenceError -Message $_.Exception.Message
            }
        }
    } elseif ($isDryRun -and $AllowDryRun) {
        if (Test-Path -LiteralPath $provPath -PathType Leaf) {
            $notes.Add('dry-run package includes provenance; still not remote CI evidence.') | Out-Null
        }
        $notes.Add('dry-run packages may omit staged subjects; offline rehash of present inventory/manifest only.') | Out-Null
    }

    $prov = $null
    if (Test-Path -LiteralPath $provPath -PathType Leaf) {
        try {
            $null = Assert-ReleasePathNotReparsePoint -Path $provPath -Label 'provenance.json'
            $prov = Get-Content -LiteralPath $provPath -Raw | ConvertFrom-Json
            if ($null -eq $prov.schema_version -or [int]$prov.schema_version -ne 1) {
                Add-SafeEvidenceError -Message ("Unknown or unsupported provenance schema_version '{0}'." -f $prov.schema_version)
            }

            foreach ($field in @('commit', 'branch', 'target')) {
                $pVal = [string]$prov.$field
                if ([string]::IsNullOrWhiteSpace($pVal)) {
                    Add-SafeEvidenceError -Message ("provenance {0} identity is empty." -f $field)
                }
            }
            if (-not [string]::IsNullOrWhiteSpace([string]$prov.commit) -and -not (Test-ReleaseGitCommitSha -Commit ([string]$prov.commit))) {
                Add-SafeEvidenceError -Message 'provenance commit is not a strict git SHA format.'
            }

            # Identity consistency: commit/branch/target must match the manifest.
            foreach ($field in @('commit', 'branch', 'target')) {
                $mVal = [string]$manifest.$field
                $pVal = [string]$prov.$field
                if ($mVal -ne $pVal) {
                    Add-SafeEvidenceError -Message ("manifest/provenance {0} identity mismatch." -f $field)
                }
            }

            $provSecretFindings = New-Object System.Collections.Generic.List[string]
            $provLeafWalk = Get-ReleaseObjectStringLeaves -Value $prov
            if ($provLeafWalk.DepthExceeded) {
                Add-SafeEvidenceError -Message ("provenance object graph exceeds scanner max depth ({0}); fail-closed depth overflow." -f $provLeafWalk.MaxDepth)
            }
            foreach ($leaf in @($provLeafWalk.Leaves)) {
                if ($null -eq $leaf) { continue }
                $text = [string]$leaf
                if ([string]::IsNullOrWhiteSpace($text)) { continue }
                foreach ($hit in @(Find-ReleaseSecretPatternFindings -Text $text)) {
                    $provSecretFindings.Add([string]$hit) | Out-Null
                }
            }
            if ($provSecretFindings.Count -gt 0) {
                Add-SafeEvidenceError -Message 'Sensitive secret-like content found in provenance object graph; package rejected (values redacted).'
            }
        } catch {
            if ($_.Exception.Message -match 'reparse|symlink|junction') {
                Add-SafeEvidenceError -Message $_.Exception.Message
            } elseif ($_.Exception.Message -match 'identity|SHA|empty|schema|secret') {
                Add-SafeEvidenceError -Message $_.Exception.Message
            } else {
                Add-SafeEvidenceError -Message 'provenance.json is not valid JSON or cannot be read safely.'
            }
        }
    }

    function Test-OneEvidencePresentEntry {
        param(
            [Parameter(Mandatory = $true)][object]$Entry,
            [Parameter(Mandatory = $true)][string]$Label
        )

        $rel = [string]$Entry.relative_path
        if (-not (Test-ReleaseStagedSubjectPath -RelativePath $rel)) {
            Add-SafeEvidenceError -Message ("{0} path is not a staged subjects/ path (source paths are not offline-verifiable): {1}" -f $Label, (Protect-ReleasePath -Text $rel -RepoRoot $evidenceRoot))
            return $false
        }

        if ($Entry.PSObject.Properties.Name -notcontains 'size_bytes' -or $null -eq $Entry.size_bytes -or [string]::IsNullOrWhiteSpace([string]$Entry.size_bytes)) {
            Add-SafeEvidenceError -Message ("{0} missing required size_bytes: {1}" -f $Label, (Protect-ReleasePath -Text $rel -RepoRoot $evidenceRoot))
            return $false
        }
        try {
            $declaredSize = [long]$Entry.size_bytes
        } catch {
            Add-SafeEvidenceError -Message ("{0} size_bytes is not a valid integer: {1}" -f $Label, (Protect-ReleasePath -Text $rel -RepoRoot $evidenceRoot))
            return $false
        }
        if ($declaredSize -lt 0) {
            Add-SafeEvidenceError -Message ("{0} size_bytes must be non-negative: {1}" -f $Label, (Protect-ReleasePath -Text $rel -RepoRoot $evidenceRoot))
            return $false
        }

        try {
            $subjectPath = Test-ReleaseEvidencePathSafe `
                -EvidenceRoot $evidenceRoot `
                -RelativePath $rel `
                -Label $Label
        } catch {
            Add-SafeEvidenceError -Message $_.Exception.Message
            return $false
        }

        if (-not (Test-Path -LiteralPath $subjectPath -PathType Leaf)) {
            Add-SafeEvidenceError -Message ("{0} file missing: {1}" -f $Label, (Protect-ReleasePath -Text $rel -RepoRoot $evidenceRoot))
            return $false
        }

        try {
            $null = Assert-ReleasePathNotReparsePoint -Path $subjectPath -Label ("{0} file" -f $Label)
        } catch {
            Add-SafeEvidenceError -Message $_.Exception.Message
            return $false
        }

        $sidecarRel = if ($Entry.PSObject.Properties.Name -contains 'hash_sidecar' -and -not [string]::IsNullOrWhiteSpace([string]$Entry.hash_sidecar)) {
            ([string]$Entry.hash_sidecar) -replace '\\', '/'
        } else {
            (([string]$rel) -replace '\\', '/') + '.sha256'
        }
        try {
            $sidecar = Test-ReleaseEvidencePathSafe `
                -EvidenceRoot $evidenceRoot `
                -RelativePath $sidecarRel `
                -Label ("{0} hash sidecar" -f $Label)
        } catch {
            $sidecar = $subjectPath + '.sha256'
            if ($_.Exception.Message -match 'reparse|symlink|junction|escape|outside') {
                Add-SafeEvidenceError -Message $_.Exception.Message
                return $false
            }
        }

        if (-not (Test-Path -LiteralPath $sidecar -PathType Leaf)) {
            Add-SafeEvidenceError -Message ("Hash sidecar missing for {0}: {1}" -f $Label, (Protect-ReleasePath -Text $rel -RepoRoot $evidenceRoot))
            return $false
        }

        try {
            $null = Assert-ReleasePathNotReparsePoint -Path $sidecar -Label ("{0} hash sidecar" -f $Label)
        } catch {
            Add-SafeEvidenceError -Message $_.Exception.Message
            return $false
        }

        $sidecarBytes = [System.IO.File]::ReadAllBytes($sidecar)
        if ($sidecarBytes.Length -ge 3 -and $sidecarBytes[0] -eq 0xEF -and $sidecarBytes[1] -eq 0xBB -and $sidecarBytes[2] -eq 0xBF) {
            Add-SafeEvidenceError -Message ("Hash sidecar is UTF-8 BOM encoded (rejected): {0}" -f (Protect-ReleasePath -Text $sidecarRel -RepoRoot $evidenceRoot))
        }

        $sidecarText = [System.Text.Encoding]::UTF8.GetString($sidecarBytes)
        if ($sidecarText.Length -gt 0 -and [int][char]$sidecarText[0] -eq 0xFEFF) {
            Add-SafeEvidenceError -Message ("Hash sidecar has BOM marker (rejected): {0}" -f (Protect-ReleasePath -Text $sidecarRel -RepoRoot $evidenceRoot))
            $sidecarText = $sidecarText.TrimStart([char]0xFEFF)
        }
        # Preserve newline structure for multi-line rejection; only strip outer whitespace edges lightly.
        $sidecarText = $sidecarText.TrimEnd("`0")

        $rehash = Get-ReleaseFileSha256 -Path $subjectPath
        $expected = ([string]$Entry.sha256).ToLowerInvariant()
        if ([string]::IsNullOrWhiteSpace($expected)) {
            Add-SafeEvidenceError -Message ("{0} missing sha256 digest: {1}" -f $Label, (Protect-ReleasePath -Text $rel -RepoRoot $evidenceRoot))
        } elseif ($rehash -ne $expected) {
            Add-SafeEvidenceError -Message ("Offline rehash mismatch for {0}: {1}" -f $Label, (Protect-ReleasePath -Text $rel -RepoRoot $evidenceRoot))
        }

        $actualSize = [long](Get-Item -LiteralPath $subjectPath).Length
        if ($actualSize -ne $declaredSize) {
            Add-SafeEvidenceError -Message ("Size mismatch for {0}: {1}" -f $Label, (Protect-ReleasePath -Text $rel -RepoRoot $evidenceRoot))
        }

        $expectedBase = Split-Path -Leaf (($rel -replace '\\', '/'))
        $sumLine = Test-ReleaseSha256SumLine -Text $sidecarText -ExpectedBasename $expectedBase
        if (-not $sumLine.Valid) {
            Add-SafeEvidenceError -Message ("Sidecar is not a single standard sha256sum line (grammar={0}): {1}" -f $sumLine.Reason, (Protect-ReleasePath -Text $sidecarRel -RepoRoot $evidenceRoot))
        } else {
            $sidecarHash = [string]$sumLine.Hash
            if ($sidecarHash -ne $rehash -or ($expected -and $sidecarHash -ne $expected)) {
                Add-SafeEvidenceError -Message ("Sidecar hash mismatch for {0}: {1}" -f $Label, (Protect-ReleasePath -Text $rel -RepoRoot $evidenceRoot))
            }
        }

        return $true
    }

    # For build_status=ok, require bidirectional exact-set binding between
    # manifest.staged_subjects (evidence-relative) and provenance.subjects.
    # Source artifact paths (target/release/..., APK build tree, etc.) are never
    # offline-verified as subject files.
    if ($buildStatus -eq 'ok') {
        $stagedSubjects = @()
        if ($manifest.PSObject.Properties.Name -contains 'staged_subjects' -and $null -ne $manifest.staged_subjects) {
            $stagedSubjects = @($manifest.staged_subjects | Where-Object { $null -ne $_ -and $_.status -eq 'present' })
        }
        $provSubjects = @()
        if ($null -ne $prov -and $prov.PSObject.Properties.Name -contains 'subjects') {
            $provSubjects = @($prov.subjects | Where-Object { $null -ne $_ })
        }

        if ($stagedSubjects.Count -eq 0) {
            Add-SafeEvidenceError -Message 'build_status=ok package has no present staged subjects to bind (source artifact paths alone are not offline-verifiable).'
        }
        if ($provSubjects.Count -eq 0) {
            Add-SafeEvidenceError -Message 'build_status=ok package has no provenance subjects to bind.'
        }

        # Each present staged_subject must uniquely map to one present source artifact
        # by source_relative_path + kind + sha256 + size_bytes + status. Offline verifier
        # does not reopen the source file outside the package; it checks the declared chain.
        $presentArtifacts = @($manifest.artifacts | Where-Object { $null -ne $_ -and $_.status -eq 'present' })
        $artifactBySource = @{}
        foreach ($art in $presentArtifacts) {
            $srcKey = (([string]$art.relative_path) -replace '\\', '/').Trim().TrimStart('./').ToLowerInvariant()
            while ($srcKey.Contains('//')) { $srcKey = $srcKey -replace '//', '/' }
            if ([string]::IsNullOrWhiteSpace($srcKey)) { continue }
            if ($artifactBySource.ContainsKey($srcKey)) {
                Add-SafeEvidenceError -Message ("duplicate present manifest artifact source path: {0}" -f (Protect-ReleasePath -Text $srcKey -RepoRoot $evidenceRoot))
            } else {
                $artifactBySource[$srcKey] = $art
            }
        }
        $mappedSources = @{}
        foreach ($subj in $stagedSubjects) {
            $srcRel = $null
            if ($subj.PSObject.Properties.Name -contains 'source_relative_path') {
                $srcRel = [string]$subj.source_relative_path
            }
            if ([string]::IsNullOrWhiteSpace($srcRel)) {
                Add-SafeEvidenceError -Message ("staged subject missing source_relative_path mapping: {0}" -f (Protect-ReleasePath -Text ([string]$subj.relative_path) -RepoRoot $evidenceRoot))
                continue
            }
            $srcKey = ($srcRel -replace '\\', '/').Trim().TrimStart('./').ToLowerInvariant()
            while ($srcKey.Contains('//')) { $srcKey = $srcKey -replace '//', '/' }
            if (-not $artifactBySource.ContainsKey($srcKey)) {
                Add-SafeEvidenceError -Message ("staged subject has no present source artifact mapping: {0}" -f (Protect-ReleasePath -Text $srcRel -RepoRoot $evidenceRoot))
                continue
            }
            if ($mappedSources.ContainsKey($srcKey)) {
                Add-SafeEvidenceError -Message ("duplicate staged subject mapping to the same source artifact: {0}" -f (Protect-ReleasePath -Text $srcRel -RepoRoot $evidenceRoot))
                continue
            }
            $mappedSources[$srcKey] = $true
            $art = $artifactBySource[$srcKey]
            $checks = @(
                @{ Name = 'kind'; Left = [string]$subj.kind; Right = [string]$art.kind },
                @{ Name = 'sha256'; Left = ([string]$subj.sha256).ToLowerInvariant(); Right = ([string]$art.sha256).ToLowerInvariant() },
                @{ Name = 'status'; Left = [string]$subj.status; Right = [string]$art.status }
            )
            foreach ($c in $checks) {
                if ($c.Left -ne $c.Right) {
                    Add-SafeEvidenceError -Message ("staged subject source mapping inconsistent on {0}: {1}" -f $c.Name, (Protect-ReleasePath -Text $srcRel -RepoRoot $evidenceRoot))
                }
            }
            $subjSizeOk = $true
            $artSizeOk = $true
            try { $subjSize = [long]$subj.size_bytes } catch { $subjSizeOk = $false }
            try { $artSize = [long]$art.size_bytes } catch { $artSizeOk = $false }
            if (-not $subjSizeOk -or -not $artSizeOk -or $subjSize -ne $artSize) {
                Add-SafeEvidenceError -Message ("staged subject source mapping inconsistent on size_bytes: {0}" -f (Protect-ReleasePath -Text $srcRel -RepoRoot $evidenceRoot))
            }
        }

        # Reverse exact-set: every present artifact must be covered by exactly one
        # staged subject (and therefore by provenance once staged↔prov binding holds).
        foreach ($srcKey in @($artifactBySource.Keys)) {
            if (-not $mappedSources.ContainsKey($srcKey)) {
                Add-SafeEvidenceError -Message ("present manifest artifact has no staged subject/provenance coverage: {0}" -f (Protect-ReleasePath -Text $srcKey -RepoRoot $evidenceRoot))
            }
        }
        if ($presentArtifacts.Count -gt 0 -and $stagedSubjects.Count -ne $presentArtifacts.Count) {
            # Count mismatch is an additional fail-closed signal when uniqueness held.
            if ($mappedSources.Count -ne $presentArtifacts.Count -or $mappedSources.Count -ne $stagedSubjects.Count) {
                Add-SafeEvidenceError -Message ("exact-set artifact/staged coverage count mismatch: present_artifacts={0} staged_subjects={1} mapped={2}." -f $presentArtifacts.Count, $stagedSubjects.Count, $mappedSources.Count)
            }
        }

        $stagedKeySet = @{}
        $provKeySet = @{}

        foreach ($subj in $stagedSubjects) {
            if (-not (Test-ReleaseStagedSubjectPath -RelativePath ([string]$subj.relative_path))) {
                Add-SafeEvidenceError -Message ("staged subject path is not under subjects/: {0}" -f (Protect-ReleasePath -Text ([string]$subj.relative_path) -RepoRoot $evidenceRoot))
                continue
            }
            try {
                $key = Get-ReleaseEvidenceSubjectBindingKey -Entry $subj
            } catch {
                Add-SafeEvidenceError -Message $_.Exception.Message
                continue
            }
            if ($stagedKeySet.ContainsKey($key)) {
                Add-SafeEvidenceError -Message ("duplicate staged subject in exact-set binding: {0}" -f (Protect-ReleasePath -Text ([string]$subj.relative_path) -RepoRoot $evidenceRoot))
            } else {
                $stagedKeySet[$key] = $true
            }
        }
        foreach ($subj in $provSubjects) {
            if (-not (Test-ReleaseStagedSubjectPath -RelativePath ([string]$subj.relative_path))) {
                Add-SafeEvidenceError -Message ("provenance subject path is not under subjects/ (source paths are not offline-verifiable): {0}" -f (Protect-ReleasePath -Text ([string]$subj.relative_path) -RepoRoot $evidenceRoot))
                continue
            }
            try {
                $key = Get-ReleaseEvidenceSubjectBindingKey -Entry $subj
            } catch {
                Add-SafeEvidenceError -Message $_.Exception.Message
                continue
            }
            if ($provKeySet.ContainsKey($key)) {
                Add-SafeEvidenceError -Message ("duplicate provenance subject in exact-set binding: {0}" -f (Protect-ReleasePath -Text ([string]$subj.relative_path) -RepoRoot $evidenceRoot))
            } else {
                $provKeySet[$key] = $true
            }
        }

        foreach ($key in @($stagedKeySet.Keys)) {
            if (-not $provKeySet.ContainsKey($key)) {
                $rel = ($key -split '\|')[0]
                Add-SafeEvidenceError -Message ("exact-set binding mismatch: staged subject missing from provenance subjects ({0})." -f (Protect-ReleasePath -Text $rel -RepoRoot $evidenceRoot))
            }
        }
        foreach ($key in @($provKeySet.Keys)) {
            if (-not $stagedKeySet.ContainsKey($key)) {
                $rel = ($key -split '\|')[0]
                Add-SafeEvidenceError -Message ("exact-set binding mismatch: provenance subject is extra vs staged subjects ({0})." -f (Protect-ReleasePath -Text $rel -RepoRoot $evidenceRoot))
            }
        }

        # Always fully check every present staged subject (existence, non-reparse,
        # sidecar, rehash, size) even when provenance subjects exist. Never skip.
        foreach ($subj in $stagedSubjects) {
            if (Test-OneEvidencePresentEntry -Entry $subj -Label 'Staged subject') {
                $subjectCount += 1
            }
        }
        # Also fully check provenance subjects (same physical set when binding holds;
        # still required so a binding bug cannot skip a subject path).
        foreach ($subj in $provSubjects) {
            $null = Test-OneEvidencePresentEntry -Entry $subj -Label 'Provenance subject'
        }
    } else {
        # Non-ok paths: still verify any present provenance/staged subjects when present.
        $subjects = @()
        if ($null -ne $prov -and $prov.PSObject.Properties.Name -contains 'subjects') {
            $subjects = @($prov.subjects | Where-Object { $null -ne $_ -and $_.status -eq 'present' })
        }
        if ($subjects.Count -eq 0 -and $manifest.PSObject.Properties.Name -contains 'staged_subjects') {
            $subjects = @($manifest.staged_subjects | Where-Object { $null -ne $_ -and $_.status -eq 'present' })
        }
        foreach ($subj in $subjects) {
            if (Test-OneEvidencePresentEntry -Entry $subj -Label 'Subject') {
                $subjectCount += 1
            }
        }
    }

    $safeErrors = @($errors | ForEach-Object {
        Protect-ReleasePath -Text (Protect-ReleasePath -Text ([string]$_) -RepoRoot $evidenceRoot)
    })
    $safeNotes = @($notes | ForEach-Object {
        Protect-ReleasePath -Text (Protect-ReleasePath -Text ([string]$_) -RepoRoot $evidenceRoot)
    })
    $safeRemoteClaim = Protect-ReleasePath -Text (Protect-ReleasePath -Text ([string]$remoteCiClaim) -RepoRoot $evidenceRoot)

    return [pscustomobject]@{
        Valid = ($errors.Count -eq 0)
        ErrorCount = $errors.Count
        Errors = $safeErrors
        Notes = $safeNotes
        subject_count = $subjectCount
        remote_ci_claim = $safeRemoteClaim
        remote_ci_claimed = [bool]$remoteCiClaimed
        build_status = $buildStatus
        trust_model = (Get-ReleaseEvidenceVerifierTrustModel).model
    }
}

function Assert-ReleaseEvidencePackage {
    param(
        [Parameter(Mandatory = $true)][string]$EvidenceDir,
        [switch]$AllowDryRun
    )

    $result = Test-ReleaseEvidencePackage -EvidenceDir $EvidenceDir -AllowDryRun:$AllowDryRun
    if (-not $result.Valid) {
        $joined = ($result.Errors -join [Environment]::NewLine)
        throw ("Evidence package verification failed:{0}{1}" -f [Environment]::NewLine, $joined)
    }
    return $result
}

function Test-ReleaseRunInvokesCommand {
    <#
    .SYNOPSIS
    Returns true only for a flat, top-level reachable CommandAst whose command name
    equals the requested command.

    .DESCRIPTION
    The verifier step is modeled as a restricted flat script. Before the wanted
    command is reached, only simple assignments and a single-command dot-source
    (`. path`) are allowed. Accepts:
      - top-level bare call: Assert-X ...
      - top-level assignment RHS: $result = Assert-X ...
    Rejects Write-Host/string decoys, commands nested in if/loop/function/try/
    scriptblock, any pre-verifier control flow (including `if ($true) { return }`),
    and any statement after a top-level return/exit/throw.
    #>
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$ScriptText,
        [Parameter(Mandatory = $true)][string]$CommandName
    )

    if ([string]::IsNullOrWhiteSpace($ScriptText) -or [string]::IsNullOrWhiteSpace($CommandName)) {
        return $false
    }

    $tokens = $null
    $parseErrors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseInput(
        $ScriptText,
        [ref]$tokens,
        [ref]$parseErrors
    )
    if ($null -eq $ast) {
        return $false
    }
    # Fail closed on parse errors: cannot trust malformed run scripts as verifiers.
    if ($null -ne $parseErrors -and @($parseErrors).Count -gt 0) {
        return $false
    }

    $wanted = $CommandName.Trim()
    $root = $ast.EndBlock
    if ($null -eq $root -or $null -eq $root.Statements) {
        return $false
    }

    function Get-CommandNameFromAst {
        param([Parameter(Mandatory = $true)]$CommandAst)
        if ($null -eq $CommandAst.CommandElements -or $CommandAst.CommandElements.Count -lt 1) {
            return $null
        }
        $first = $CommandAst.CommandElements[0]
        if ($first -is [System.Management.Automation.Language.StringConstantExpressionAst]) {
            return [string]$first.Value
        }
        return $null
    }

    function Test-IsTransferCommand {
        param([Parameter(Mandatory = $true)][string]$Name)
        return [string]::Equals($Name, 'return', [System.StringComparison]::OrdinalIgnoreCase) -or
            [string]::Equals($Name, 'exit', [System.StringComparison]::OrdinalIgnoreCase) -or
            [string]::Equals($Name, 'throw', [System.StringComparison]::OrdinalIgnoreCase)
    }

    function Test-IsControlFlowStatement {
        param([Parameter(Mandatory = $true)]$Statement)
        return (
            $Statement -is [System.Management.Automation.Language.IfStatementAst] -or
            $Statement -is [System.Management.Automation.Language.SwitchStatementAst] -or
            $Statement -is [System.Management.Automation.Language.ForStatementAst] -or
            $Statement -is [System.Management.Automation.Language.ForEachStatementAst] -or
            $Statement -is [System.Management.Automation.Language.WhileStatementAst] -or
            $Statement -is [System.Management.Automation.Language.DoWhileStatementAst] -or
            $Statement -is [System.Management.Automation.Language.DoUntilStatementAst] -or
            $Statement -is [System.Management.Automation.Language.TryStatementAst] -or
            $Statement -is [System.Management.Automation.Language.TrapStatementAst] -or
            $Statement -is [System.Management.Automation.Language.FunctionDefinitionAst] -or
            $Statement -is [System.Management.Automation.Language.DataStatementAst]
        )
    }

    function Test-PipelineRunsInBackground {
        param([Parameter(Mandatory = $true)]$Pipeline)
        return (($Pipeline.PSObject.Properties.Name -contains 'Background') -and [bool]$Pipeline.Background)
    }

    function Test-AssignmentIsWantedCommand {
        param(
            [Parameter(Mandatory = $true)]$Assignment,
            [Parameter(Mandatory = $true)][string]$WantedName
        )
        if ($Assignment.Operator -ne [System.Management.Automation.Language.TokenKind]::Equals) {
            return $false
        }
        if ($Assignment.Left -isnot [System.Management.Automation.Language.VariableExpressionAst] -or
            -not [string]::Equals([string]$Assignment.Left.VariablePath.UserPath, 'result', [System.StringComparison]::OrdinalIgnoreCase)) {
            return $false
        }
        $rhs = $Assignment.Right
        if ($rhs -is [System.Management.Automation.Language.CommandExpressionAst]) {
            # PowerShell wraps assignment RHS expressions; pipeline RHS is nested.
            if ($rhs.Expression -is [System.Management.Automation.Language.PipelineAst]) {
                $rhs = $rhs.Expression
            } else {
                return $false
            }
        }
        if ($rhs -isnot [System.Management.Automation.Language.PipelineAst]) {
            return $false
        }
        if (Test-PipelineRunsInBackground -Pipeline $rhs) {
            return $false
        }
        $elements = @($rhs.PipelineElements)
        if ($elements.Count -ne 1 -or $elements[0] -isnot [System.Management.Automation.Language.CommandAst]) {
            return $false
        }
        $name = Get-CommandNameFromAst -CommandAst $elements[0]
        if ($null -eq $name) {
            return $false
        }
        return [string]::Equals($name, $WantedName, [System.StringComparison]::OrdinalIgnoreCase)
    }

    # Walk only top-level statements. Before the wanted command is reached the step
    # must stay flat: simple assignment and dot-source only. Nested blocks and any
    # pre-verifier control flow (including conditional return/exit/throw) fail closed.
    foreach ($stmt in @($root.Statements)) {
        if ($null -eq $stmt) { continue }

        # Unconditional transfer statements make later top-level statements unreachable.
        if ($stmt -is [System.Management.Automation.Language.ReturnStatementAst] -or
            $stmt -is [System.Management.Automation.Language.ExitStatementAst] -or
            $stmt -is [System.Management.Automation.Language.ThrowStatementAst]) {
            return $false
        }

        # Pre-verifier control flow can skip the call (e.g. if ($true) { return }).
        if (Test-IsControlFlowStatement -Statement $stmt) {
            return $false
        }

        # Pipeline statement: command1 | command2 ...  (single-command only)
        if ($stmt -is [System.Management.Automation.Language.PipelineAst]) {
            if (Test-PipelineRunsInBackground -Pipeline $stmt) {
                return $false
            }
            $elements = @($stmt.PipelineElements)
            if ($elements.Count -ne 1) {
                # Multi-command pipelines are not a guaranteed standalone verifier call.
                return $false
            }
            $elem = $elements[0]
            if ($elem -isnot [System.Management.Automation.Language.CommandAst]) {
                return $false
            }
            # Dot-source (`. path`) is the only non-wanted setup command allowed
            # before the verifier. InvocationOperator carries the dot; the first
            # CommandElement is the path expression, not a command name of '.'.
            if ($elem.InvocationOperator -eq [System.Management.Automation.Language.TokenKind]::Dot) {
                continue
            }
            # Call operator (& path) is not accepted as the verifier itself here.
            if ($elem.InvocationOperator -eq [System.Management.Automation.Language.TokenKind]::Ampersand) {
                return $false
            }
            $name = Get-CommandNameFromAst -CommandAst $elem
            if ($null -eq $name) {
                return $false
            }
            if (Test-IsTransferCommand -Name $name) {
                return $false
            }
            if ([string]::Equals($name, $wanted, [System.StringComparison]::OrdinalIgnoreCase)) {
                return $true
            }
            # Any other pre-verifier command (Write-Host, etc.) fails closed.
            return $false
        }

        # Assignment: simple values allowed; RHS may be the wanted command.
        if ($stmt -is [System.Management.Automation.Language.AssignmentStatementAst]) {
            if (Test-AssignmentIsWantedCommand -Assignment $stmt -WantedName $wanted) {
                return $true
            }
            # Non-command RHS (or non-wanted command) is allowed as flat setup,
            # but nested control-flow / scriptblocks in the assignment fail closed.
            $nestedControl = $false
            foreach ($node in @($stmt.FindAll({
                            param($n)
                            $n -is [System.Management.Automation.Language.IfStatementAst] -or
                            $n -is [System.Management.Automation.Language.SwitchStatementAst] -or
                            $n -is [System.Management.Automation.Language.ForStatementAst] -or
                            $n -is [System.Management.Automation.Language.ForEachStatementAst] -or
                            $n -is [System.Management.Automation.Language.WhileStatementAst] -or
                            $n -is [System.Management.Automation.Language.DoWhileStatementAst] -or
                            $n -is [System.Management.Automation.Language.DoUntilStatementAst] -or
                            $n -is [System.Management.Automation.Language.TryStatementAst] -or
                            $n -is [System.Management.Automation.Language.TrapStatementAst] -or
                            $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -or
                            $n -is [System.Management.Automation.Language.ScriptBlockExpressionAst]
                        }, $true))) {
                if ($null -ne $node) {
                    $nestedControl = $true
                    break
                }
            }
            if ($nestedControl) {
                return $false
            }
            continue
        }

        # Any other top-level statement kind before the verifier fails closed.
        return $false
    }

    return $false
}

function Test-ReleaseVerifierStepScriptContract {
    <#
    .SYNOPSIS
    Fail-closed checks for a host-evidence verifier run script beyond CommandAst reachability.

    .DESCRIPTION
    Requires a flat pwsh script that:
      - sources scripts/release-build/ReleaseBuild.Common.ps1 via InvocationOperator Dot
      - invokes Assert-ReleaseEvidencePackage without -AllowDryRun
      - binds -EvidenceDir to either a literal steps.evidence.outputs.dir expression
        or a variable previously assigned from that exact expression
    Returns a PSCustomObject with Valid, Errors, EvidenceDirExpression.
    #>
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$ScriptText
    )

    $errs = New-Object System.Collections.Generic.List[string]
    $evidenceExpr = $null
    $requiredEvidenceLiteral = '${{ steps.evidence.outputs.dir }}'
    $requiredDotSourceRelativePath = 'scripts/release-build/ReleaseBuild.Common.ps1'
    $wanted = 'Assert-ReleaseEvidencePackage'

    if ([string]::IsNullOrWhiteSpace($ScriptText)) {
        return [pscustomobject]@{
            Valid = $false
            Errors = @('verifier run script is empty')
            EvidenceDirExpression = $null
        }
    }

    if (-not (Test-ReleaseRunInvokesCommand -ScriptText $ScriptText -CommandName $wanted)) {
        $errs.Add('missing top-level reachable Assert-ReleaseEvidencePackage CommandAst (flat script required)') | Out-Null
        return [pscustomobject]@{
            Valid = $false
            Errors = @($errs)
            EvidenceDirExpression = $null
        }
    }

    $tokens = $null
    $parseErrors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseInput(
        $ScriptText,
        [ref]$tokens,
        [ref]$parseErrors
    )
    if ($null -eq $ast -or ($null -ne $parseErrors -and @($parseErrors).Count -gt 0)) {
        $errs.Add('verifier run script failed PowerShell parse') | Out-Null
        return [pscustomobject]@{
            Valid = $false
            Errors = @($errs)
            EvidenceDirExpression = $null
        }
    }

    $usingStatements = @($ast.UsingStatements | Where-Object { $null -ne $_ })
    if ($usingStatements.Count -gt 0 -or
        $null -ne $ast.ScriptRequirements -or
        $null -ne $ast.ParamBlock -or
        $null -ne $ast.DynamicParamBlock -or
        $null -ne $ast.BeginBlock -or
        $null -ne $ast.ProcessBlock -or
        (($ast.PSObject.Properties.Name -contains 'CleanBlock') -and $null -ne $ast.CleanBlock)) {
        $errs.Add('verifier run must use a bare EndBlock only; using/requires/param and named-block preambles are not allowed') | Out-Null
        return [pscustomobject]@{
            Valid = $false
            Errors = @($errs)
            EvidenceDirExpression = $null
        }
    }

    $root = $ast.EndBlock
    if ($null -eq $root -or $null -eq $root.Statements) {
        $errs.Add('verifier run script has no statements') | Out-Null
        return [pscustomobject]@{
            Valid = $false
            Errors = @($errs)
            EvidenceDirExpression = $null
        }
    }

    function Get-CommandNameFromAstLocal {
        param([Parameter(Mandatory = $true)]$CommandAst)
        if ($null -eq $CommandAst.CommandElements -or $CommandAst.CommandElements.Count -lt 1) {
            return $null
        }
        $first = $CommandAst.CommandElements[0]
        if ($first -is [System.Management.Automation.Language.StringConstantExpressionAst]) {
            return [string]$first.Value
        }
        return $null
    }

    function Get-StringConstantValue {
        param($Node)
        if ($null -eq $Node) { return $null }
        if ($Node -is [System.Management.Automation.Language.StringConstantExpressionAst]) {
            return [string]$Node.Value
        }
        if ($Node -is [System.Management.Automation.Language.CommandExpressionAst]) {
            return Get-StringConstantValue -Node $Node.Expression
        }
        return $null
    }

    function Test-IsRequiredCommonDotSource {
        param([Parameter(Mandatory = $true)]$CommandAst)
        if ($CommandAst.InvocationOperator -ne [System.Management.Automation.Language.TokenKind]::Dot) {
            return $false
        }
        if (@($CommandAst.Redirections | Where-Object { $null -ne $_ }).Count -gt 0) {
            return $false
        }
        if ($null -eq $CommandAst.CommandElements -or $CommandAst.CommandElements.Count -ne 1) {
            return $false
        }
        $first = $CommandAst.CommandElements[0]
        if ($first -isnot [System.Management.Automation.Language.StringConstantExpressionAst]) {
            return $false
        }
        $pathText = [string]$first.Value
        if ([string]::IsNullOrWhiteSpace($pathText)) {
            return $false
        }
        $norm = ($pathText -replace '\\', '/').Trim()
        while ($norm.StartsWith('./', [System.StringComparison]::Ordinal)) {
            $norm = $norm.Substring(2)
        }
        # A suffix check would trust e.g. .\\evil\\scripts\\release-build\\ReleaseBuild.Common.ps1.
        # The controlled verifier may source exactly one repository-relative common script.
        return [string]::Equals($norm, $requiredDotSourceRelativePath, [System.StringComparison]::OrdinalIgnoreCase)
    }

    function Get-EvidenceDirArgExpression {
        param([Parameter(Mandatory = $true)]$CommandAst)
        $elements = @($CommandAst.CommandElements)
        # Whitelist the whole invocation rather than hunting only for dangerous
        # parameters. Command arguments are executable PowerShell expressions, so
        # an extra -Verbose:$(...) can mutate evidence or replace the command before
        # normal parameter binding happens.
        if ($elements.Count -ne 3) {
            return [pscustomobject]@{ Kind = 'invalid-grammar'; Expression = $null }
        }
        if (@($CommandAst.Redirections | Where-Object { $null -ne $_ }).Count -gt 0) {
            return [pscustomobject]@{ Kind = 'invalid-grammar'; Expression = $null }
        }
        $commandName = Get-CommandNameFromAstLocal -CommandAst $CommandAst
        if ($null -eq $commandName -or
            -not [string]::Equals($commandName, 'Assert-ReleaseEvidencePackage', [System.StringComparison]::OrdinalIgnoreCase)) {
            return [pscustomobject]@{ Kind = 'invalid-grammar'; Expression = $null }
        }
        $parameter = $elements[1]
        if ($parameter -isnot [System.Management.Automation.Language.CommandParameterAst] -or
            -not [string]::Equals([string]$parameter.ParameterName, 'EvidenceDir', [System.StringComparison]::OrdinalIgnoreCase) -or
            $null -ne $parameter.Argument) {
            return [pscustomobject]@{ Kind = 'invalid-grammar'; Expression = $null }
        }
        $arg = $elements[2]
        if ($arg -is [System.Management.Automation.Language.VariableExpressionAst]) {
            return [pscustomobject]@{
                Kind = 'variable'
                Expression = [string]$arg.VariablePath.UserPath
            }
        }
        if ($arg -is [System.Management.Automation.Language.StringConstantExpressionAst]) {
            return [pscustomobject]@{
                Kind = 'literal'
                Expression = [string]$arg.Value
            }
        }
        return [pscustomobject]@{
            Kind = 'unsupported'
            Expression = [string]$arg.Extent.Text
        }
    }

    function Test-CommandHasAllowDryRun {
        param([Parameter(Mandatory = $true)]$CommandAst)
        foreach ($el in @($CommandAst.CommandElements)) {
            if ($el -is [System.Management.Automation.Language.CommandParameterAst]) {
                $parameterName = [string]$el.ParameterName
                # PowerShell resolves unambiguous parameter prefixes at runtime, so
                # -AllowD is just as dangerous as the full -AllowDryRun switch here.
                if (-not [string]::IsNullOrWhiteSpace($parameterName) -and
                    'AllowDryRun'.StartsWith($parameterName, [System.StringComparison]::OrdinalIgnoreCase)) {
                    return $true
                }
            }
        }
        return $false
    }

    function Resolve-WantedCommandBinding {
        param(
            [Parameter(Mandatory = $true)]$CommandAst,
            [Parameter(Mandatory = $true)]$VarMap
        )
        $localAllow = Test-CommandHasAllowDryRun -CommandAst $CommandAst
        $argInfo = Get-EvidenceDirArgExpression -CommandAst $CommandAst
        $expr = $null
        $localErrs = New-Object System.Collections.Generic.List[string]
        if ($argInfo.Kind -eq 'literal') {
            $expr = [string]$argInfo.Expression
        } elseif ($argInfo.Kind -eq 'variable') {
            $vn = [string]$argInfo.Expression
            if ($VarMap.ContainsKey($vn)) {
                $expr = [string]$VarMap[$vn]
            } else {
                $localErrs.Add(("EvidenceDir variable `${0} is not bound to steps.evidence.outputs.dir before Assert" -f $vn)) | Out-Null
            }
        } elseif ($argInfo.Kind -eq 'invalid-grammar') {
            $localErrs.Add('Assert-ReleaseEvidencePackage must use the exact grammar: -EvidenceDir followed by one literal or simple variable argument') | Out-Null
        } else {
            $localErrs.Add('Assert-ReleaseEvidencePackage -EvidenceDir must be a constant or simple variable bound to steps.evidence.outputs.dir') | Out-Null
        }
        return [pscustomobject]@{
            EvidenceDirExpression = $expr
            AllowDryRun = $localAllow
            Errors = @($localErrs)
        }
    }

    $varBindings = @{}
    $requiredDotSourceCount = 0
    $dotSourceCount = 0
    $requiredDotSourceIndex = -1
    $sawWanted = $false
    $wantedStatementIndex = -1
    $allowDryRunSeen = $false
    $statementIndex = -1

    foreach ($stmt in @($root.Statements)) {
        if ($null -eq $stmt) { continue }
        $statementIndex += 1

        # The offline verifier must be the terminal executable operation in its
        # step. Even apparently benign logging can contain subexpressions which
        # mutate the already-verified evidence before the following upload.
        if ($sawWanted) {
            $errs.Add('verifier run must contain no statements after Assert-ReleaseEvidencePackage') | Out-Null
            continue
        }

        if ($stmt -is [System.Management.Automation.Language.AssignmentStatementAst]) {
            $leftName = $null
            $isSimpleLocalAssignmentTarget = $false
            if ($stmt.Left -is [System.Management.Automation.Language.VariableExpressionAst]) {
                $leftName = [string]$stmt.Left.VariablePath.UserPath
                $isSimpleLocalAssignmentTarget = ($leftName -match '^[A-Za-z_][A-Za-z0-9_]*$')
            }
            if ($null -ne $leftName -and -not $isSimpleLocalAssignmentTarget) {
                $errs.Add('verifier setup assignments must target an unqualified simple local variable (no provider, scope, or drive)') | Out-Null
            }
            $rhsLit = Get-StringConstantValue -Node $stmt.Right
            $isWantedAssignment = $false
            if ($isSimpleLocalAssignmentTarget -and
                $stmt.Operator -eq [System.Management.Automation.Language.TokenKind]::Equals -and
                $null -ne $rhsLit) {
                $varBindings[$leftName] = $rhsLit
            } elseif ($isSimpleLocalAssignmentTarget) {
                # A non-literal (or compound) rebind must invalidate a prior trusted
                # literal binding rather than leaving stale provenance in the map.
                [void]$varBindings.Remove($leftName)
            }

            $rhs = $stmt.Right
            if ($rhs -is [System.Management.Automation.Language.CommandExpressionAst] -and
                $rhs.Expression -is [System.Management.Automation.Language.PipelineAst]) {
                $rhs = $rhs.Expression
            }
            if ($rhs -is [System.Management.Automation.Language.PipelineAst]) {
                $els = @($rhs.PipelineElements)
                if ($els.Count -eq 1 -and $els[0] -is [System.Management.Automation.Language.CommandAst]) {
                    $cmd = $els[0]
                    $name = Get-CommandNameFromAstLocal -CommandAst $cmd
                    if ($stmt.Operator -eq [System.Management.Automation.Language.TokenKind]::Equals -and
                        $isSimpleLocalAssignmentTarget -and
                        [string]::Equals($leftName, 'result', [System.StringComparison]::OrdinalIgnoreCase) -and
                        $null -ne $name -and [string]::Equals($name, $wanted, [System.StringComparison]::OrdinalIgnoreCase)) {
                        $isWantedAssignment = $true
                        $sawWanted = $true
                        if ($wantedStatementIndex -lt 0) { $wantedStatementIndex = $statementIndex }
                        $binding = Resolve-WantedCommandBinding -CommandAst $cmd -VarMap $varBindings
                        if ($binding.AllowDryRun) { $allowDryRunSeen = $true }
                        if ($null -ne $binding.EvidenceDirExpression) { $evidenceExpr = [string]$binding.EvidenceDirExpression }
                        foreach ($be in @($binding.Errors)) { $errs.Add([string]$be) | Out-Null }
                    }
                }
            }
            if (-not $isWantedAssignment) {
                if ($null -eq $leftName) {
                    $errs.Add('verifier setup assignments must target a simple variable') | Out-Null
                } elseif (-not $isSimpleLocalAssignmentTarget) {
                    # The qualified target has already been reported above; never
                    # treat provider/scope state as a trusted variable binding.
                } elseif ($null -eq $rhsLit) {
                    $errs.Add('verifier setup assignments must be simple string literals; command-bearing or dynamic setup is not allowed') | Out-Null
                }
            }
            continue
        }

        if ($stmt -is [System.Management.Automation.Language.PipelineAst]) {
            $els = @($stmt.PipelineElements)
            if ($els.Count -ne 1 -or $els[0] -isnot [System.Management.Automation.Language.CommandAst]) {
                continue
            }
            $cmd = $els[0]
            if ($cmd.InvocationOperator -eq [System.Management.Automation.Language.TokenKind]::Dot) {
                $dotSourceCount += 1
                if (Test-IsRequiredCommonDotSource -CommandAst $cmd) {
                    $requiredDotSourceCount += 1
                    if ($requiredDotSourceIndex -lt 0) { $requiredDotSourceIndex = $statementIndex }
                } else {
                    $errs.Add('verifier run may dot-source only the exact scripts/release-build/ReleaseBuild.Common.ps1 path') | Out-Null
                }
                continue
            }
            $name = Get-CommandNameFromAstLocal -CommandAst $cmd
            if ($null -ne $name -and [string]::Equals($name, $wanted, [System.StringComparison]::OrdinalIgnoreCase)) {
                $sawWanted = $true
                if ($wantedStatementIndex -lt 0) { $wantedStatementIndex = $statementIndex }
                $binding = Resolve-WantedCommandBinding -CommandAst $cmd -VarMap $varBindings
                if ($binding.AllowDryRun) { $allowDryRunSeen = $true }
                if ($null -ne $binding.EvidenceDirExpression) { $evidenceExpr = [string]$binding.EvidenceDirExpression }
                foreach ($be in @($binding.Errors)) { $errs.Add([string]$be) | Out-Null }
                continue
            }
            continue
        }
    }

    if ($dotSourceCount -eq 0) {
        $errs.Add('verifier run must dot-source scripts/release-build/ReleaseBuild.Common.ps1 before Assert') | Out-Null
    } elseif ($dotSourceCount -ne 1 -or $requiredDotSourceCount -ne 1) {
        $errs.Add('verifier run must dot-source only one exact scripts/release-build/ReleaseBuild.Common.ps1 path') | Out-Null
    }
    if (-not $sawWanted) {
        $errs.Add('missing Assert-ReleaseEvidencePackage CommandAst after flat-script walk') | Out-Null
    } elseif ($requiredDotSourceIndex -lt 0 -or $requiredDotSourceIndex -ge $wantedStatementIndex) {
        $errs.Add('verifier run must dot-source scripts/release-build/ReleaseBuild.Common.ps1 before Assert') | Out-Null
    }
    if ($allowDryRunSeen) {
        $errs.Add('verifier run must not pass -AllowDryRun (host evidence is not dry-run)') | Out-Null
    }
    if ([string]::IsNullOrWhiteSpace($evidenceExpr)) {
        $errs.Add('verifier EvidenceDir binding could not be resolved') | Out-Null
    } elseif (-not [string]::Equals($evidenceExpr.Trim(), $requiredEvidenceLiteral, [System.StringComparison]::Ordinal)) {
        $errs.Add(("verifier EvidenceDir must be exactly '{0}' (got '{1}')" -f $requiredEvidenceLiteral, $evidenceExpr)) | Out-Null
    }

    return [pscustomobject]@{
        Valid = ($errs.Count -eq 0)
        Errors = @($errs)
        EvidenceDirExpression = $evidenceExpr
    }
}

function Get-ReleasePowerShellRunAst {
    <#
    .SYNOPSIS
    Parses a workflow run block as PowerShell without treating its text as evidence
    that a command actually executes.

    .DESCRIPTION
    A here-string, comment, or quoted literal may contain a command-looking string.
    Callers therefore consume CommandAst nodes from this helper rather than matching
    raw run text. Parse errors deliberately yield Valid=false.
    #>
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$ScriptText)

    $tokens = $null
    $parseErrors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseInput(
        $ScriptText,
        [ref]$tokens,
        [ref]$parseErrors
    )
    if ($null -eq $ast -or ($null -ne $parseErrors -and @($parseErrors).Count -gt 0)) {
        return [pscustomobject]@{ Valid = $false; Ast = $null; TopLevelCommands = @(); AllCommands = @() }
    }

    $topLevel = New-Object System.Collections.Generic.List[object]
    $endBlock = $ast.EndBlock
    if ($null -ne $endBlock -and $null -ne $endBlock.Statements) {
        foreach ($statement in @($endBlock.Statements)) {
            if ($statement -isnot [System.Management.Automation.Language.PipelineAst]) { continue }
            if (($statement.PSObject.Properties.Name -contains 'Background') -and [bool]$statement.Background) { continue }
            $elements = @($statement.PipelineElements)
            if ($elements.Count -eq 1 -and $elements[0] -is [System.Management.Automation.Language.CommandAst]) {
                $topLevel.Add($elements[0]) | Out-Null
            }
        }
    }
    $all = @($ast.FindAll({ param($node) $node -is [System.Management.Automation.Language.CommandAst] }, $true))
    $topLevelArray = [object[]]$topLevel.ToArray()
    $allArray = [object[]]$all
    return [pscustomobject]@{
        Valid = $true
        Ast = $ast
        TopLevelCommands = $topLevelArray
        AllCommands = $allArray
    }
}

function Test-ReleasePowerShellAstHasBareEndBlock {
    <#
    .SYNOPSIS
    Rejects parse-time and named-block preambles before evaluating a restricted
    workflow `run` grammar.

    .DESCRIPTION
    `using module`, `#requires`, and a param/begin/process/clean block can load
    or run code before an otherwise command-shaped gate or producer. A flat
    EndBlock command check alone therefore cannot prove that the checked-in
    command is the first executable behavior.
    #>
    param([Parameter(Mandatory = $true)]$Ast)

    if ($null -eq $Ast -or $null -eq $Ast.EndBlock) { return $false }
    if (($Ast.PSObject.Properties.Name -contains 'UsingStatements') -and
        @($Ast.UsingStatements | Where-Object { $null -ne $_ }).Count -gt 0) {
        return $false
    }
    if (($Ast.PSObject.Properties.Name -contains 'ScriptRequirements') -and $null -ne $Ast.ScriptRequirements) {
        return $false
    }
    foreach ($propertyName in @('ParamBlock', 'DynamicParamBlock', 'BeginBlock', 'ProcessBlock', 'CleanBlock')) {
        if (($Ast.PSObject.Properties.Name -contains $propertyName) -and $null -ne $Ast.$propertyName) {
            return $false
        }
    }
    $definitions = @($Ast.FindAll({
                param($node)
                $node -is [System.Management.Automation.Language.TypeDefinitionAst] -or
                [string]::Equals($node.GetType().Name, 'ConfigurationDefinitionAst', [System.StringComparison]::Ordinal)
            }, $true))
    return ($definitions.Count -eq 0)
}

function Get-ReleaseCommandFirstString {
    param([Parameter(Mandatory = $true)]$CommandAst)
    $elements = @($CommandAst.CommandElements)
    if ($elements.Count -eq 0) { return $null }
    if ($elements[0] -is [System.Management.Automation.Language.StringConstantExpressionAst]) {
        return [string]$elements[0].Value
    }
    return $null
}

function Test-ReleaseCommandHasParameter {
    param(
        [Parameter(Mandatory = $true)]$CommandAst,
        [Parameter(Mandatory = $true)][string]$ParameterName
    )
    foreach ($element in @($CommandAst.CommandElements)) {
        if ($element -is [System.Management.Automation.Language.CommandParameterAst] -and
            [string]::Equals([string]$element.ParameterName, $ParameterName, [System.StringComparison]::OrdinalIgnoreCase)) {
            return $true
        }
    }
    return $false
}

function Test-ReleaseCommandHasFileArgument {
    param(
        [Parameter(Mandatory = $true)]$CommandAst,
        [Parameter(Mandatory = $true)][string]$ExpectedRelativePath
    )
    $expected = ($ExpectedRelativePath -replace '\\', '/').Trim()
    $elements = @($CommandAst.CommandElements)
    for ($i = 0; $i -lt ($elements.Count - 1); $i += 1) {
        $parameter = $elements[$i]
        if ($parameter -isnot [System.Management.Automation.Language.CommandParameterAst] -or
            -not [string]::Equals([string]$parameter.ParameterName, 'File', [System.StringComparison]::OrdinalIgnoreCase)) {
            continue
        }
        $value = $elements[$i + 1]
        if ($value -isnot [System.Management.Automation.Language.StringConstantExpressionAst]) { continue }
        $actual = ([string]$value.Value -replace '\\', '/').Trim()
        while ($actual.StartsWith('./', [System.StringComparison]::Ordinal)) {
            $actual = $actual.Substring(2)
        }
        if ([string]::Equals($actual, $expected, [System.StringComparison]::OrdinalIgnoreCase)) {
            return $true
        }
    }
    return $false
}

function Test-ReleaseCommandHasBareSwitchParameter {
    <#
    .SYNOPSIS
    Requires exactly one bare switch parameter, not merely a parameter-shaped
    token whose explicit value can disable the requested behavior.
    #>
    param(
        [Parameter(Mandatory = $true)]$CommandAst,
        [Parameter(Mandatory = $true)][string]$ParameterName
    )

    $elements = @($CommandAst.CommandElements)
    $matches = @()
    for ($i = 0; $i -lt $elements.Count; $i += 1) {
        $element = $elements[$i]
        if ($element -is [System.Management.Automation.Language.CommandParameterAst] -and
            [string]::Equals([string]$element.ParameterName, $ParameterName, [System.StringComparison]::OrdinalIgnoreCase)) {
            $matches += [pscustomobject]@{ Parameter = $element; Index = $i }
        }
    }
    if ($matches.Count -ne 1) { return $false }

    $match = $matches[0]
    if (-not [string]::IsNullOrEmpty([string]$match.Parameter.Argument)) { return $false }
    # `-Switch $false` is also an explicit false switch value even though the
    # AST stores it as the next command element rather than Parameter.Argument.
    if (($match.Index + 1) -lt $elements.Count -and
        $elements[$match.Index + 1] -isnot [System.Management.Automation.Language.CommandParameterAst]) {
        return $false
    }
    return $true
}

function Test-ReleaseCommandHasExactVariableParameterValue {
    <#
    .SYNOPSIS
    Requires one `-Parameter $variable` pair with no inline/dynamic value.
    #>
    param(
        [Parameter(Mandatory = $true)]$CommandAst,
        [Parameter(Mandatory = $true)][string]$ParameterName,
        [Parameter(Mandatory = $true)][string]$VariableName
    )

    $elements = @($CommandAst.CommandElements)
    $parameterIndexes = @()
    for ($i = 0; $i -lt $elements.Count; $i += 1) {
        $element = $elements[$i]
        if ($element -is [System.Management.Automation.Language.CommandParameterAst] -and
            [string]::Equals([string]$element.ParameterName, $ParameterName, [System.StringComparison]::OrdinalIgnoreCase)) {
            $parameterIndexes += $i
        }
    }
    if ($parameterIndexes.Count -ne 1) { return $false }
    $index = [int]$parameterIndexes[0]
    $parameter = $elements[$index]
    if (-not [string]::IsNullOrEmpty([string]$parameter.Argument) -or ($index + 1) -ge $elements.Count) {
        return $false
    }
    $value = $elements[$index + 1]
    return ($value -is [System.Management.Automation.Language.VariableExpressionAst] -and
        [string]::Equals([string]$value.VariablePath.UserPath, $VariableName, [System.StringComparison]::OrdinalIgnoreCase))
}

function Test-ReleasePowerShellFileInvocationShape {
    <#
    .SYNOPSIS
    Verifies that a CommandAst really launches a fixed script via pwsh -File.

    .DESCRIPTION
    `pwsh -Command ... -File script.ps1` and `-EncodedCommand ... -File` are
    not trusted as file invocations: the earlier launcher mode can prevent the
    intended script from running. Required switches must be bare, so
    `-SecretScanOnly:$false` cannot masquerade as a scan.
    #>
    param(
        [Parameter(Mandatory = $true)]$CommandAst,
        [Parameter(Mandatory = $true)][string]$ExpectedRelativePath,
        [string]$RequiredBareSwitch,
        [switch]$RequireNoProfile
    )

    $first = Get-ReleaseCommandFirstString -CommandAst $CommandAst
    if ($null -eq $first -or
        -not (@('pwsh', 'pwsh.exe', 'powershell', 'powershell.exe') -contains $first.Trim().ToLowerInvariant())) {
        return $false
    }

    $elements = @($CommandAst.CommandElements)
    $fileIndexes = @()
    $noProfileIndexes = @()
    for ($i = 1; $i -lt $elements.Count; $i += 1) {
        $element = $elements[$i]
        if ($element -isnot [System.Management.Automation.Language.CommandParameterAst]) { continue }
        $name = ([string]$element.ParameterName).Trim().ToLowerInvariant()
        if ($name -eq 'noprofile') {
            if (-not [string]::IsNullOrEmpty([string]$element.Argument)) { return $false }
            $noProfileIndexes += $i
        }
        if ($name -eq 'file') { $fileIndexes += $i }
    }
    if ($fileIndexes.Count -ne 1) { return $false }

    $fileIndex = [int]$fileIndexes[0]
    # Treat the launcher prefix as a closed grammar. PowerShell accepts
    # abbreviated host switches (`-Co`, `-Enc`) and `-WorkingDirectory`; any
    # such token before -File can prevent the repository-relative script from
    # being the program that actually executes. Production only needs bare
    # -NoProfile before one literal -File.
    $preFileNoProfileCount = 0
    for ($i = 1; $i -lt $fileIndex; $i += 1) {
        $element = $elements[$i]
        if ($element -isnot [System.Management.Automation.Language.CommandParameterAst] -or
            -not [string]::Equals([string]$element.ParameterName, 'NoProfile', [System.StringComparison]::OrdinalIgnoreCase) -or
            -not [string]::IsNullOrEmpty([string]$element.Argument)) {
            return $false
        }
        $preFileNoProfileCount += 1
    }
    if ($RequireNoProfile -and $preFileNoProfileCount -ne 1) { return $false }
    $fileParameter = $elements[$fileIndex]
    if (-not [string]::IsNullOrEmpty([string]$fileParameter.Argument) -or ($fileIndex + 1) -ge $elements.Count) {
        return $false
    }
    $fileValue = $elements[$fileIndex + 1]
    if ($fileValue -isnot [System.Management.Automation.Language.StringConstantExpressionAst]) { return $false }
    $actualPath = ([string]$fileValue.Value -replace '\\', '/').Trim()
    while ($actualPath.StartsWith('./', [System.StringComparison]::Ordinal)) {
        $actualPath = $actualPath.Substring(2)
    }
    $expectedPath = ($ExpectedRelativePath -replace '\\', '/').Trim()
    if (-not [string]::Equals($actualPath, $expectedPath, [System.StringComparison]::OrdinalIgnoreCase)) {
        return $false
    }
    if (-not [string]::IsNullOrWhiteSpace($RequiredBareSwitch) -and
        -not (Test-ReleaseCommandHasBareSwitchParameter -CommandAst $CommandAst -ParameterName $RequiredBareSwitch)) {
        return $false
    }
    return $true
}

function Test-ReleasePowerShellFileInvocationExactArguments {
    <#
    .SYNOPSIS
    Requires a closed post-`-File` grammar for a repository-controlled host
    script invocation.

    .DESCRIPTION
    A correct `pwsh -NoProfile -File scripts/x.ps1` prefix is insufficient if a
    later workflow edit can append behavior-changing flags such as `-BuildApk`,
    `-DryRun`, or an alternate output path. This helper permits only named bare
    switches and named `$variable` values explicitly declared by the caller.
    #>
    param(
        [Parameter(Mandatory = $true)]$CommandAst,
        [Parameter(Mandatory = $true)][string]$ExpectedRelativePath,
        [string[]]$RequiredBareSwitches = @(),
        [string[]]$AllowedBareSwitches = @(),
        [hashtable]$RequiredVariableParameters = @{},
        [switch]$RequireNoProfile
    )

    if (-not (Test-ReleasePowerShellFileInvocationShape `
            -CommandAst $CommandAst `
            -ExpectedRelativePath $ExpectedRelativePath `
            -RequireNoProfile:$RequireNoProfile)) {
        return $false
    }

    $elements = @($CommandAst.CommandElements)
    $fileIndex = -1
    for ($i = 1; $i -lt $elements.Count; $i += 1) {
        $element = $elements[$i]
        if ($element -is [System.Management.Automation.Language.CommandParameterAst] -and
            [string]::Equals([string]$element.ParameterName, 'File', [System.StringComparison]::OrdinalIgnoreCase)) {
            $fileIndex = $i
            break
        }
    }
    if ($fileIndex -lt 0 -or ($fileIndex + 1) -ge $elements.Count) { return $false }

    $allowedBare = @{}
    foreach ($name in @($AllowedBareSwitches) + @($RequiredBareSwitches)) {
        if ([string]::IsNullOrWhiteSpace([string]$name)) { return $false }
        $allowedBare[([string]$name).Trim().ToLowerInvariant()] = $true
    }
    $requiredBare = @{}
    foreach ($name in @($RequiredBareSwitches)) {
        if ([string]::IsNullOrWhiteSpace([string]$name)) { return $false }
        $requiredBare[([string]$name).Trim().ToLowerInvariant()] = $true
    }
    $expectedVariables = @{}
    foreach ($key in @($RequiredVariableParameters.Keys)) {
        if ([string]::IsNullOrWhiteSpace([string]$key) -or
            [string]::IsNullOrWhiteSpace([string]$RequiredVariableParameters[$key])) {
            return $false
        }
        $expectedVariables[([string]$key).Trim().ToLowerInvariant()] = [string]$RequiredVariableParameters[$key]
    }

    $seenBare = @{}
    $seenVariables = @{}
    $i = $fileIndex + 2
    while ($i -lt $elements.Count) {
        $parameter = $elements[$i]
        if ($parameter -isnot [System.Management.Automation.Language.CommandParameterAst]) { return $false }
        $parameterName = ([string]$parameter.ParameterName).Trim().ToLowerInvariant()
        if ([string]::IsNullOrWhiteSpace($parameterName) -or -not [string]::IsNullOrEmpty([string]$parameter.Argument)) {
            return $false
        }

        if ($allowedBare.ContainsKey($parameterName)) {
            if ($seenBare.ContainsKey($parameterName)) { return $false }
            if (($i + 1) -lt $elements.Count -and
                $elements[$i + 1] -isnot [System.Management.Automation.Language.CommandParameterAst]) {
                return $false
            }
            $seenBare[$parameterName] = $true
            $i += 1
            continue
        }

        if ($expectedVariables.ContainsKey($parameterName)) {
            if ($seenVariables.ContainsKey($parameterName) -or ($i + 1) -ge $elements.Count) { return $false }
            $value = $elements[$i + 1]
            if ($value -isnot [System.Management.Automation.Language.VariableExpressionAst] -or
                -not [string]::Equals([string]$value.VariablePath.UserPath, [string]$expectedVariables[$parameterName], [System.StringComparison]::OrdinalIgnoreCase)) {
                return $false
            }
            $seenVariables[$parameterName] = $true
            $i += 2
            continue
        }
        return $false
    }

    foreach ($required in @($requiredBare.Keys)) {
        if (-not $seenBare.ContainsKey($required)) { return $false }
    }
    foreach ($required in @($expectedVariables.Keys)) {
        if (-not $seenVariables.ContainsKey($required)) { return $false }
    }
    return $true
}

function Get-ReleaseFlatSingleCommandAst {
    <#
    .SYNOPSIS
    Returns a command only when a workflow run body is one flat foreground
    command with no setup, fall-through, or exit-code-masking tail.
    #>
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$ScriptText)

    $parsed = Get-ReleasePowerShellRunAst -ScriptText $ScriptText
    if (-not $parsed.Valid -or $null -eq $parsed.Ast -or $null -eq $parsed.Ast.EndBlock) { return $null }
    if (-not (Test-ReleasePowerShellAstHasBareEndBlock -Ast $parsed.Ast)) { return $null }
    $statements = @($parsed.Ast.EndBlock.Statements)
    if ($statements.Count -ne 1 -or $statements[0] -isnot [System.Management.Automation.Language.PipelineAst]) {
        return $null
    }
    $pipeline = $statements[0]
    if (($pipeline.PSObject.Properties.Name -contains 'Background') -and [bool]$pipeline.Background) { return $null }
    $elements = @($pipeline.PipelineElements)
    if ($elements.Count -ne 1 -or $elements[0] -isnot [System.Management.Automation.Language.CommandAst]) { return $null }
    return $elements[0]
}

function Test-ReleaseExactNpmCiGateScript {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$ScriptText)

    $command = Get-ReleaseFlatSingleCommandAst -ScriptText $ScriptText
    if ($null -eq $command) { return $false }
    $elements = @($command.CommandElements)
    if ($elements.Count -ne 2 -or
        $elements[0] -isnot [System.Management.Automation.Language.StringConstantExpressionAst] -or
        $elements[1] -isnot [System.Management.Automation.Language.StringConstantExpressionAst]) {
        return $false
    }
    return ((@('npm', 'npm.cmd') -contains ([string]$elements[0].Value).Trim().ToLowerInvariant()) -and
        [string]::Equals([string]$elements[1].Value, 'ci', [System.StringComparison]::OrdinalIgnoreCase))
}

function Test-ReleaseExactSecretScanGateScript {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$ScriptText)

    $command = Get-ReleaseFlatSingleCommandAst -ScriptText $ScriptText
    if ($null -eq $command) { return $false }
    return Test-ReleasePowerShellFileInvocationExactArguments `
        -CommandAst $command `
        -ExpectedRelativePath 'scripts/verify-release.ps1' `
        -RequiredBareSwitches @('SecretScanOnly') `
        -RequireNoProfile
}

function Test-ReleaseRunContainsPowerShellFileInvocation {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$ScriptText,
        [Parameter(Mandatory = $true)][string]$ExpectedRelativePath,
        [string]$RequiredParameter,
        [switch]$TopLevelOnly
    )
    $parsed = Get-ReleasePowerShellRunAst -ScriptText $ScriptText
    if (-not $parsed.Valid) { return $false }
    $commands = if ($TopLevelOnly) { @($parsed.TopLevelCommands) } else { @($parsed.AllCommands) }
    foreach ($command in $commands) {
        if (Test-ReleasePowerShellFileInvocationShape `
            -CommandAst $command `
            -ExpectedRelativePath $ExpectedRelativePath `
            -RequiredBareSwitch $RequiredParameter `
            -RequireNoProfile) {
            return $true
        }
    }
    return $false
}

function Test-ReleaseRunContainsTopLevelNpmCommand {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$ScriptText,
        [Parameter(Mandatory = $true)][ValidateSet('ci', 'install')][string]$Subcommand
    )
    $parsed = Get-ReleasePowerShellRunAst -ScriptText $ScriptText
    if (-not $parsed.Valid) { return $false }
    foreach ($command in @($parsed.TopLevelCommands)) {
        $first = Get-ReleaseCommandFirstString -CommandAst $command
        if ($null -eq $first -or -not (@('npm', 'npm.cmd') -contains $first.Trim().ToLowerInvariant())) { continue }
        $elements = @($command.CommandElements)
        if ($elements.Count -lt 2 -or $elements[1] -isnot [System.Management.Automation.Language.StringConstantExpressionAst]) { continue }
        if ([string]::Equals([string]$elements[1].Value, $Subcommand, [System.StringComparison]::OrdinalIgnoreCase)) {
            return $true
        }
    }
    return $false
}

function Test-ReleaseWindowsHostOnlyProducerScriptContract {
    <#
    .SYNOPSIS
    Proves the Windows evidence producer takes the controlled host-only path by
    default and for tag runs.

    .DESCRIPTION
    A raw `-SkipBundle` substring is not enough: it can live in an unreachable
    branch while the default branch still builds a bundle. The controlled
    producer deliberately has one narrow shape:

      if ($skipBundleInput -eq 'false') {
          pwsh ... run-release-build.ps1 -OutputDir $evidenceDir
      } else {
          pwsh ... run-release-build.ps1 -SkipBundle -OutputDir $evidenceDir
      }

    `$skipBundleInput` is a literal data value loaded from a controlled step
    environment variable, never an expression interpolated into PowerShell
    source. Empty tag input and the workflow_dispatch default of `true`
    therefore take the else branch. This helper rejects a branch reversal, an
    arbitrary condition, switch arguments such as `-SkipBundle:$false`, extra
    build invocations, and nested/control-flow decoys.
    #>
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$ScriptText)

    $parsed = Get-ReleasePowerShellRunAst -ScriptText $ScriptText
    if (-not $parsed.Valid) { return $false }
    if (-not (Test-ReleasePowerShellAstHasBareEndBlock -Ast $parsed.Ast)) { return $false }

    function Test-IsControlledWindowsBuildCommand {
        param([Parameter(Mandatory = $true)]$CommandAst)
        return Test-ReleasePowerShellFileInvocationExactArguments `
            -CommandAst $CommandAst `
            -ExpectedRelativePath 'scripts/run-release-build.ps1' `
            -AllowedBareSwitches @('SkipBundle') `
            -RequiredVariableParameters @{ OutputDir = 'evidenceDir' } `
            -RequireNoProfile
    }

    function Test-CommandHasExactEvidenceOutputDir {
        param([Parameter(Mandatory = $true)]$CommandAst)
        return Test-ReleaseCommandHasExactVariableParameterValue `
            -CommandAst $CommandAst `
            -ParameterName 'OutputDir' `
            -VariableName 'evidenceDir'
    }

    function Test-CommandHasRequiredBareSkipBundle {
        param(
            [Parameter(Mandatory = $true)]$CommandAst,
            [Parameter(Mandatory = $true)][bool]$Required
        )
        $skipParameters = @($CommandAst.CommandElements | Where-Object {
                $_ -is [System.Management.Automation.Language.CommandParameterAst] -and
                [string]::Equals([string]$_.ParameterName, 'SkipBundle', [System.StringComparison]::OrdinalIgnoreCase)
            })
        if (-not $Required) { return ($skipParameters.Count -eq 0) }
        if ($skipParameters.Count -ne 1) { return $false }
        # Only a bare switch is safe. `-SkipBundle:$false` would syntactically
        # contain the parameter while disabling the host-only behavior.
        return [string]::IsNullOrEmpty([string]$skipParameters[0].Argument)
    }

    function Get-ExactBuildCommandFromBranch {
        param(
            [Parameter(Mandatory = $true)]$StatementBlock,
            [Parameter(Mandatory = $true)][bool]$RequireSkipBundle
        )
        if ($null -eq $StatementBlock) { return $null }
        $statements = @($StatementBlock.Statements)
        if ($statements.Count -ne 1) { return $null }
        $statement = $statements[0]
        if ($statement -isnot [System.Management.Automation.Language.PipelineAst] -or
            (($statement.PSObject.Properties.Name -contains 'Background') -and [bool]$statement.Background)) {
            return $null
        }
        $elements = @($statement.PipelineElements)
        if ($elements.Count -ne 1 -or $elements[0] -isnot [System.Management.Automation.Language.CommandAst]) {
            return $null
        }
        $command = $elements[0]
        $requiredBareSwitches = if ($RequireSkipBundle) { @('SkipBundle') } else { @() }
        $allowedBareSwitches = if ($RequireSkipBundle) { @('SkipBundle') } else { @() }
        if (-not (Test-ReleasePowerShellFileInvocationExactArguments `
                -CommandAst $command `
                -ExpectedRelativePath 'scripts/run-release-build.ps1' `
                -RequiredBareSwitches $requiredBareSwitches `
                -AllowedBareSwitches $allowedBareSwitches `
                -RequiredVariableParameters @{ OutputDir = 'evidenceDir' } `
                -RequireNoProfile)) {
            return $null
        }
        return $command
    }

    $controlledBuildCommands = @($parsed.AllCommands | Where-Object {
            Test-IsControlledWindowsBuildCommand -CommandAst $_
        })
    if ($controlledBuildCommands.Count -ne 2) { return $false }

    $candidateIfs = @($parsed.Ast.FindAll({
                param($node)
                if ($node -isnot [System.Management.Automation.Language.IfStatementAst]) { return $false }
                foreach ($command in $controlledBuildCommands) {
                    if ($command.Extent.StartOffset -ge $node.Extent.StartOffset -and
                        $command.Extent.EndOffset -le $node.Extent.EndOffset) {
                        return $true
                    }
                }
                return $false
            }, $true))
    if ($candidateIfs.Count -ne 1) { return $false }

    $candidate = $candidateIfs[0]
    if (@($candidate.Clauses).Count -ne 1 -or $null -eq $candidate.ElseClause) { return $false }
    $condition = ([string]$candidate.Clauses[0].Item1.Extent.Text -replace '\s+', '').ToLowerInvariant()
    $expectedCondition = '$skipbundleinput-eq''false'''
    if (-not [string]::Equals($condition, $expectedCondition, [System.StringComparison]::Ordinal)) { return $false }

    $bundleBranch = Get-ExactBuildCommandFromBranch -StatementBlock $candidate.Clauses[0].Item2 -RequireSkipBundle $false
    $hostOnlyBranch = Get-ExactBuildCommandFromBranch -StatementBlock $candidate.ElseClause -RequireSkipBundle $true
    if ($null -eq $bundleBranch -or $null -eq $hostOnlyBranch) { return $false }

    # Both and only both controlled invocations must belong to the recognized
    # branch pair; otherwise a decoy branch can coexist with an extra bundle run.
    foreach ($command in $controlledBuildCommands) {
        if ($command.Extent.StartOffset -ne $bundleBranch.Extent.StartOffset -and
            $command.Extent.StartOffset -ne $hostOnlyBranch.Extent.StartOffset) {
            return $false
        }
    }
    return $true
}

function Test-ReleaseEvidenceProducerScriptContract {
    <#
    .SYNOPSIS
    Verifies the complete controlled producer body, not just that it happens to
    mention a build helper.

    .DESCRIPTION
    The producer executes before the verifier and therefore cannot be allowed
    to run arbitrary setup commands that rewrite the dot-sourced helper or
    redirect evidence to a stale path. Its grammar is deliberately narrow:
    read-only Join-Path/Test-Path checks, a fixed pwsh -File producer using
    `-OutputDir $evidenceDir`, one controlled GUID-namespaced assignment, and
    one exact `dir=$evidenceDir` write to GITHUB_OUTPUT. Any other command,
    assignment, member invocation, rebind, or output mechanism fails closed.
    #>
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$ScriptText,
        [Parameter(Mandatory = $true)][ValidateSet('run-release-build.ps1', 'run-android-host-pipeline.ps1')][string]$ScriptLeafName,
        [switch]$RequireWindowsHostOnly
    )

    $parsed = Get-ReleasePowerShellRunAst -ScriptText $ScriptText
    if (-not $parsed.Valid -or $null -eq $parsed.Ast) { return $false }
    if (-not (Test-ReleasePowerShellAstHasBareEndBlock -Ast $parsed.Ast)) { return $false }
    $expectedRelativePath = "scripts/{0}" -f $ScriptLeafName
    $prefix = if ($ScriptLeafName -eq 'run-release-build.ps1') { 'windows' } else { 'android' }

    function Test-IsControlledProducerBuildCommand {
        param([Parameter(Mandatory = $true)]$CommandAst)
        $allowedBareSwitches = if ($RequireWindowsHostOnly) { @('SkipBundle') } else { @() }
        return Test-ReleasePowerShellFileInvocationExactArguments `
                -CommandAst $CommandAst `
                -ExpectedRelativePath $expectedRelativePath `
                -AllowedBareSwitches $allowedBareSwitches `
                -RequiredVariableParameters @{ OutputDir = 'evidenceDir' } `
                -RequireNoProfile
    }

    function Test-IsExactSingleThrowGuard {
        param(
            [Parameter(Mandatory = $true)]$IfAst,
            [Parameter(Mandatory = $true)][string]$ExpectedCondition
        )

        if ($IfAst -isnot [System.Management.Automation.Language.IfStatementAst] -or
            @($IfAst.Clauses).Count -ne 1 -or $null -ne $IfAst.ElseClause) {
            return $false
        }
        $condition = (([string]$IfAst.Clauses[0].Item1.Extent.Text -replace '\s+', '')).ToLowerInvariant()
        if (-not [string]::Equals($condition, $ExpectedCondition.ToLowerInvariant(), [System.StringComparison]::Ordinal)) {
            return $false
        }
        $body = @($IfAst.Clauses[0].Item2.Statements)
        return ($body.Count -eq 1 -and $body[0] -is [System.Management.Automation.Language.ThrowStatementAst])
    }

    $assignments = @($parsed.Ast.FindAll({
                param($node)
                $node -is [System.Management.Automation.Language.AssignmentStatementAst]
            }, $true))
    $expectedAssignmentCount = if ($RequireWindowsHostOnly) { 3 } else { 2 }
    if ($assignments.Count -ne $expectedAssignmentCount) { return $false }
    $seenErrorActionPreference = $false
    $evidenceAssignment = $null
    $skipBundleAssignment = $null
    $expectedEvidenceRhs = ('Join-Path$PWD("artifacts\release-build\' + $prefix + '-gitea-"+[guid]::NewGuid().ToString(''N''))').ToLowerInvariant()
    foreach ($assignment in $assignments) {
        if ($assignment.Operator -ne [System.Management.Automation.Language.TokenKind]::Equals -or
            $assignment.Left -isnot [System.Management.Automation.Language.VariableExpressionAst]) {
            return $false
        }
        $variableName = [string]$assignment.Left.VariablePath.UserPath
        if ([string]::Equals($variableName, 'ErrorActionPreference', [System.StringComparison]::OrdinalIgnoreCase)) {
            if ($seenErrorActionPreference -or -not [string]::Equals(([string]$assignment.Right.Extent.Text).Trim(), "'Stop'", [System.StringComparison]::Ordinal)) {
                return $false
            }
            $seenErrorActionPreference = $true
            continue
        }
        if ([string]::Equals($variableName, 'evidenceDir', [System.StringComparison]::OrdinalIgnoreCase)) {
            if ($null -ne $evidenceAssignment) { return $false }
            $actualRhs = (([string]$assignment.Right.Extent.Text -replace '\s+', '').Replace('/', '\')).ToLowerInvariant()
            if (-not [string]::Equals($actualRhs, $expectedEvidenceRhs, [System.StringComparison]::Ordinal)) {
                return $false
            }
            $evidenceAssignment = $assignment
            continue
        }
        if ([string]::Equals($variableName, 'skipBundleInput', [System.StringComparison]::OrdinalIgnoreCase)) {
            if (-not $RequireWindowsHostOnly -or $null -ne $skipBundleAssignment -or
                -not [string]::Equals((([string]$assignment.Right.Extent.Text -replace '\s+', '')).ToLowerInvariant(), '[string]$env:sf_release_skip_bundle_input', [System.StringComparison]::Ordinal)) {
                return $false
            }
            $skipBundleAssignment = $assignment
            continue
        }
        return $false
    }
    if (-not $seenErrorActionPreference -or $null -eq $evidenceAssignment -or
        ($RequireWindowsHostOnly -and $null -eq $skipBundleAssignment)) { return $false }

    $allowedCommands = @('join-path', 'test-path', 'out-file', 'pwsh', 'pwsh.exe', 'powershell', 'powershell.exe')
    $buildCommands = @()
    $outFileCommands = @()
    foreach ($command in @($parsed.AllCommands)) {
        $name = Get-ReleaseCommandFirstString -CommandAst $command
        if ($null -eq $name) { return $false }
        $normalizedName = $name.Trim().ToLowerInvariant()
        if ($allowedCommands -notcontains $normalizedName) { return $false }
        if (@('pwsh', 'pwsh.exe', 'powershell', 'powershell.exe') -contains $normalizedName) {
            if (-not (Test-IsControlledProducerBuildCommand -CommandAst $command)) { return $false }
            $buildCommands += $command
        }
        if ($normalizedName -eq 'out-file') { $outFileCommands += $command }
    }

    $expectedBuildCount = if ($RequireWindowsHostOnly) { 2 } else { 1 }
    if ($buildCommands.Count -ne $expectedBuildCount -or $outFileCommands.Count -ne 1) { return $false }
    $buildOffsets = @($buildCommands | ForEach-Object { [int]$_.Extent.StartOffset })
    $firstBuildOffset = [int](($buildOffsets | Measure-Object -Minimum).Minimum)
    $lastBuildOffset = [int](($buildOffsets | Measure-Object -Maximum).Maximum)
    if ($evidenceAssignment.Extent.StartOffset -ge $firstBuildOffset) {
        return $false
    }

    # The producer's observable side effects must have one fixed, fail-closed
    # sequence. `$ErrorActionPreference = 'Stop'` does not reliably turn a
    # native pwsh child-process non-zero exit into a terminating error, so the
    # explicit `$LASTEXITCODE` guard is mandatory. The fresh-directory and
    # manifest guards likewise prevent stale evidence reuse and false output
    # publication after a producer that did not yield a manifest.
    $rootStatements = @($parsed.Ast.EndBlock.Statements)
    if ($rootStatements.Count -lt 7 -or
        $rootStatements[0] -isnot [System.Management.Automation.Language.AssignmentStatementAst] -or
        $rootStatements[1] -isnot [System.Management.Automation.Language.AssignmentStatementAst]) {
        return $false
    }
    if ($rootStatements[0].Extent.StartOffset -ne $assignments[0].Extent.StartOffset -or
        $rootStatements[1].Extent.StartOffset -ne $evidenceAssignment.Extent.StartOffset) {
        return $false
    }
    if ($RequireWindowsHostOnly) {
        if ($rootStatements.Count -ne 9 -or
            $rootStatements[2] -isnot [System.Management.Automation.Language.AssignmentStatementAst] -or
            $rootStatements[3] -isnot [System.Management.Automation.Language.IfStatementAst] -or
            $rootStatements[4] -isnot [System.Management.Automation.Language.IfStatementAst] -or
            $rootStatements[5] -isnot [System.Management.Automation.Language.IfStatementAst] -or
            $rootStatements[6] -isnot [System.Management.Automation.Language.IfStatementAst] -or
            $rootStatements[7] -isnot [System.Management.Automation.Language.IfStatementAst]) {
            return $false
        }
        if ($rootStatements[2].Extent.StartOffset -ne $skipBundleAssignment.Extent.StartOffset -or
            -not (Test-IsExactSingleThrowGuard -IfAst $rootStatements[3] -ExpectedCondition "`$skipbundleinput-notin@('','true','false')") -or
            -not (Test-IsExactSingleThrowGuard -IfAst $rootStatements[4] -ExpectedCondition 'test-path-literalpath$evidencedir') -or
            -not (Test-IsExactSingleThrowGuard -IfAst $rootStatements[6] -ExpectedCondition '$lastexitcode-ne0') -or
            -not (Test-IsExactSingleThrowGuard -IfAst $rootStatements[7] -ExpectedCondition "-not(test-path-literalpath(join-path`$evidencedir'manifest.json')-pathtypeleaf)")) {
            return $false
        }
        if ($rootStatements[3].Extent.StartOffset -le $skipBundleAssignment.Extent.StartOffset -or
            $rootStatements[4].Extent.StartOffset -le $rootStatements[3].Extent.EndOffset -or
            $firstBuildOffset -le $rootStatements[4].Extent.EndOffset -or
            $rootStatements[6].Extent.StartOffset -le $lastBuildOffset -or
            $rootStatements[7].Extent.StartOffset -le $rootStatements[6].Extent.EndOffset) {
            return $false
        }
    } else {
        if ($rootStatements.Count -ne 7 -or
            $rootStatements[2] -isnot [System.Management.Automation.Language.IfStatementAst] -or
            $rootStatements[3] -isnot [System.Management.Automation.Language.PipelineAst] -or
            @($rootStatements[3].PipelineElements).Count -ne 1 -or
            $rootStatements[3].PipelineElements[0] -isnot [System.Management.Automation.Language.CommandAst] -or
            $rootStatements[4] -isnot [System.Management.Automation.Language.IfStatementAst] -or
            $rootStatements[5] -isnot [System.Management.Automation.Language.IfStatementAst] -or
            -not (Test-IsControlledProducerBuildCommand -CommandAst $rootStatements[3].PipelineElements[0]) -or
            -not (Test-IsExactSingleThrowGuard -IfAst $rootStatements[2] -ExpectedCondition 'test-path-literalpath$evidencedir') -or
            -not (Test-IsExactSingleThrowGuard -IfAst $rootStatements[4] -ExpectedCondition '$lastexitcode-ne0') -or
            -not (Test-IsExactSingleThrowGuard -IfAst $rootStatements[5] -ExpectedCondition "-not(test-path-literalpath(join-path`$evidencedir'manifest.json')-pathtypeleaf)")) {
            return $false
        }
        if ($firstBuildOffset -le $rootStatements[2].Extent.EndOffset -or
            $rootStatements[4].Extent.StartOffset -le $lastBuildOffset -or
            $rootStatements[5].Extent.StartOffset -le $rootStatements[4].Extent.EndOffset) {
            return $false
        }
    }
    $expectedThrowCount = if ($RequireWindowsHostOnly) { 4 } else { 3 }
    if (@($parsed.Ast.FindAll({
                    param($node)
                    $node -is [System.Management.Automation.Language.ThrowStatementAst]
                }, $true)).Count -ne $expectedThrowCount) {
        return $false
    }
    if (-not $RequireWindowsHostOnly) {
        $topLevelBuilds = @($parsed.TopLevelCommands | Where-Object { Test-IsControlledProducerBuildCommand -CommandAst $_ })
        if ($topLevelBuilds.Count -ne 1) { return $false }
    } elseif (-not (Test-ReleaseWindowsHostOnlyProducerScriptContract -ScriptText $ScriptText)) {
        return $false
    }

    # GUID construction is the only method invocation in a producer. Blocking
    # every other member call closes direct .NET file-write and reflection paths.
    foreach ($memberInvocation in @($parsed.Ast.FindAll({
                    param($node)
                    $node -is [System.Management.Automation.Language.InvokeMemberExpressionAst]
                }, $true))) {
        $normalizedInvocation = (([string]$memberInvocation.Extent.Text -replace '\s+', '')).ToLowerInvariant()
        if ($normalizedInvocation -notin @("[guid]::newguid()", "[guid]::newguid().tostring('n')")) { return $false }
    }
    if (@($parsed.Ast.FindAll({
                    param($node)
                    $node -is [System.Management.Automation.Language.ScriptBlockExpressionAst]
                }, $true)).Count -ne 0) {
        return $false
    }
    # A producer must not mask a failed build or make the required output write
    # unreachable. `throw` is intentionally allowed for fail-closed checks;
    # returns/exits/catches/loops are not part of the controlled grammar.
    if (@($parsed.Ast.FindAll({
                    param($node)
                    $node -is [System.Management.Automation.Language.ExitStatementAst] -or
                    $node -is [System.Management.Automation.Language.ReturnStatementAst] -or
                    $node -is [System.Management.Automation.Language.BreakStatementAst] -or
                    $node -is [System.Management.Automation.Language.ContinueStatementAst] -or
                    $node -is [System.Management.Automation.Language.TryStatementAst] -or
                    $node -is [System.Management.Automation.Language.TrapStatementAst] -or
                    $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -or
                    $node -is [System.Management.Automation.Language.DataStatementAst] -or
                    $node -is [System.Management.Automation.Language.ForStatementAst] -or
                    $node -is [System.Management.Automation.Language.ForEachStatementAst] -or
                    $node -is [System.Management.Automation.Language.WhileStatementAst] -or
                    $node -is [System.Management.Automation.Language.DoWhileStatementAst] -or
                    $node -is [System.Management.Automation.Language.DoUntilStatementAst] -or
                    $node -is [System.Management.Automation.Language.SwitchStatementAst]
                }, $true)).Count -ne 0) {
        return $false
    }

    $outputPipelines = @($rootStatements | Where-Object {
            if ($_ -isnot [System.Management.Automation.Language.PipelineAst]) { return $false }
            $elements = @($_.PipelineElements)
            if ($elements.Count -ne 2 -or
                $elements[0] -isnot [System.Management.Automation.Language.CommandExpressionAst] -or
                $elements[1] -isnot [System.Management.Automation.Language.CommandAst]) {
                return $false
            }
            $outName = Get-ReleaseCommandFirstString -CommandAst $elements[1]
            return ($null -ne $outName -and [string]::Equals($outName, 'Out-File', [System.StringComparison]::OrdinalIgnoreCase))
        })
    if ($outputPipelines.Count -ne 1) { return $false }
    $outputPipeline = $outputPipelines[0]
    $outputElements = @($outputPipeline.PipelineElements)
    if (-not [string]::Equals(([string]$outputElements[0].Extent.Text).Trim(), '"dir=$evidenceDir"', [System.StringComparison]::Ordinal) -or
        -not (Test-ReleaseCommandHasExactVariableParameterValue `
            -CommandAst $outputElements[1] `
            -ParameterName 'FilePath' `
            -VariableName 'env:GITHUB_OUTPUT') -or
        -not (Test-ReleaseCommandHasBareSwitchParameter -CommandAst $outputElements[1] -ParameterName 'Append')) {
        return $false
    }
    $manifestGuardIndex = if ($RequireWindowsHostOnly) { 7 } else { 5 }
    if ($outputPipeline.Extent.StartOffset -le $rootStatements[$manifestGuardIndex].Extent.EndOffset) {
        return $false
    }
    return $true
}

function Test-ReleaseHostEvidenceVerifierOrder {
    <#
    .SYNOPSIS
    Parses release-host-evidence.yml with a real YAML engine and checks that each
    host job has a controlled Assert-ReleaseEvidencePackage run step before upload.

    .DESCRIPTION
    Fail-closed on missing jobs, comment-only mentions, Write-Host/assignment
    string decoys, verifier-after-upload, wrong shell, continue-on-error, upload
    if:always(), -AllowDryRun, forged common.ps1 source, EvidenceDir/path mismatch,
    or absence of a real YAML parser. Structural fallback is never treated as PASS.
    Command recognition uses PowerShell AST CommandAst with flat-script reachability
    and full step-metadata binding.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$WorkflowPath,
        [switch]$RequireRetentionDays14,
        [switch]$RequireCheckout,
        [switch]$RequireFreshProducer
    )

    $requiredJobs = @('windows-host-evidence', 'android-host-evidence')
    $jobResults = @{}
    $unavailableJobResults = @{}
    foreach ($unavailableJobName in $requiredJobs) {
        $unavailableJobResults[$unavailableJobName] = [pscustomobject]@{
            Present = $false
            HasVerifierBeforeUpload = $false
            VerifierIndex = -1
            UploadIndex = -1
            CheckoutIndex = -1
            ProducerIndex = -1
            Reason = 'workflow metadata unavailable'
            ShellOk = $false
            ContinueOnErrorOk = $false
            JobContinueOnErrorOk = $false
            JobIfOk = $false
            JobNeedsOk = $false
            JobRunnerOk = $false
            VerifierWorkingDirectoryOk = $false
            CheckoutOk = $false
            ProducerOk = $false
            ProducerOutputIdOk = $false
                ProducerHostOnlyOk = $false
                RunTopologyOk = $false
                UploadIfOk = $false
            UploadContinueOnErrorOk = $false
            UploadActionOk = $false
            RetentionOk = $false
            ScriptContractOk = $false
            PathBindOk = $false
        }
    }
    $errors = New-Object System.Collections.Generic.List[string]
    $verifierCommand = 'Assert-ReleaseEvidencePackage'
    $requiredEvidenceLiteral = '${{ steps.evidence.outputs.dir }}'
    $allowedShells = @('pwsh', 'powershell')
    $requiredCheckoutAction = 'actions/checkout@v4'

    if (-not (Test-Path -LiteralPath $WorkflowPath -PathType Leaf)) {
        return [pscustomobject]@{
            Valid = $false
            Engine = 'none'
            Errors = @("Workflow file not found: $WorkflowPath")
            jobs = [hashtable]$unavailableJobResults
            ActionReferences = @()
            RunBlocks = @()
            RootPermissionsPresent = $false
            RootPermissionsKind = ''
            RootPermissions = [pscustomobject]@{}
            RootPermissionsKeysUnique = $false
            RootPermissionsRawKeys = @()
            JobPermissionRecords = @()
            SkipBundleDefaultPresent = $false
            SkipBundleDefaultKind = ''
            SkipBundleDefault = ''
        }
    }

    $pythonCmd = Get-Command python -ErrorAction SilentlyContinue
    if (-not $pythonCmd) { $pythonCmd = Get-Command python3 -ErrorAction SilentlyContinue }
    $nodeCmd = Get-Command node -ErrorAction SilentlyContinue

    # YAML engines extract full step metadata; command authenticity and controlled
    # binding are decided later in PowerShell (AST + fail-closed metadata rules).
    $pyScript = @'
import sys, json, re
try:
    import yaml
except ImportError:
    print(json.dumps({"ok": False, "engine": "python-missing-pyyaml", "error": "PyYAML is not installed", "jobs": {}}))
    sys.exit(0)

path = sys.argv[1]
class UniqueKeySafeLoader(yaml.SafeLoader):
    pass
def construct_unique_mapping(loader, node, deep=False):
    mapping = {}
    for key_node, value_node in node.value:
        if key_node.tag == "tag:yaml.org,2002:merge" or key_node.value == "<<":
            raise yaml.constructor.ConstructorError(
                "while constructing a mapping", node.start_mark,
                "YAML merge keys are not allowed in workflow files", key_node.start_mark
            )
        key = loader.construct_object(key_node, deep=deep)
        if key in mapping:
            raise yaml.constructor.ConstructorError(
                "while constructing a mapping", node.start_mark,
                "found duplicate key: {0}".format(key), key_node.start_mark
            )
        mapping[key] = loader.construct_object(value_node, deep=deep)
    return mapping
UniqueKeySafeLoader.add_constructor(
    yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG,
    construct_unique_mapping
)
try:
    with open(path, "r", encoding="utf-8") as f:
        source = f.read()
    def has_explicit_root_on_key(text):
        root_indent = None
        for raw_line in text.splitlines():
            line = raw_line.lstrip("\ufeff")
            stripped = line.strip()
            if not stripped or stripped.startswith("#") or stripped in ("---", "{", "}"):
                continue
            key_match = re.match(r"^([ \t]*)(?:\"[^\"\r\n]+\"|'[^'\r\n]+'|[A-Za-z_][A-Za-z0-9_-]*)[ \t]*:", line)
            if not key_match:
                continue
            if root_indent is None:
                root_indent = key_match.group(1)
            on_match = re.match(r"^([ \t]*)(?:on|[\"']on[\"'])[ \t]*:", line)
            if on_match and root_indent == on_match.group(1):
                return True
        return False
    # PyYAML parses YAML 1.1's unquoted `on` as Boolean True. Require the
    # source-level trigger spelling so a `true:` mapping key cannot impersonate
    # a runnable Gitea workflow.
    has_explicit_on_key = has_explicit_root_on_key(source)
    if re.search(r"(?m)^[ \t]*<<[ \t]*:", source):
        raise ValueError("YAML merge keys are not allowed in workflow files")
    if re.search(r"(?:^|[ \t,:\[{])(?:&|\*)[A-Za-z0-9_-]+", source):
        raise ValueError("YAML anchors and aliases are not allowed in workflow files")
    data = yaml.load(source, Loader=UniqueKeySafeLoader)
except Exception as exc:
    print(json.dumps({"ok": False, "engine": "pyyaml", "error": "YAML parse error: " + str(exc)[:500], "jobs": {}}))
    sys.exit(0)

if not isinstance(data, dict):
    print(json.dumps({"ok": False, "engine": "pyyaml", "error": "Workflow root must be a mapping", "jobs": {}}))
    sys.exit(0)

jobs = data.get("jobs")
if not isinstance(jobs, dict):
    print(json.dumps({"ok": False, "engine": "pyyaml", "error": "Top-level jobs must be a mapping", "jobs": {}}))
    sys.exit(0)

def scalar_kind(value):
    return (
        "mapping" if isinstance(value, dict) else
        "sequence" if isinstance(value, list) else
        "boolean" if isinstance(value, bool) else
        "string" if isinstance(value, str) else
        "null" if value is None else
        "number" if isinstance(value, (int, float)) else
        "other"
    )

def permissions_meta(owner):
    present = isinstance(owner, dict) and "permissions" in owner
    value = owner.get("permissions") if present else None
    if not isinstance(value, dict):
        return present, scalar_kind(value), {}, True, []
    raw_keys = [str(key) for key in value.keys()]
    normalized_keys = [key.lower() for key in raw_keys]
    entries = {}
    for key, raw in value.items():
        entries[str(key)] = raw if isinstance(raw, str) else ("" if raw is None else str(raw))
    return present, "mapping", entries, len(normalized_keys) == len(set(normalized_keys)), raw_keys

def run_working_directory_meta(owner):
    defaults = owner.get("defaults") if isinstance(owner, dict) else None
    run_defaults = defaults.get("run") if isinstance(defaults, dict) else None
    present = isinstance(run_defaults, dict) and "working-directory" in run_defaults
    value = run_defaults.get("working-directory") if present else None
    rendered = value if isinstance(value, str) else ("" if value is None else str(value))
    return present, scalar_kind(value), rendered

def run_shell_meta(owner):
    defaults = owner.get("defaults") if isinstance(owner, dict) else None
    run_defaults = defaults.get("run") if isinstance(defaults, dict) else None
    present = isinstance(run_defaults, dict) and "shell" in run_defaults
    value = run_defaults.get("shell") if present else None
    rendered = value if isinstance(value, str) else ("" if value is None else str(value))
    return present, scalar_kind(value), rendered

def env_meta(owner):
    present = isinstance(owner, dict) and "env" in owner
    value = owner.get("env") if present else None
    if not isinstance(value, dict):
        return present, scalar_kind(value), {}, True, []
    raw_keys = [str(key) for key in value.keys()]
    normalized_keys = [key.lower() for key in raw_keys]
    entries = {}
    for key, raw in value.items():
        entries[str(key)] = raw if isinstance(raw, str) else ("" if raw is None else str(raw))
    return present, "mapping", entries, len(normalized_keys) == len(set(normalized_keys)), raw_keys

def step_meta(i, step):
    uses = step.get("uses")
    run = step.get("run")
    uses_present = "uses" in step
    run_present = "run" in step
    shell = step.get("shell")
    shell_present = "shell" in step
    name = step.get("name")
    step_id = step.get("id")
    if_present = "if" in step
    if_expr = step.get("if") if if_present else None
    if_kind = scalar_kind(if_expr)
    coe_present = "continue-on-error" in step
    coe = step.get("continue-on-error") if coe_present else None
    coe_kind = scalar_kind(coe)
    working_directory_present = "working-directory" in step
    working_directory = step.get("working-directory") if working_directory_present else None
    working_directory_kind = scalar_kind(working_directory)
    env_present, env_kind, env_entries, env_keys_unique, env_raw_keys = env_meta(step)
    with_block = step.get("with")
    with_path = ""
    with_name = ""
    with_retention = None
    def with_scalar_meta(key):
        present = isinstance(with_block, dict) and key in with_block
        value = with_block.get(key) if present else None
        rendered = value if isinstance(value, str) else ("" if value is None else str(value))
        return present, scalar_kind(value), rendered
    if isinstance(with_block, dict):
        p = with_block.get("path")
        n = with_block.get("name")
        r = with_block.get("retention-days")
        with_path = p if isinstance(p, str) else ""
        with_name = n if isinstance(n, str) else ""
        if isinstance(r, bool):
            with_retention = None
        elif isinstance(r, int):
            with_retention = r
        elif isinstance(r, float) and r == int(r):
            with_retention = int(r)
        elif isinstance(r, str) and r.strip().isdigit():
            with_retention = int(r.strip())
    with_path_present, with_path_kind, with_path_scalar = with_scalar_meta("path")
    with_repository_present, with_repository_kind, with_repository = with_scalar_meta("repository")
    with_ref_present, with_ref_kind, with_ref = with_scalar_meta("ref")
    with_token_present, with_token_kind, with_token = with_scalar_meta("token")
    with_ssh_key_present, with_ssh_key_kind, with_ssh_key = with_scalar_meta("ssh-key")
    with_clean_present, with_clean_kind, with_clean = with_scalar_meta("clean")
    raw_with_keys = [str(key) for key in with_block.keys()] if isinstance(with_block, dict) else []
    normalized_with_keys = [key.lower() for key in raw_with_keys]
    return {
        "index": i,
        "name": name if isinstance(name, str) else "",
        "id": step_id if isinstance(step_id, str) else "",
        "id_present": "id" in step,
        "id_kind": scalar_kind(step_id),
        "uses": uses if isinstance(uses, str) else "",
        "uses_present": uses_present,
        "uses_kind": scalar_kind(uses),
        "run": run if isinstance(run, str) else "",
        "run_present": run_present,
        "run_kind": scalar_kind(run),
        "shell": shell if isinstance(shell, str) else "",
        "shell_present": shell_present,
        "shell_kind": scalar_kind(shell),
        "if": if_expr if isinstance(if_expr, str) else ("" if if_expr is None else str(if_expr)),
        "if_present": if_present,
        "if_kind": if_kind,
        "continue_on_error": True if coe is True else (False if coe is False else None),
        "continue_on_error_present": coe_present,
        "continue_on_error_kind": coe_kind,
        "working_directory": working_directory if isinstance(working_directory, str) else ("" if working_directory is None else str(working_directory)),
        "working_directory_present": working_directory_present,
        "working_directory_kind": working_directory_kind,
        "env_present": env_present,
        "env_kind": env_kind,
        "env": env_entries,
        "env_keys_unique": env_keys_unique,
        "env_raw_keys": env_raw_keys,
        "with_path": with_path,
        "with_path_scalar": with_path_scalar,
        "with_path_present": with_path_present,
        "with_path_kind": with_path_kind,
        "with_name": with_name,
        "with_retention_days": with_retention,
        "with_raw_keys": raw_with_keys,
        "with_keys": normalized_with_keys,
        "with_keys_unique": len(normalized_with_keys) == len(set(normalized_with_keys)),
        "with_repository": with_repository,
        "with_repository_present": with_repository_present,
        "with_repository_kind": with_repository_kind,
        "with_ref": with_ref,
        "with_ref_present": with_ref_present,
        "with_ref_kind": with_ref_kind,
        "with_token": with_token,
        "with_token_present": with_token_present,
        "with_token_kind": with_token_kind,
        "with_ssh_key": with_ssh_key,
        "with_ssh_key_present": with_ssh_key_present,
        "with_ssh_key_kind": with_ssh_key_kind,
        "with_clean": with_clean,
        "with_clean_present": with_clean_present,
        "with_clean_kind": with_clean_kind,
    }

workflow_wd_present, workflow_wd_kind, workflow_wd = run_working_directory_meta(data)
workflow_shell_present, workflow_shell_kind, workflow_shell = run_shell_meta(data)
workflow_env_present, workflow_env_kind, workflow_env, workflow_env_keys_unique, workflow_env_raw_keys = env_meta(data)
root_permissions_present, root_permissions_kind, root_permissions, root_permissions_keys_unique, root_permissions_raw_keys = permissions_meta(data)
out = {}
for name, job in jobs.items():
    name = str(name)
    if not isinstance(job, dict):
        out[name] = {"present": False, "steps": [], "reason": "job missing or not a mapping"}
        continue
    steps = job.get("steps")
    if not isinstance(steps, list):
        out[name] = {"present": True, "steps": [], "reason": "steps missing or not a list"}
        continue
    rendered = []
    for i, step in enumerate(steps):
        if not isinstance(step, dict):
            continue
        rendered.append(step_meta(i, step))
    job_wd_present, job_wd_kind, job_wd = run_working_directory_meta(job)
    job_shell_present, job_shell_kind, job_shell = run_shell_meta(job)
    job_env_present, job_env_kind, job_env, job_env_keys_unique, job_env_raw_keys = env_meta(job)
    job_coe_present = "continue-on-error" in job
    job_coe = job.get("continue-on-error") if job_coe_present else None
    job_if_present = "if" in job
    job_if = job.get("if") if job_if_present else None
    job_needs_present = "needs" in job
    job_container_present = "container" in job
    job_services_present = "services" in job
    job_runs_on = job.get("runs-on")
    job_permissions_present, job_permissions_kind, _, _, _ = permissions_meta(job)
    out[name] = {
        "present": True,
        "steps": rendered,
        "reason": "ok",
        "defaults_run_working_directory": job_wd,
        "defaults_run_working_directory_present": job_wd_present,
        "defaults_run_working_directory_kind": job_wd_kind,
        "defaults_run_shell": job_shell,
        "defaults_run_shell_present": job_shell_present,
        "defaults_run_shell_kind": job_shell_kind,
        "env_present": job_env_present,
        "env_kind": job_env_kind,
        "env": job_env,
        "env_keys_unique": job_env_keys_unique,
        "env_raw_keys": job_env_raw_keys,
        "continue_on_error": True if job_coe is True else (False if job_coe is False else None),
        "continue_on_error_present": job_coe_present,
        "continue_on_error_kind": scalar_kind(job_coe),
        "if": job_if if isinstance(job_if, str) else ("" if job_if is None else str(job_if)),
        "if_present": job_if_present,
        "if_kind": scalar_kind(job_if),
        "needs_present": job_needs_present,
        "container_present": job_container_present,
        "services_present": job_services_present,
        "runs_on": job_runs_on if isinstance(job_runs_on, str) else ("" if job_runs_on is None else str(job_runs_on)),
        "runs_on_kind": scalar_kind(job_runs_on),
        "permissions_present": job_permissions_present,
        "permissions_kind": job_permissions_kind,
    }

upload_artifact_references = []
action_references = []
run_blocks = []
job_permission_records = []
for job_name, job in jobs.items():
    if not isinstance(job, dict):
        continue
    job_permissions_present, job_permissions_kind, _, _, _ = permissions_meta(job)
    job_permission_records.append({"job": str(job_name), "present": job_permissions_present, "kind": job_permissions_kind})
    job_coe_present = "continue-on-error" in job
    job_coe = job.get("continue-on-error") if job_coe_present else None
    job_if_present = "if" in job
    job_if = job.get("if") if job_if_present else None
    job_needs_present = "needs" in job
    if "uses" in job:
        job_uses = job.get("uses")
        action_references.append({
            "job": str(job_name), "index": -1, "scope": "job",
            "uses": job_uses if isinstance(job_uses, str) else "",
            "uses_kind": scalar_kind(job_uses),
        })
    steps = job.get("steps")
    if not isinstance(steps, list):
        continue
    for index, step in enumerate(steps):
        if not isinstance(step, dict):
            continue
        uses = step.get("uses")
        run = step.get("run")
        step_record = step_meta(index, step)
        if "uses" in step:
            action_references.append({
                "job": str(job_name), "index": index, "scope": "step",
                "uses": uses if isinstance(uses, str) else "",
                "uses_kind": scalar_kind(uses),
            })
        if isinstance(run, str):
            run_blocks.append({
                "job": str(job_name), "index": index, "run": run,
                "shell": step_record["shell"], "shell_kind": step_record["shell_kind"],
                "if": step_record["if"], "if_present": step_record["if_present"], "if_kind": step_record["if_kind"],
                "continue_on_error": step_record["continue_on_error"],
                "continue_on_error_present": step_record["continue_on_error_present"],
                "continue_on_error_kind": step_record["continue_on_error_kind"],
                "job_if": job_if if isinstance(job_if, str) else ("" if job_if is None else str(job_if)),
                "job_if_present": job_if_present, "job_if_kind": scalar_kind(job_if),
                "job_continue_on_error": True if job_coe is True else (False if job_coe is False else None),
                "job_continue_on_error_present": job_coe_present,
                "job_continue_on_error_kind": scalar_kind(job_coe),
                "job_needs_present": job_needs_present,
            })
        if isinstance(uses, str) and uses.lower().startswith("actions/upload-artifact"):
            upload_artifact_references.append({"job": str(job_name), "index": index, "uses": uses})

workflow_on_present = has_explicit_on_key
workflow_on = (data.get("on") if "on" in data else data.get(True)) if has_explicit_on_key else None
workflow_triggers = [str(key) for key in workflow_on.keys()] if isinstance(workflow_on, dict) else []
skip_bundle_default_present = False
skip_bundle_default = None
if isinstance(workflow_on, dict):
    dispatch = workflow_on.get("workflow_dispatch")
    if isinstance(dispatch, dict):
        inputs = dispatch.get("inputs")
        if isinstance(inputs, dict):
            skip_bundle = inputs.get("skip_bundle")
            if isinstance(skip_bundle, dict) and "default" in skip_bundle:
                skip_bundle_default_present = True
                skip_bundle_default = skip_bundle.get("default")

print(json.dumps({
    "ok": True,
    "engine": "pyyaml",
    "error": "",
    "workflow_defaults_run_working_directory": workflow_wd,
    "workflow_defaults_run_working_directory_present": workflow_wd_present,
    "workflow_defaults_run_working_directory_kind": workflow_wd_kind,
    "workflow_defaults_run_shell": workflow_shell,
    "workflow_defaults_run_shell_present": workflow_shell_present,
    "workflow_defaults_run_shell_kind": workflow_shell_kind,
    "workflow_env_present": workflow_env_present,
    "workflow_env_kind": workflow_env_kind,
    "workflow_env": workflow_env,
    "workflow_env_keys_unique": workflow_env_keys_unique,
    "workflow_env_raw_keys": workflow_env_raw_keys,
    "workflow_on_present": workflow_on_present,
    "workflow_on_kind": scalar_kind(workflow_on),
    "workflow_triggers": workflow_triggers,
    "upload_artifact_references": upload_artifact_references,
    "action_references": action_references,
    "run_blocks": run_blocks,
    "root_permissions_present": root_permissions_present,
    "root_permissions_kind": root_permissions_kind,
    "root_permissions": root_permissions,
    "root_permissions_keys_unique": root_permissions_keys_unique,
    "root_permissions_raw_keys": root_permissions_raw_keys,
    "job_permission_records": job_permission_records,
    "skip_bundle_default_present": skip_bundle_default_present,
    "skip_bundle_default_kind": scalar_kind(skip_bundle_default),
    "skip_bundle_default": skip_bundle_default if isinstance(skip_bundle_default, str) else ("" if skip_bundle_default is None else str(skip_bundle_default)),
    "jobs": out,
}))
'@

    $jsScript = @'
const fs = require("fs");
const path = process.argv[2];
function emit(obj) { process.stdout.write(JSON.stringify(obj)); process.exit(0); }
function hasExplicitRootOnKey(text) {
  let rootIndent = null;
  for (let line of text.split(/\r?\n/)) {
    line = line.replace(/^\uFEFF/, "");
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#") || trimmed === "---" || trimmed === "{" || trimmed === "}") continue;
    const key = /^([ \t]*)(?:"[^"\r\n]+"|'[^'\r\n]+'|[A-Za-z_][A-Za-z0-9_-]*)[ \t]*:/.exec(line);
    if (!key) continue;
    if (rootIndent === null) rootIndent = key[1];
    const on = /^([ \t]*)(?:on|["']on["'])[ \t]*:/.exec(line);
    if (on && rootIndent === on[1]) return true;
  }
  return false;
}
let yaml;
try { yaml = require("yaml"); }
catch (_) {
  try { yaml = require("js-yaml"); }
  catch (e2) { emit({ ok: false, engine: "node-yaml", error: "Neither yaml nor js-yaml installed", jobs: {} }); }
}
let data;
let workflowSource = "";
try {
  const text = fs.readFileSync(path, "utf8");
  workflowSource = text;
  if (/(^|\n)[ \t]*<<[ \t]*:/.test(text)) throw new Error("YAML merge keys are not allowed in workflow files");
  if (/(^|[ \t,:\[{])(?:&|\*)[A-Za-z0-9_-]+/.test(text)) throw new Error("YAML anchors and aliases are not allowed in workflow files");
  data = yaml.load ? yaml.load(text) : yaml.parse(text);
} catch (e) {
  emit({ ok: false, engine: "node-yaml", error: "YAML parse error: " + String(e && e.message ? e.message : e).slice(0, 500), jobs: {} });
}
if (!data || typeof data !== "object" || Array.isArray(data)) {
  emit({ ok: false, engine: "node-yaml", error: "Workflow root must be a mapping", jobs: {} });
}
const jobs = data.jobs;
if (!jobs || typeof jobs !== "object" || Array.isArray(jobs)) {
  emit({ ok: false, engine: "node-yaml", error: "Top-level jobs must be a mapping", jobs: {} });
}
function scalarKind(value) {
  if (value === null) return "null";
  if (Array.isArray(value)) return "sequence";
  if (typeof value === "object") return "mapping";
  return typeof value;
}
function permissionsMeta(owner) {
  const present = !!owner && typeof owner === "object" && !Array.isArray(owner) && Object.prototype.hasOwnProperty.call(owner, "permissions");
  const raw = present ? owner.permissions : null;
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
    return { present: present, kind: scalarKind(raw), entries: {}, keysUnique: true, rawKeys: [] };
  }
  const rawKeys = Object.keys(raw).map((key) => String(key));
  const normalizedKeys = rawKeys.map((key) => key.toLowerCase());
  const entries = {};
  for (const [key, value] of Object.entries(raw)) {
    entries[String(key)] = typeof value === "string" ? value : (value == null ? "" : String(value));
  }
  return { present: present, kind: "mapping", entries: entries, keysUnique: normalizedKeys.length === new Set(normalizedKeys).size, rawKeys: rawKeys };
}
function runWorkingDirectoryMeta(owner) {
  const defaults = owner && typeof owner.defaults === "object" && !Array.isArray(owner.defaults) ? owner.defaults : null;
  const runDefaults = defaults && typeof defaults.run === "object" && !Array.isArray(defaults.run) ? defaults.run : null;
  const present = !!runDefaults && Object.prototype.hasOwnProperty.call(runDefaults, "working-directory");
  const raw = present ? runDefaults["working-directory"] : null;
  return {
    present: present,
    kind: scalarKind(raw),
    value: typeof raw === "string" ? raw : (raw == null ? "" : String(raw))
  };
}
function runShellMeta(owner) {
  const defaults = owner && typeof owner.defaults === "object" && !Array.isArray(owner.defaults) ? owner.defaults : null;
  const runDefaults = defaults && typeof defaults.run === "object" && !Array.isArray(defaults.run) ? defaults.run : null;
  const present = !!runDefaults && Object.prototype.hasOwnProperty.call(runDefaults, "shell");
  const raw = present ? runDefaults.shell : null;
  return {
    present: present,
    kind: scalarKind(raw),
    value: typeof raw === "string" ? raw : (raw == null ? "" : String(raw))
  };
}
function envMeta(owner) {
  const present = !!owner && typeof owner === "object" && !Array.isArray(owner) && Object.prototype.hasOwnProperty.call(owner, "env");
  const raw = present ? owner.env : null;
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
    return { present: present, kind: scalarKind(raw), entries: {}, keysUnique: true, rawKeys: [] };
  }
  const rawKeys = Object.keys(raw).map((key) => String(key));
  const normalizedKeys = rawKeys.map((key) => key.toLowerCase());
  const entries = {};
  for (const [key, value] of Object.entries(raw)) {
    entries[String(key)] = typeof value === "string" ? value : (value == null ? "" : String(value));
  }
  return { present: present, kind: "mapping", entries: entries, keysUnique: normalizedKeys.length === new Set(normalizedKeys).size, rawKeys: rawKeys };
}
function stepMeta(i, step) {
  const uses = typeof step.uses === "string" ? step.uses : "";
  const run = typeof step.run === "string" ? step.run : "";
  const shellRaw = step.shell;
  const shell = typeof shellRaw === "string" ? shellRaw : "";
  const name = typeof step.name === "string" ? step.name : "";
  const idRaw = step.id;
  const id = typeof idRaw === "string" ? idRaw : "";
  const hasOwn = (key) => Object.prototype.hasOwnProperty.call(step, key);
  const ifPresent = hasOwn("if");
  const ifRaw = ifPresent ? step.if : null;
  const ifKind = ifRaw === null ? "null" : typeof ifRaw;
  let ifExpr = "";
  if (typeof ifRaw === "string") ifExpr = ifRaw;
  else if (ifRaw != null) ifExpr = String(ifRaw);
  const coePresent = hasOwn("continue-on-error");
  const coeRaw = coePresent ? step["continue-on-error"] : null;
  const coeKind = coeRaw === null ? "null" : typeof coeRaw;
  let coe = null;
  if (coeRaw === true) coe = true;
  else if (coeRaw === false) coe = false;
  const workingDirectoryPresent = hasOwn("working-directory");
  const workingDirectoryRaw = workingDirectoryPresent ? step["working-directory"] : null;
  const workingDirectoryKind = scalarKind(workingDirectoryRaw);
  const stepEnv = envMeta(step);
  let withPath = "", withName = "", withRetention = null;
  if (step.with && typeof step.with === "object" && !Array.isArray(step.with)) {
    if (typeof step.with.path === "string") withPath = step.with.path;
    if (typeof step.with.name === "string") withName = step.with.name;
    const r = step.with["retention-days"];
    if (typeof r === "number" && Number.isFinite(r) && Number.isInteger(r)) withRetention = r;
    else if (typeof r === "string" && /^\d+$/.test(r.trim())) withRetention = parseInt(r.trim(), 10);
  }
  const withScalarMeta = (key) => {
    const present = !!step.with && typeof step.with === "object" && !Array.isArray(step.with) && Object.prototype.hasOwnProperty.call(step.with, key);
    const raw = present ? step.with[key] : null;
    return {
      present: present,
      kind: scalarKind(raw),
      value: typeof raw === "string" ? raw : (raw == null ? "" : String(raw))
    };
  };
  const withRepository = withScalarMeta("repository");
  const withRef = withScalarMeta("ref");
  const withToken = withScalarMeta("token");
  const withSshKey = withScalarMeta("ssh-key");
  const withClean = withScalarMeta("clean");
  const withPathScalar = withScalarMeta("path");
  const rawWithKeys = step.with && typeof step.with === "object" && !Array.isArray(step.with) ? Object.keys(step.with).map((key) => String(key)) : [];
  const normalizedWithKeys = rawWithKeys.map((key) => key.toLowerCase());
  return {
    index: i,
    name: name,
    id: id,
    id_present: hasOwn("id"),
    id_kind: scalarKind(idRaw),
    uses: uses,
    uses_present: hasOwn("uses"),
    uses_kind: scalarKind(step.uses),
    run: run,
    run_present: hasOwn("run"),
    run_kind: scalarKind(step.run),
    shell: shell,
    shell_present: hasOwn("shell"),
    shell_kind: scalarKind(shellRaw),
    if: ifExpr,
    if_present: ifPresent,
    if_kind: ifKind,
    continue_on_error: coe,
    continue_on_error_present: coePresent,
    continue_on_error_kind: coeKind,
    working_directory: typeof workingDirectoryRaw === "string" ? workingDirectoryRaw : (workingDirectoryRaw == null ? "" : String(workingDirectoryRaw)),
    working_directory_present: workingDirectoryPresent,
    working_directory_kind: workingDirectoryKind,
    env_present: stepEnv.present,
    env_kind: stepEnv.kind,
    env: stepEnv.entries,
    env_keys_unique: stepEnv.keysUnique,
    env_raw_keys: stepEnv.rawKeys,
    with_path: withPath,
    with_path_scalar: withPathScalar.value,
    with_path_present: withPathScalar.present,
    with_path_kind: withPathScalar.kind,
    with_name: withName,
    with_retention_days: withRetention,
    with_raw_keys: rawWithKeys,
    with_keys: normalizedWithKeys,
    with_keys_unique: normalizedWithKeys.length === new Set(normalizedWithKeys).size,
    with_repository: withRepository.value,
    with_repository_present: withRepository.present,
    with_repository_kind: withRepository.kind,
    with_ref: withRef.value,
    with_ref_present: withRef.present,
    with_ref_kind: withRef.kind,
    with_token: withToken.value,
    with_token_present: withToken.present,
    with_token_kind: withToken.kind,
    with_ssh_key: withSshKey.value,
    with_ssh_key_present: withSshKey.present,
    with_ssh_key_kind: withSshKey.kind,
    with_clean: withClean.value,
    with_clean_present: withClean.present,
    with_clean_kind: withClean.kind
  };
}
const workflowWorkingDirectory = runWorkingDirectoryMeta(data);
const workflowRunShell = runShellMeta(data);
const workflowEnv = envMeta(data);
const rootPermissions = permissionsMeta(data);
const out = {};
for (const [name, job] of Object.entries(jobs)) {
  if (!job || typeof job !== "object" || Array.isArray(job)) {
    out[name] = { present: false, steps: [], reason: "job missing or not a mapping" };
    continue;
  }
  const steps = job.steps;
  if (!Array.isArray(steps)) {
    out[name] = { present: true, steps: [], reason: "steps missing or not a list" };
    continue;
  }
  const rendered = [];
  for (let i = 0; i < steps.length; i++) {
    const step = steps[i];
    if (!step || typeof step !== "object") continue;
    rendered.push(stepMeta(i, step));
  }
  const jobWorkingDirectory = runWorkingDirectoryMeta(job);
  const jobRunShell = runShellMeta(job);
  const jobEnv = envMeta(job);
  const jobCoePresent = Object.prototype.hasOwnProperty.call(job, "continue-on-error");
  const jobCoeRaw = jobCoePresent ? job["continue-on-error"] : null;
  const jobIfPresent = Object.prototype.hasOwnProperty.call(job, "if");
  const jobIfRaw = jobIfPresent ? job.if : null;
  const jobNeedsPresent = Object.prototype.hasOwnProperty.call(job, "needs");
  const jobContainerPresent = Object.prototype.hasOwnProperty.call(job, "container");
  const jobServicesPresent = Object.prototype.hasOwnProperty.call(job, "services");
  const jobRunsOn = job["runs-on"];
  const jobPermissions = permissionsMeta(job);
  out[name] = {
    present: true,
    steps: rendered,
    reason: "ok",
    defaults_run_working_directory: jobWorkingDirectory.value,
    defaults_run_working_directory_present: jobWorkingDirectory.present,
    defaults_run_working_directory_kind: jobWorkingDirectory.kind,
    defaults_run_shell: jobRunShell.value,
    defaults_run_shell_present: jobRunShell.present,
    defaults_run_shell_kind: jobRunShell.kind,
    env_present: jobEnv.present,
    env_kind: jobEnv.kind,
    env: jobEnv.entries,
    env_keys_unique: jobEnv.keysUnique,
    env_raw_keys: jobEnv.rawKeys,
    continue_on_error: jobCoeRaw === true ? true : (jobCoeRaw === false ? false : null),
    continue_on_error_present: jobCoePresent,
    continue_on_error_kind: scalarKind(jobCoeRaw),
    if: typeof jobIfRaw === "string" ? jobIfRaw : (jobIfRaw == null ? "" : String(jobIfRaw)),
    if_present: jobIfPresent,
    if_kind: scalarKind(jobIfRaw),
    needs_present: jobNeedsPresent,
    container_present: jobContainerPresent,
    services_present: jobServicesPresent,
    runs_on: typeof jobRunsOn === "string" ? jobRunsOn : (jobRunsOn == null ? "" : String(jobRunsOn)),
    runs_on_kind: scalarKind(jobRunsOn),
    permissions_present: jobPermissions.present,
    permissions_kind: jobPermissions.kind
  };
}
const uploadArtifactReferences = [];
const actionReferences = [];
const runBlocks = [];
const jobPermissionRecords = [];
for (const [jobName, job] of Object.entries(jobs)) {
  if (!job || typeof job !== "object" || Array.isArray(job)) continue;
  const jobPermissions = permissionsMeta(job);
  jobPermissionRecords.push({ job: String(jobName), present: jobPermissions.present, kind: jobPermissions.kind });
  const jobCoePresent = Object.prototype.hasOwnProperty.call(job, "continue-on-error");
  const jobCoeRaw = jobCoePresent ? job["continue-on-error"] : null;
  const jobIfPresent = Object.prototype.hasOwnProperty.call(job, "if");
  const jobIfRaw = jobIfPresent ? job.if : null;
  const jobNeedsPresent = Object.prototype.hasOwnProperty.call(job, "needs");
  if (Object.prototype.hasOwnProperty.call(job, "uses")) {
    const jobUses = job.uses;
    actionReferences.push({
      job: String(jobName), index: -1, scope: "job",
      uses: typeof jobUses === "string" ? jobUses : "", uses_kind: scalarKind(jobUses)
    });
  }
  if (!Array.isArray(job.steps)) continue;
  for (let index = 0; index < job.steps.length; index++) {
    const step = job.steps[index];
    if (!step || typeof step !== "object" || Array.isArray(step)) continue;
    const stepRecord = stepMeta(index, step);
    if (Object.prototype.hasOwnProperty.call(step, "uses")) {
      actionReferences.push({
        job: String(jobName), index, scope: "step",
        uses: typeof step.uses === "string" ? step.uses : "", uses_kind: scalarKind(step.uses)
      });
    }
    if (typeof step.run === "string") {
      runBlocks.push({
        job: String(jobName), index, run: step.run,
        shell: stepRecord.shell, shell_kind: stepRecord.shell_kind,
        if: stepRecord.if, if_present: stepRecord.if_present, if_kind: stepRecord.if_kind,
        continue_on_error: stepRecord.continue_on_error,
        continue_on_error_present: stepRecord.continue_on_error_present,
        continue_on_error_kind: stepRecord.continue_on_error_kind,
        job_if: typeof jobIfRaw === "string" ? jobIfRaw : (jobIfRaw == null ? "" : String(jobIfRaw)),
        job_if_present: jobIfPresent, job_if_kind: scalarKind(jobIfRaw),
        job_continue_on_error: jobCoeRaw === true ? true : (jobCoeRaw === false ? false : null),
        job_continue_on_error_present: jobCoePresent,
        job_continue_on_error_kind: scalarKind(jobCoeRaw),
        job_needs_present: jobNeedsPresent
      });
    }
    if (typeof step.uses === "string" && step.uses.toLowerCase().startsWith("actions/upload-artifact")) {
      uploadArtifactReferences.push({ job: String(jobName), index: index, uses: step.uses });
    }
  }
}
// Node YAML implementations and PyYAML do not share YAML 1.1 key coercion.
// Bind trigger presence to the literal root source key, not a parsed Boolean
// key such as `true:`.
const workflowOnPresent = hasExplicitRootOnKey(workflowSource);
const workflowOn = workflowOnPresent
  ? (Object.prototype.hasOwnProperty.call(data, "on") ? data.on
    : (Object.prototype.hasOwnProperty.call(data, "true") ? data.true : null))
  : null;
const workflowTriggers = workflowOn && typeof workflowOn === "object" && !Array.isArray(workflowOn) ? Object.keys(workflowOn).map((key) => String(key)) : [];
let skipBundleDefaultPresent = false, skipBundleDefault = null;
if (workflowOn && typeof workflowOn === "object" && !Array.isArray(workflowOn)) {
  const dispatch = workflowOn.workflow_dispatch;
  if (dispatch && typeof dispatch === "object" && !Array.isArray(dispatch) && dispatch.inputs && typeof dispatch.inputs === "object" && !Array.isArray(dispatch.inputs)) {
    const skipBundle = dispatch.inputs.skip_bundle;
    if (skipBundle && typeof skipBundle === "object" && !Array.isArray(skipBundle) && Object.prototype.hasOwnProperty.call(skipBundle, "default")) {
      skipBundleDefaultPresent = true;
      skipBundleDefault = skipBundle.default;
    }
  }
}
emit({
  ok: true,
  engine: "node-yaml",
  error: "",
  workflow_defaults_run_working_directory: workflowWorkingDirectory.value,
  workflow_defaults_run_working_directory_present: workflowWorkingDirectory.present,
  workflow_defaults_run_working_directory_kind: workflowWorkingDirectory.kind,
  workflow_defaults_run_shell: workflowRunShell.value,
  workflow_defaults_run_shell_present: workflowRunShell.present,
  workflow_defaults_run_shell_kind: workflowRunShell.kind,
  workflow_env_present: workflowEnv.present,
  workflow_env_kind: workflowEnv.kind,
  workflow_env: workflowEnv.entries,
  workflow_env_keys_unique: workflowEnv.keysUnique,
  workflow_env_raw_keys: workflowEnv.rawKeys,
  workflow_on_present: workflowOnPresent,
  workflow_on_kind: scalarKind(workflowOn),
  workflow_triggers: workflowTriggers,
  upload_artifact_references: uploadArtifactReferences,
  action_references: actionReferences,
  run_blocks: runBlocks,
  root_permissions_present: rootPermissions.present,
  root_permissions_kind: rootPermissions.kind,
  root_permissions: rootPermissions.entries,
  root_permissions_keys_unique: rootPermissions.keysUnique,
  root_permissions_raw_keys: rootPermissions.rawKeys,
  job_permission_records: jobPermissionRecords,
  skip_bundle_default_present: skipBundleDefaultPresent,
  skip_bundle_default_kind: scalarKind(skipBundleDefault),
  skip_bundle_default: typeof skipBundleDefault === "string" ? skipBundleDefault : (skipBundleDefault == null ? "" : String(skipBundleDefault)),
  jobs: out
});
'@

    $parsed = $null
    $engine = 'none'
    if ($pythonCmd) {
        $tmpPy = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-host-order-{0}.py" -f [guid]::NewGuid().ToString('N'))
        try {
            $utf8NoBom = New-Object System.Text.UTF8Encoding $false
            [System.IO.File]::WriteAllText($tmpPy, $pyScript, $utf8NoBom)
            $prevEap = $ErrorActionPreference
            $ErrorActionPreference = 'Continue'
            try {
                $raw = & $pythonCmd.Source $tmpPy $WorkflowPath 2>&1
                $code = $LASTEXITCODE
            } finally {
                $ErrorActionPreference = $prevEap
            }
            $line = @($raw | Where-Object { "$_" -match '^\{' })[-1]
            if ($code -eq 0 -and $line) {
                $candidate = $line | ConvertFrom-Json
                if ([string]$candidate.engine -ne 'python-missing-pyyaml') {
                    $parsed = $candidate
                    $engine = [string]$candidate.engine
                }
            }
        } finally {
            Remove-Item -LiteralPath $tmpPy -Force -ErrorAction SilentlyContinue
        }
    }

    if ($null -eq $parsed -and $nodeCmd) {
        $tmpJs = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-host-order-{0}.js" -f [guid]::NewGuid().ToString('N'))
        try {
            $utf8NoBom = New-Object System.Text.UTF8Encoding $false
            [System.IO.File]::WriteAllText($tmpJs, $jsScript, $utf8NoBom)
            $prevEap = $ErrorActionPreference
            $ErrorActionPreference = 'Continue'
            try {
                $raw = & $nodeCmd.Source $tmpJs $WorkflowPath 2>&1
                $code = $LASTEXITCODE
            } finally {
                $ErrorActionPreference = $prevEap
            }
            $line = @($raw | Where-Object { "$_" -match '^\{' })[-1]
            if ($code -eq 0 -and $line) {
                $parsed = $line | ConvertFrom-Json
                $engine = [string]$parsed.engine
            }
        } finally {
            Remove-Item -LiteralPath $tmpJs -Force -ErrorAction SilentlyContinue
        }
    }

    if ($null -eq $parsed) {
        return [pscustomobject]@{
            Valid = $false
            Engine = 'none'
            Errors = @('No real YAML parser available for host evidence verifier order (need python+PyYAML or node with yaml/js-yaml). Structural-only checks are not accepted.')
            jobs = [hashtable]$unavailableJobResults
            ActionReferences = @()
            RunBlocks = @()
            RootPermissionsPresent = $false
            RootPermissionsKind = ''
            RootPermissions = [pscustomobject]@{}
            RootPermissionsKeysUnique = $false
            RootPermissionsRawKeys = @()
            JobPermissionRecords = @()
            SkipBundleDefaultPresent = $false
            SkipBundleDefaultKind = ''
            SkipBundleDefault = ''
        }
    }

    if (-not [bool]$parsed.ok) {
        $msg = if ($parsed.error) { [string]$parsed.error } else { 'host evidence verifier order parse failed' }
        return [pscustomobject]@{
            Valid = $false
            Engine = $engine
            Errors = @($msg)
            jobs = [hashtable]$unavailableJobResults
            ActionReferences = @()
            RunBlocks = @()
            RootPermissionsPresent = $false
            RootPermissionsKind = ''
            RootPermissions = [pscustomobject]@{}
            RootPermissionsKeysUnique = $false
            RootPermissionsRawKeys = @()
            JobPermissionRecords = @()
            SkipBundleDefaultPresent = $false
            SkipBundleDefaultKind = ''
            SkipBundleDefault = ''
        }
    }

    function Get-StepProp {
        param($Step, [string]$Name, $Default = '')
        if ($null -eq $Step) { return $Default }
        if ($Step.PSObject.Properties.Name -contains $Name) {
            return $Step.$Name
        }
        return $Default
    }

    function Test-StepHasNoCondition {
        param($Step)
        $present = [bool](Get-StepProp -Step $Step -Name 'if_present' -Default $false)
        # Controlled evidence steps need the field truly absent. An explicit empty
        # scalar is parser/runner-version sensitive and must not be treated as a
        # portable unconditional execution guarantee.
        return (-not $present)
    }

    function Test-ContinueOnErrorDisabled {
        param(
            [bool]$Present,
            [AllowEmptyString()][string]$Kind,
            $Value
        )
        # Controlled evidence and CI gates need the field absent, not merely a
        # parser-specific value that happens to coerce to false (YAML 1.1 `off` /
        # `no` differ from YAML 1.2). The production workflows do not need this
        # tolerance, so reject every explicit spelling.
        return (-not $Present)
    }

    function Test-StepContinueOnErrorDisabled {
        param($Step)
        return Test-ContinueOnErrorDisabled `
            -Present ([bool](Get-StepProp -Step $Step -Name 'continue_on_error_present' -Default $false)) `
            -Kind ([string](Get-StepProp -Step $Step -Name 'continue_on_error_kind' -Default '')) `
            -Value (Get-StepProp -Step $Step -Name 'continue_on_error' -Default $null)
    }

    function Test-RunWorkingDirectoryIsRepositoryDefault {
        param(
            [bool]$Present,
            [AllowEmptyString()][string]$Kind,
            [AllowEmptyString()][string]$Value
        )
        if (-not $Present) { return $true }
        # Relative dot-sourcing is trusted only from the checkout root. An explicit
        # empty scalar has the same runtime meaning as the default; every non-empty
        # or non-string override can redirect the common helper to an attacker path.
        return ($Kind -eq 'string' -and [string]::IsNullOrWhiteSpace($Value))
    }

    function Test-RunShellIsAbsent {
        param([bool]$Present)
        # A workflow/job default shell can wrap a command-shaped gate and mask
        # its exit code. Governed steps either declare their required shell
        # directly or inherit the platform default, never a defaults.run.shell.
        return (-not $Present)
    }

    function Test-StepHasNoEnvironment {
        param($Step)
        # Metadata omission is not a safe default: a parser adapter must prove
        # env absence before a trusted command name such as pwsh/npm is used.
        return (-not [bool](Get-StepProp -Step $Step -Name 'env_present' -Default $true))
    }

    function Test-HostStepEnvironmentContract {
        param(
            [Parameter(Mandatory = $true)][string]$JobName,
            [Parameter(Mandatory = $true)]$Step,
            [int]$ProducerIndex
        )

        if (-not [bool](Get-StepProp -Step $Step -Name 'env_present' -Default $false)) {
            return $true
        }
        # The sole controlled exception turns workflow_dispatch input into an
        # inert environment value for the Windows producer. Everything else
        # remains env-free so PATH/shell/tool identity cannot be shadowed.
        $index = [int](Get-StepProp -Step $Step -Name 'index' -Default -1)
        if ($JobName -ne 'windows-host-evidence' -or $index -ne $ProducerIndex -or
            -not [string]::Equals(([string](Get-StepProp -Step $Step -Name 'env_kind' -Default '')).Trim(), 'mapping', [System.StringComparison]::Ordinal) -or
            -not [bool](Get-StepProp -Step $Step -Name 'env_keys_unique' -Default $false)) {
            return $false
        }
        $rawKeys = @((Get-StepProp -Step $Step -Name 'env_raw_keys' -Default @()) | ForEach-Object { ([string]$_).Trim() })
        if (@($rawKeys).Count -ne 1 -or -not [string]::Equals(@($rawKeys)[0], 'SF_RELEASE_SKIP_BUNDLE_INPUT', [System.StringComparison]::Ordinal)) {
            return $false
        }
        $entries = Get-StepProp -Step $Step -Name 'env' -Default $null
        $value = $null
        if ($entries -is [System.Collections.IDictionary]) {
            $value = $entries['SF_RELEASE_SKIP_BUNDLE_INPUT']
        } elseif ($null -ne $entries -and $entries.PSObject.Properties.Name -contains 'SF_RELEASE_SKIP_BUNDLE_INPUT') {
            $value = $entries.PSObject.Properties['SF_RELEASE_SKIP_BUNDLE_INPUT'].Value
        } else {
            return $false
        }
        return [string]::Equals([string]$value, '${{ github.event.inputs.skip_bundle }}', [System.StringComparison]::Ordinal)
    }

    function Test-HostStepExecutionShape {
        param($Step)
        # `uses` and `run` are mutually exclusive workflow-step execution
        # modes. Do not assume a particular Gitea/act implementation will run
        # the command when both fields appear: accepting the command text would
        # turn a future ambiguous YAML edit into a false-green evidence claim.
        $usesPresent = [bool](Get-StepProp -Step $Step -Name 'uses_present' -Default $false)
        $runPresent = [bool](Get-StepProp -Step $Step -Name 'run_present' -Default $false)
        if ($usesPresent -eq $runPresent) { return $false }
        if ($usesPresent) {
            return ([string]::Equals(([string](Get-StepProp -Step $Step -Name 'uses_kind' -Default '')).Trim(), 'string', [System.StringComparison]::Ordinal) -and
                -not [string]::IsNullOrWhiteSpace([string](Get-StepProp -Step $Step -Name 'uses' -Default '')))
        }
        return ([string]::Equals(([string](Get-StepProp -Step $Step -Name 'run_kind' -Default '')).Trim(), 'string', [System.StringComparison]::Ordinal) -and
            -not [string]::IsNullOrWhiteSpace([string](Get-StepProp -Step $Step -Name 'run' -Default '')))
    }

    function Test-HostJobEnvironmentContract {
        param($Job)
        if (-not [bool](Get-StepProp -Step $Job -Name 'env_present' -Default $false)) {
            return $true
        }
        if (-not [string]::Equals([string](Get-StepProp -Step $Job -Name 'env_kind' -Default ''), 'mapping', [System.StringComparison]::Ordinal) -or
            -not [bool](Get-StepProp -Step $Job -Name 'env_keys_unique' -Default $false)) {
            return $false
        }
        $expected = [ordered]@{
            CARGO_TARGET_DIR = ''
            RUST_LOG          = 'info'
        }
        $rawKeys = @((Get-StepProp -Step $Job -Name 'env_raw_keys' -Default @()) | ForEach-Object { ([string]$_).Trim() })
        if ($rawKeys.Count -ne $expected.Count) { return $false }
        foreach ($expectedKey in $expected.Keys) {
            if (@($rawKeys | Where-Object {
                        [string]::Equals($_, [string]$expectedKey, [System.StringComparison]::Ordinal)
                    }).Count -ne 1) {
                return $false
            }
        }
        $entries = Get-StepProp -Step $Job -Name 'env' -Default $null
        foreach ($expectedKey in $expected.Keys) {
            $value = $null
            if ($entries -is [System.Collections.IDictionary]) {
                $value = $entries[$expectedKey]
            } elseif ($null -ne $entries -and $entries.PSObject.Properties.Name -contains $expectedKey) {
                $value = $entries.PSObject.Properties[$expectedKey].Value
            } else {
                return $false
            }
            if (-not [string]::Equals([string]$value, [string]$expected[$expectedKey], [System.StringComparison]::Ordinal)) {
                return $false
            }
        }
        return $true
    }

    function Test-UploadArtifactActionReference {
        param([AllowEmptyString()][string]$Uses)
        if ([string]::IsNullOrWhiteSpace($Uses)) { return $false }
        return $Uses.Trim().StartsWith('actions/upload-artifact', [System.StringComparison]::OrdinalIgnoreCase)
    }

    function Test-UsesPinnedUploadArtifactV4 {
        param([AllowEmptyString()][string]$Uses)
        return [string]::Equals($Uses.Trim(), 'actions/upload-artifact@v4', [System.StringComparison]::OrdinalIgnoreCase)
    }

    function Test-UsesPinnedCheckoutV4 {
        param([AllowEmptyString()][string]$Uses)
        return [string]::Equals($Uses.Trim(), $requiredCheckoutAction, [System.StringComparison]::OrdinalIgnoreCase)
    }

    function Test-CheckoutActionReference {
        param([AllowEmptyString()][string]$Uses)
        if ([string]::IsNullOrWhiteSpace($Uses)) { return $false }
        return $Uses.Trim().StartsWith('actions/checkout', [System.StringComparison]::OrdinalIgnoreCase)
    }

    function Test-RunReferencesControlledScript {
        param(
            [AllowEmptyString()][string]$Run,
            [Parameter(Mandatory = $true)][string]$ScriptLeafName,
            [switch]$RequireOutputDir
        )
        if ([string]::IsNullOrWhiteSpace($Run)) { return $false }
        # Do not trust a leaf-name substring: comments, here-strings, Write-Host,
        # and evil/run-release-build.ps1 must not count as evidence production.
        $parsedRun = Get-ReleasePowerShellRunAst -ScriptText $Run
        if (-not $parsedRun.Valid) { return $false }
        foreach ($command in @($parsedRun.AllCommands)) {
            if (-not (Test-ReleasePowerShellFileInvocationShape `
                    -CommandAst $command `
                    -ExpectedRelativePath ("scripts/{0}" -f $ScriptLeafName) `
                    -RequireNoProfile)) {
                continue
            }
            if (-not $RequireOutputDir -or
                (Test-ReleaseCommandHasExactVariableParameterValue `
                    -CommandAst $command `
                    -ParameterName 'OutputDir' `
                    -VariableName 'evidenceDir')) {
                return $true
            }
        }
        return $false
    }

    function ConvertTo-ReleaseControlledRunText {
        param([AllowEmptyString()][string]$Text)
        return (([string]$Text -replace '\s+', ' ').Trim())
    }

    function Test-ReleaseWindowsTauriCliSetupStepContract {
        param($Step)
        if ($null -eq $Step -or
            -not (Test-HostStepExecutionShape -Step $Step) -or
            -not [bool](Get-StepProp -Step $Step -Name 'run_present' -Default $false) -or
            -not [bool](Get-StepProp -Step $Step -Name 'if_present' -Default $false) -or
            -not [string]::Equals(([string](Get-StepProp -Step $Step -Name 'if_kind' -Default '')).Trim(), 'string', [System.StringComparison]::Ordinal) -or
            -not [string]::Equals(([string](Get-StepProp -Step $Step -Name 'if' -Default '')).Trim(), "github.event.inputs.skip_bundle == 'false'", [System.StringComparison]::Ordinal) -or
            -not [string]::Equals(([string](Get-StepProp -Step $Step -Name 'shell' -Default '')).Trim(), 'pwsh', [System.StringComparison]::OrdinalIgnoreCase) -or
            -not (Test-StepContinueOnErrorDisabled -Step $Step) -or
            -not (Test-RunWorkingDirectoryIsRepositoryDefault `
                    -Present ([bool](Get-StepProp -Step $Step -Name 'working_directory_present' -Default $false)) `
                    -Kind ([string](Get-StepProp -Step $Step -Name 'working_directory_kind' -Default '')) `
                    -Value ([string](Get-StepProp -Step $Step -Name 'working_directory' -Default '')))) {
            return $false
        }
        $expected = @'
$ErrorActionPreference = 'Stop'
$pinned = '2.11.2'
cargo install tauri-cli --locked --version $pinned --force
cargo tauri --version
$ver = (& cargo tauri --version 2>&1 | Out-String).Trim()
if ($ver -notmatch [regex]::Escape($pinned)) {
  throw "Expected tauri-cli $pinned, got: $ver"
}
'@
        return [string]::Equals(
            (ConvertTo-ReleaseControlledRunText -Text ([string](Get-StepProp -Step $Step -Name 'run' -Default ''))),
            (ConvertTo-ReleaseControlledRunText -Text $expected),
            [System.StringComparison]::Ordinal
        )
    }

    function Test-ReleaseAndroidTargetSetupStepContract {
        param($Step)
        if ($null -eq $Step -or
            -not (Test-HostStepExecutionShape -Step $Step) -or
            -not [bool](Get-StepProp -Step $Step -Name 'run_present' -Default $false) -or
            -not (Test-StepHasNoCondition -Step $Step) -or
            -not [string]::Equals(([string](Get-StepProp -Step $Step -Name 'shell' -Default '')).Trim(), 'pwsh', [System.StringComparison]::OrdinalIgnoreCase) -or
            -not (Test-StepContinueOnErrorDisabled -Step $Step) -or
            -not (Test-RunWorkingDirectoryIsRepositoryDefault `
                    -Present ([bool](Get-StepProp -Step $Step -Name 'working_directory_present' -Default $false)) `
                    -Kind ([string](Get-StepProp -Step $Step -Name 'working_directory_kind' -Default '')) `
                    -Value ([string](Get-StepProp -Step $Step -Name 'working_directory' -Default '')))) {
            return $false
        }
        $expected = @'
rustup target add aarch64-linux-android
rustup target list --installed | Select-String 'aarch64-linux-android'
'@
        return [string]::Equals(
            (ConvertTo-ReleaseControlledRunText -Text ([string](Get-StepProp -Step $Step -Name 'run' -Default ''))),
            (ConvertTo-ReleaseControlledRunText -Text $expected),
            [System.StringComparison]::Ordinal
        )
    }

    function Test-ReleaseHostTrustedRunTopology {
        param(
            [Parameter(Mandatory = $true)][string]$JobName,
            [Parameter(Mandatory = $true)][object[]]$Steps,
            [int]$CheckoutIndex,
            [int]$ProducerIndex,
            [int]$VerifierIndex
        )

        $topologyErrors = New-Object System.Collections.Generic.List[string]
        $expectedSetupCount = 0
        foreach ($step in @($Steps)) {
            if ($null -eq $step) { continue }
            $run = ([string](Get-StepProp -Step $step -Name 'run' -Default '')).Trim()
            if ([string]::IsNullOrWhiteSpace($run)) { continue }
            try { $index = [int](Get-StepProp -Step $step -Name 'index' -Default -1) } catch { $index = -1 }
            if ($index -eq $ProducerIndex -or $index -eq $VerifierIndex) { continue }

            $isExpectedSetup = if ($JobName -eq 'windows-host-evidence') {
                Test-ReleaseWindowsTauriCliSetupStepContract -Step $step
            } else {
                Test-ReleaseAndroidTargetSetupStepContract -Step $step
            }
            if (-not $isExpectedSetup) {
                $topologyErrors.Add(("unexpected executable run step at index {0}; trusted host topology permits only the fixed setup, producer, and verifier scripts" -f $index)) | Out-Null
                continue
            }
            $expectedSetupCount += 1
            if ($index -le $CheckoutIndex -or $index -ge $ProducerIndex) {
                $topologyErrors.Add(("trusted setup run step at index {0} must be after checkout and before the controlled producer" -f $index)) | Out-Null
            }
        }
        if ($expectedSetupCount -ne 1) {
            $topologyErrors.Add(("trusted host topology requires exactly one fixed setup run step before the producer (found {0})" -f $expectedSetupCount)) | Out-Null
        }
        return [pscustomobject]@{ Valid = ($topologyErrors.Count -eq 0); Errors = @($topologyErrors) }
    }

    $workflowDefaultWorkingDirectoryOk = Test-RunWorkingDirectoryIsRepositoryDefault `
        -Present ([bool](Get-StepProp -Step $parsed -Name 'workflow_defaults_run_working_directory_present' -Default $false)) `
        -Kind ([string](Get-StepProp -Step $parsed -Name 'workflow_defaults_run_working_directory_kind' -Default '')) `
        -Value ([string](Get-StepProp -Step $parsed -Name 'workflow_defaults_run_working_directory' -Default ''))
    $workflowDefaultShellOk = Test-RunShellIsAbsent `
        -Present ([bool](Get-StepProp -Step $parsed -Name 'workflow_defaults_run_shell_present' -Default $true))
    $workflowEnvironmentOk = -not [bool](Get-StepProp -Step $parsed -Name 'workflow_env_present' -Default $true)

    $allUploadArtifactReferences = @($parsed.upload_artifact_references | Where-Object { $null -ne $_ })
    $globalUploadTopologyOk = ($allUploadArtifactReferences.Count -eq $requiredJobs.Count)
    if ($globalUploadTopologyOk) {
        foreach ($requiredJobName in $requiredJobs) {
            $jobUploads = @($allUploadArtifactReferences | Where-Object {
                    [string]::Equals([string]$_.job, $requiredJobName, [System.StringComparison]::Ordinal)
                })
            if ($jobUploads.Count -ne 1 -or
                -not (Test-UsesPinnedUploadArtifactV4 -Uses ([string]$jobUploads[0].uses))) {
                $globalUploadTopologyOk = $false
                break
            }
        }
    }

    foreach ($jobName in $requiredJobs) {
        $info = $null
        if ($parsed.jobs -and $parsed.jobs.PSObject.Properties.Name -contains $jobName) {
            $info = $parsed.jobs.$jobName
        }
        if ($null -eq $info) {
            $errors.Add(("job '{0}' missing from parsed workflow." -f $jobName)) | Out-Null
            $jobResults[$jobName] = [pscustomobject]@{
                Present = $false
                HasVerifierBeforeUpload = $false
                VerifierIndex = -1
                UploadIndex = -1
                Reason = 'missing'
                ShellOk = $false
                ContinueOnErrorOk = $false
                UploadIfOk = $false
                ScriptContractOk = $false
                PathBindOk = $false
            }
            continue
        }

        $present = [bool]$info.present
        $steps = @()
        if ($info.PSObject.Properties.Name -contains 'steps' -and $null -ne $info.steps) {
            $steps = @($info.steps)
        }

        $uploadIdx = -1
        $verifierIdx = -1
        $checkoutIdx = -1
        $uploadStep = $null
        $verifierStep = $null
        $checkoutStep = $null
        $uploadSteps = @()
        $checkoutSteps = @()
        $producerSteps = @()
        $evidenceOutputIdSteps = @()
        $producerScriptLeafName = if ($jobName -eq 'windows-host-evidence') { 'run-release-build.ps1' } else { 'run-android-host-pipeline.ps1' }
        foreach ($step in $steps) {
            if ($null -eq $step) { continue }
            $idx = -1
            try { $idx = [int](Get-StepProp -Step $step -Name 'index' -Default -1) } catch { $idx = -1 }
            $uses = [string](Get-StepProp -Step $step -Name 'uses' -Default '')
            $run = [string](Get-StepProp -Step $step -Name 'run' -Default '')
            $stepId = [string](Get-StepProp -Step $step -Name 'id' -Default '')

            if (Test-UploadArtifactActionReference -Uses $uses) {
                $uploadSteps += [pscustomobject]@{
                    Index = $idx
                    Step = $step
                    Uses = $uses
                }
            }
            if ($verifierIdx -lt 0 -and -not [string]::IsNullOrWhiteSpace($run)) {
                if (Test-ReleaseRunInvokesCommand -ScriptText $run -CommandName $verifierCommand) {
                    $verifierIdx = $idx
                    $verifierStep = $step
                }
            }
            if (Test-CheckoutActionReference -Uses $uses) {
                $checkoutSteps += [pscustomobject]@{
                    Index = $idx
                    Step = $step
                    Uses = $uses
                }
            }
            if ([string]::Equals($stepId.Trim(), 'evidence', [System.StringComparison]::OrdinalIgnoreCase)) {
                $evidenceOutputIdSteps += [pscustomobject]@{
                    Index = $idx
                    Step = $step
                    Run = $run
                }
            }
            # Locate the producer even when the caller only requests basic
            # verifier ordering. The Windows producer is the sole step allowed
            # to bind workflow_dispatch input through a tightly governed env
            # key, so its identity cannot depend on RequireFreshProducer.
            if (Test-RunReferencesControlledScript -Run $run -ScriptLeafName $producerScriptLeafName -RequireOutputDir) {
                $producerSteps += [pscustomobject]@{
                    Index = $idx
                    Step = $step
                    Run = $run
                }
            }
        }

        if ($uploadSteps.Count -eq 1) {
            $uploadIdx = [int]$uploadSteps[0].Index
            $uploadStep = $uploadSteps[0].Step
        }
        if ($checkoutSteps.Count -eq 1) {
            $checkoutIdx = [int]$checkoutSteps[0].Index
            $checkoutStep = $checkoutSteps[0].Step
        }
        $producerIdx = -1
        $producerStep = $null
        if ($producerSteps.Count -eq 1) {
            $producerIdx = [int]$producerSteps[0].Index
            $producerStep = $producerSteps[0].Step
        }

        $reasonParts = New-Object System.Collections.Generic.List[string]
        $has = $false
        $shellOk = $false
        $continueOnErrorOk = $false
        $verifierIfOk = $false
        $uploadIfOk = $false
        $uploadContinueOnErrorOk = $false
        $uploadActionOk = $false
        $checkoutOk = (-not $RequireCheckout)
        $producerOk = (-not $RequireFreshProducer)
        $producerOutputIdOk = (-not $RequireFreshProducer)
        $producerHostOnlyOk = (-not $RequireFreshProducer -or $jobName -ne 'windows-host-evidence')
        $producerShellOk = (-not $RequireFreshProducer)
        $producerEvidenceBindingOk = (-not $RequireFreshProducer)
        $retentionOk = (-not $RequireRetentionDays14)
        $scriptContractOk = $false
        $verifierWorkingDirectoryOk = $false
        $runTopologyOk = (-not $RequireFreshProducer)
        $trustedRunTopology = $null
        $jobContinueOnErrorOk = Test-ContinueOnErrorDisabled `
            -Present ([bool](Get-StepProp -Step $info -Name 'continue_on_error_present' -Default $false)) `
            -Kind ([string](Get-StepProp -Step $info -Name 'continue_on_error_kind' -Default '')) `
            -Value (Get-StepProp -Step $info -Name 'continue_on_error' -Default $null)
        $jobIfOk = Test-StepHasNoCondition -Step $info
        $jobNeedsOk = -not [bool](Get-StepProp -Step $info -Name 'needs_present' -Default $false)
        $jobRunnerOk = ([string](Get-StepProp -Step $info -Name 'runs_on_kind' -Default '') -eq 'string' -and
            [string]::Equals(([string](Get-StepProp -Step $info -Name 'runs_on' -Default '')).Trim(), 'windows-latest', [System.StringComparison]::OrdinalIgnoreCase))
        $jobDefaultWorkingDirectoryOk = Test-RunWorkingDirectoryIsRepositoryDefault `
            -Present ([bool](Get-StepProp -Step $info -Name 'defaults_run_working_directory_present' -Default $false)) `
            -Kind ([string](Get-StepProp -Step $info -Name 'defaults_run_working_directory_kind' -Default '')) `
            -Value ([string](Get-StepProp -Step $info -Name 'defaults_run_working_directory' -Default ''))
        $jobDefaultShellOk = Test-RunShellIsAbsent `
            -Present ([bool](Get-StepProp -Step $info -Name 'defaults_run_shell_present' -Default $true))
        $jobEnvironmentOk = Test-HostJobEnvironmentContract -Job $info
        $jobExecutionContextOk = (-not [bool](Get-StepProp -Step $info -Name 'container_present' -Default $true) -and
            -not [bool](Get-StepProp -Step $info -Name 'services_present' -Default $true))
        $stepEnvironmentOk = (@($steps | Where-Object {
                    -not (Test-HostStepEnvironmentContract -JobName $jobName -Step $_ -ProducerIndex $producerIdx)
                }).Count -eq 0)
        $stepExecutionShapeOk = (@($steps | Where-Object { -not (Test-HostStepExecutionShape -Step $_) }).Count -eq 0)
        $pathBindOk = $false
        $scriptContract = $null
        if ($RequireFreshProducer) {
            $trustedRunTopology = Test-ReleaseHostTrustedRunTopology `
                -JobName $jobName `
                -Steps $steps `
                -CheckoutIndex $checkoutIdx `
                -ProducerIndex $producerIdx `
                -VerifierIndex $verifierIdx
            $runTopologyOk = [bool]$trustedRunTopology.Valid
        }

        if (-not $present) {
            $reasonParts.Add($(if ($info.reason) { [string]$info.reason } else { 'job missing or not a mapping' })) | Out-Null
        } elseif ($steps.Count -eq 0 -and $info.reason -and [string]$info.reason -ne 'ok') {
            $reasonParts.Add([string]$info.reason) | Out-Null
        } elseif ($verifierIdx -lt 0) {
            $reasonParts.Add('missing executable Assert-ReleaseEvidencePackage CommandAst invocation in run step') | Out-Null
        } elseif ($uploadSteps.Count -eq 0) {
            $reasonParts.Add('missing actions/upload-artifact step') | Out-Null
        } elseif ($uploadSteps.Count -ne 1) {
            $reasonParts.Add(("host job must contain exactly one actions/upload-artifact@v4 step (found {0})" -f $uploadSteps.Count)) | Out-Null
        } elseif ($RequireCheckout -and $checkoutSteps.Count -eq 0) {
            $reasonParts.Add('host job must contain exactly one actions/checkout@v4 step before the controlled verifier') | Out-Null
        } elseif ($RequireCheckout -and $checkoutSteps.Count -ne 1) {
            $reasonParts.Add(("host job must contain exactly one actions/checkout@v4 step (found {0})" -f $checkoutSteps.Count)) | Out-Null
        } elseif ($RequireCheckout -and $checkoutIdx -ge $verifierIdx) {
            $reasonParts.Add('actions/checkout@v4 must run before the controlled verifier') | Out-Null
        } elseif ($RequireFreshProducer -and $producerSteps.Count -eq 0) {
            $reasonParts.Add(("host job must contain exactly one fresh evidence producer ({0}) before the controlled verifier" -f $producerScriptLeafName)) | Out-Null
        } elseif ($RequireFreshProducer -and $producerSteps.Count -ne 1) {
            $reasonParts.Add(("host job must contain exactly one fresh evidence producer ({0}; found {1})" -f $producerScriptLeafName, $producerSteps.Count)) | Out-Null
        } elseif ($RequireFreshProducer -and $evidenceOutputIdSteps.Count -ne 1) {
            $reasonParts.Add(("host job must assign id: evidence exactly once to the fresh producer (found {0})" -f $evidenceOutputIdSteps.Count)) | Out-Null
        } elseif ($RequireFreshProducer -and $evidenceOutputIdSteps[0].Index -ne $producerIdx) {
            $reasonParts.Add('id: evidence must belong to the controlled fresh evidence producer') | Out-Null
        } elseif ($RequireFreshProducer -and $producerIdx -ge $verifierIdx) {
            $reasonParts.Add('fresh evidence producer must run before the controlled verifier') | Out-Null
        } elseif ($verifierIdx -ge $uploadIdx) {
            $reasonParts.Add('Assert-ReleaseEvidencePackage is not before upload-artifact') | Out-Null
        } elseif ($uploadIdx -ne ($verifierIdx + 1)) {
            $reasonParts.Add('controlled verifier must be immediately followed by its only upload-artifact step') | Out-Null
        } else {
            if (-not $globalUploadTopologyOk) {
                $reasonParts.Add('workflow must contain exactly one actions/upload-artifact@v4 step in each required host job and no other upload-artifact steps') | Out-Null
            }
            if (-not $runTopologyOk) {
                foreach ($topologyError in @($trustedRunTopology.Errors)) {
                    $reasonParts.Add([string]$topologyError) | Out-Null
                }
            }
            if (-not $jobContinueOnErrorOk) {
                    $reasonParts.Add('host job continue-on-error must be absent') | Out-Null
            }
            if (-not $jobIfOk) {
                $reasonParts.Add('host job must not set an if: condition') | Out-Null
            }
            if (-not $jobNeedsOk) {
                $reasonParts.Add('host job must not set needs; a skipped or failed dependency can suppress evidence execution') | Out-Null
            }
            if (-not $jobRunnerOk) {
                $reasonParts.Add('host job runs-on must be exactly windows-latest') | Out-Null
            }
            if (-not $workflowDefaultWorkingDirectoryOk) {
                $reasonParts.Add('workflow defaults.run.working-directory must be absent or empty for repository-root verification') | Out-Null
            }
            if (-not $workflowDefaultShellOk) {
                $reasonParts.Add('workflow defaults.run.shell must be absent for controlled host command execution') | Out-Null
            }
            if (-not $workflowEnvironmentOk) {
                $reasonParts.Add('workflow env must be absent for controlled host command execution') | Out-Null
            }
            if (-not $jobDefaultWorkingDirectoryOk) {
                $reasonParts.Add('job defaults.run.working-directory must be absent or empty for repository-root verification') | Out-Null
            }
            if (-not $jobDefaultShellOk) {
                $reasonParts.Add('host job defaults.run.shell must be absent') | Out-Null
            }
            if (-not $jobEnvironmentOk) {
                $reasonParts.Add('host job env must be exactly CARGO_TARGET_DIR="" and RUST_LOG="info" with exact key spelling') | Out-Null
            }
            if (-not $jobExecutionContextOk) {
                $reasonParts.Add('host job must not set container or services') | Out-Null
            }
            if (-not $stepEnvironmentOk) {
                $reasonParts.Add('controlled host job steps must not set env overrides, except the exact Windows producer skip_bundle input binding') | Out-Null
            }
            if (-not $stepExecutionShapeOk) {
                $reasonParts.Add('each controlled host step must use exactly one execution mode (uses or run), never both or neither') | Out-Null
            }
            if ($RequireFreshProducer) {
                $producerExecutionShapeOk = Test-HostStepExecutionShape -Step $producerStep
                $producerIfOk = Test-StepHasNoCondition -Step $producerStep
                $producerContinueOk = Test-StepContinueOnErrorDisabled -Step $producerStep
                $producerWorkingDirectoryOk = Test-RunWorkingDirectoryIsRepositoryDefault `
                    -Present ([bool](Get-StepProp -Step $producerStep -Name 'working_directory_present' -Default $false)) `
                    -Kind ([string](Get-StepProp -Step $producerStep -Name 'working_directory_kind' -Default '')) `
                    -Value ([string](Get-StepProp -Step $producerStep -Name 'working_directory' -Default ''))
                if (-not $producerIfOk) {
                    $reasonParts.Add('fresh evidence producer must not set an if: condition') | Out-Null
                }
                if (-not $producerExecutionShapeOk -or -not [bool](Get-StepProp -Step $producerStep -Name 'run_present' -Default $false)) {
                    $reasonParts.Add('fresh evidence producer must be an unambiguous run-only step') | Out-Null
                }
                if (-not $producerContinueOk) {
                    $reasonParts.Add('fresh evidence producer continue-on-error must be absent') | Out-Null
                }
                if (-not $producerWorkingDirectoryOk) {
                    $reasonParts.Add('fresh evidence producer working-directory must be absent or empty') | Out-Null
                }
                $producerOutputIdOk = ($evidenceOutputIdSteps.Count -eq 1 -and $evidenceOutputIdSteps[0].Index -eq $producerIdx)
                if (-not $producerOutputIdOk) {
                    $reasonParts.Add('fresh evidence producer must own id: evidence so verifier/upload consume this run output') | Out-Null
                }
                if ($jobName -eq 'windows-host-evidence') {
                    $producerHostOnlyOk = Test-ReleaseWindowsHostOnlyProducerScriptContract `
                        -ScriptText ([string]$producerSteps[0].Run)
                    if (-not $producerHostOnlyOk) {
                        $reasonParts.Add('Windows fresh evidence producer must prove the empty/default input branch invokes real run-release-build.ps1 -SkipBundle') | Out-Null
                    }
                }
                $producerShellOk = ([string](Get-StepProp -Step $producerStep -Name 'shell_kind' -Default '') -eq 'string' -and
                    [string]::Equals(([string](Get-StepProp -Step $producerStep -Name 'shell' -Default '')).Trim(), 'pwsh', [System.StringComparison]::OrdinalIgnoreCase))
                if (-not $producerShellOk) {
                    $reasonParts.Add('fresh evidence producer shell must be exactly pwsh so its PowerShell AST contract matches runtime execution') | Out-Null
                }
                $producerEvidenceBindingOk = Test-ReleaseEvidenceProducerScriptContract `
                    -ScriptText ([string]$producerSteps[0].Run) `
                    -ScriptLeafName $producerScriptLeafName `
                    -RequireWindowsHostOnly:($jobName -eq 'windows-host-evidence')
                if (-not $producerEvidenceBindingOk) {
                    $reasonParts.Add('fresh evidence producer must use only the controlled AST grammar and bind -OutputDir $evidenceDir to dir=$evidenceDir in GITHUB_OUTPUT') | Out-Null
                }
                $producerOk = $producerExecutionShapeOk -and $producerIfOk -and $producerContinueOk -and $producerWorkingDirectoryOk -and
                    $producerOutputIdOk -and $producerHostOnlyOk -and $producerShellOk -and $producerEvidenceBindingOk
            }
            if ($RequireCheckout) {
                $checkoutUses = [string](Get-StepProp -Step $checkoutStep -Name 'uses' -Default '')
                $checkoutActionOk = Test-UsesPinnedCheckoutV4 -Uses $checkoutUses
                $checkoutExecutionShapeOk = (Test-HostStepExecutionShape -Step $checkoutStep) -and
                    [bool](Get-StepProp -Step $checkoutStep -Name 'uses_present' -Default $false)
                $checkoutIfOk = Test-StepHasNoCondition -Step $checkoutStep
                $checkoutContinueOk = Test-StepContinueOnErrorDisabled -Step $checkoutStep
                # Checkout determines the helper that the verifier dot-sources.
                # Keep its input surface deliberately tiny: the production workflow
                # only needs fetch-depth. Unknown, case-variant, or duplicate-after-
                # normalization keys (github-server-url, repository, token, path,
                # sparse checkout, etc.) can redirect or alter the trusted checkout.
                $checkoutRawWithKeys = @((Get-StepProp -Step $checkoutStep -Name 'with_raw_keys' -Default @()) | ForEach-Object {
                        ([string]$_).Trim()
                    })
                $checkoutWithKeys = @($checkoutRawWithKeys | ForEach-Object {
                        ([string]$_).Trim().ToLowerInvariant()
                    })
                $checkoutWithKeysUnique = [bool](Get-StepProp -Step $checkoutStep -Name 'with_keys_unique' -Default $false)
                $allowedCheckoutWithKeys = @('fetch-depth')
                $unsupportedCheckoutWithKeys = @($checkoutWithKeys | Where-Object {
                        [string]::IsNullOrWhiteSpace($_) -or $allowedCheckoutWithKeys -notcontains $_
                    })
                $caseVariantCheckoutWithKeys = @($checkoutRawWithKeys | Where-Object {
                        -not [string]::Equals($_, 'fetch-depth', [System.StringComparison]::Ordinal)
                    })
                $checkoutInputsOk = $checkoutWithKeysUnique -and $unsupportedCheckoutWithKeys.Count -eq 0 -and
                    $caseVariantCheckoutWithKeys.Count -eq 0
                if (-not $checkoutActionOk) {
                    $reasonParts.Add(("checkout step must use exactly actions/checkout@v4 (got '{0}')" -f $checkoutUses)) | Out-Null
                }
                if (-not $checkoutExecutionShapeOk) {
                    $reasonParts.Add('checkout step must be an unambiguous uses-only action step') | Out-Null
                }
                if (-not $checkoutIfOk) {
                    $reasonParts.Add('checkout step must not set an if: condition') | Out-Null
                }
                if (-not $checkoutContinueOk) {
                    $reasonParts.Add('checkout step continue-on-error must be absent') | Out-Null
                }
                if (-not $checkoutWithKeysUnique) {
                    $reasonParts.Add('checkout step with keys must be unique after case-insensitive normalization') | Out-Null
                }
                if ($unsupportedCheckoutWithKeys.Count -gt 0) {
                    $reasonParts.Add(("checkout step has unsupported with input(s): {0}; only fetch-depth is allowed" -f ($unsupportedCheckoutWithKeys -join ', '))) | Out-Null
                }
                if ($caseVariantCheckoutWithKeys.Count -gt 0) {
                    $reasonParts.Add(("checkout step with keys must use exact lowercase spelling fetch-depth (got {0})" -f ($caseVariantCheckoutWithKeys -join ', '))) | Out-Null
                }
                $checkoutOk = $checkoutActionOk -and $checkoutExecutionShapeOk -and $checkoutIfOk -and $checkoutContinueOk -and $checkoutInputsOk
            }
            # Controlled verifier step metadata
            $shell = ([string](Get-StepProp -Step $verifierStep -Name 'shell' -Default '')).Trim()
            if ($allowedShells -contains $shell) {
                $shellOk = $true
            } else {
                $reasonParts.Add(("verifier step shell must be pwsh/powershell (got '{0}')" -f $shell)) | Out-Null
            }

            $verifierWorkingDirectoryOk = Test-RunWorkingDirectoryIsRepositoryDefault `
                -Present ([bool](Get-StepProp -Step $verifierStep -Name 'working_directory_present' -Default $false)) `
                -Kind ([string](Get-StepProp -Step $verifierStep -Name 'working_directory_kind' -Default '')) `
                -Value ([string](Get-StepProp -Step $verifierStep -Name 'working_directory' -Default ''))
            if (-not $verifierWorkingDirectoryOk) {
                $reasonParts.Add('verifier step working-directory must be absent or empty (repository root required)') | Out-Null
            }

            if (Test-StepContinueOnErrorDisabled -Step $verifierStep) {
                $continueOnErrorOk = $true
            } else {
                $reasonParts.Add('verifier step continue-on-error must be absent') | Out-Null
            }

            if (Test-StepHasNoCondition -Step $verifierStep) {
                $verifierIfOk = $true
            } else {
                $reasonParts.Add('verifier step must not set an if: condition') | Out-Null
            }

            $run = [string](Get-StepProp -Step $verifierStep -Name 'run' -Default '')
            $verifierExecutionShapeOk = (Test-HostStepExecutionShape -Step $verifierStep) -and
                [bool](Get-StepProp -Step $verifierStep -Name 'run_present' -Default $false)
            if (-not $verifierExecutionShapeOk) {
                $reasonParts.Add('verifier step must be an unambiguous run-only step') | Out-Null
            }
            $scriptContract = Test-ReleaseVerifierStepScriptContract -ScriptText $run
            if ($scriptContract.Valid) {
                $scriptContractOk = $true
            } else {
                foreach ($se in @($scriptContract.Errors)) {
                    $reasonParts.Add([string]$se) | Out-Null
                }
            }

            # Controlled upload step metadata
            $uploadUses = [string](Get-StepProp -Step $uploadStep -Name 'uses' -Default '')
            $uploadExecutionShapeOk = (Test-HostStepExecutionShape -Step $uploadStep) -and
                [bool](Get-StepProp -Step $uploadStep -Name 'uses_present' -Default $false)
            if ((Test-UsesPinnedUploadArtifactV4 -Uses $uploadUses) -and $uploadExecutionShapeOk) {
                $uploadActionOk = $true
            } else {
                $reasonParts.Add(("upload-artifact step must use exactly actions/upload-artifact@v4 (got '{0}')" -f $uploadUses)) | Out-Null
                if (-not $uploadExecutionShapeOk) {
                    $reasonParts.Add('upload-artifact step must be an unambiguous uses-only action step') | Out-Null
                }
            }

            if (Test-StepHasNoCondition -Step $uploadStep) {
                $uploadIfOk = $true
            } else {
                $reasonParts.Add('upload-artifact step must not set an if: condition') | Out-Null
            }

            if (Test-StepContinueOnErrorDisabled -Step $uploadStep) {
                $uploadContinueOnErrorOk = $true
            } else {
                $reasonParts.Add('upload-artifact step continue-on-error must be absent') | Out-Null
            }

            if ($RequireRetentionDays14) {
                $retentionRaw = Get-StepProp -Step $uploadStep -Name 'with_retention_days' -Default $null
                try {
                    $retentionOk = ($null -ne $retentionRaw -and [int]$retentionRaw -eq 14)
                } catch {
                    $retentionOk = $false
                }
                if (-not $retentionOk) {
                    $reasonParts.Add('upload-artifact retention-days must be exactly 14') | Out-Null
                }
            }

            $uploadPath = [string](Get-StepProp -Step $uploadStep -Name 'with_path' -Default '')
            if ([string]::IsNullOrWhiteSpace($uploadPath)) {
                $reasonParts.Add('upload-artifact step missing with.path bound to steps.evidence.outputs.dir') | Out-Null
            } elseif (-not [string]::Equals($uploadPath.Trim(), $requiredEvidenceLiteral, [System.StringComparison]::Ordinal)) {
                $reasonParts.Add(("upload-artifact with.path must be exactly '{0}' (got '{1}')" -f $requiredEvidenceLiteral, $uploadPath)) | Out-Null
            } elseif ($scriptContractOk -and
                      -not [string]::IsNullOrWhiteSpace([string]$scriptContract.EvidenceDirExpression) -and
                      [string]::Equals([string]$scriptContract.EvidenceDirExpression.Trim(), $requiredEvidenceLiteral, [System.StringComparison]::Ordinal)) {
                $pathBindOk = $true
            } elseif ($scriptContractOk) {
                $reasonParts.Add('verifier EvidenceDir and upload path are not both bound to steps.evidence.outputs.dir') | Out-Null
            }

            if ($globalUploadTopologyOk -and $jobContinueOnErrorOk -and $jobIfOk -and $jobNeedsOk -and $jobRunnerOk -and
                $workflowDefaultWorkingDirectoryOk -and $workflowDefaultShellOk -and $workflowEnvironmentOk -and
                $jobDefaultWorkingDirectoryOk -and $jobDefaultShellOk -and $jobEnvironmentOk -and $jobExecutionContextOk -and
                $stepEnvironmentOk -and $stepExecutionShapeOk -and $verifierExecutionShapeOk -and $verifierWorkingDirectoryOk -and
                $checkoutOk -and $producerOk -and $runTopologyOk -and $retentionOk -and
                $shellOk -and $continueOnErrorOk -and $verifierIfOk -and
                $uploadActionOk -and $uploadIfOk -and $uploadContinueOnErrorOk -and
                $scriptContractOk -and $pathBindOk -and $reasonParts.Count -eq 0) {
                $has = $true
            }
        }

        $reason = if ($reasonParts.Count -eq 0) { 'ok' } else { ($reasonParts -join '; ') }

        $jobResults[$jobName] = [pscustomobject]@{
            Present = $present
            HasVerifierBeforeUpload = $has
            VerifierIndex = $verifierIdx
            UploadIndex = $uploadIdx
            CheckoutIndex = $checkoutIdx
            ProducerIndex = $producerIdx
            Reason = $reason
            ShellOk = $shellOk
            ContinueOnErrorOk = $continueOnErrorOk
            JobContinueOnErrorOk = $jobContinueOnErrorOk
            JobIfOk = $jobIfOk
            JobNeedsOk = $jobNeedsOk
            JobRunnerOk = $jobRunnerOk
            VerifierWorkingDirectoryOk = $verifierWorkingDirectoryOk
            JobDefaultShellOk = $jobDefaultShellOk
            JobEnvironmentOk = $jobEnvironmentOk
            JobExecutionContextOk = $jobExecutionContextOk
            StepEnvironmentOk = $stepEnvironmentOk
            CheckoutOk = $checkoutOk
            ProducerOk = $producerOk
            ProducerOutputIdOk = $producerOutputIdOk
            ProducerHostOnlyOk = $producerHostOnlyOk
            RunTopologyOk = $runTopologyOk
            ProducerShellOk = $producerShellOk
            ProducerEvidenceBindingOk = $producerEvidenceBindingOk
            UploadIfOk = $uploadIfOk
            UploadContinueOnErrorOk = $uploadContinueOnErrorOk
            UploadActionOk = $uploadActionOk
            RetentionOk = $retentionOk
            ScriptContractOk = $scriptContractOk
            PathBindOk = $pathBindOk
        }
        if (-not $has) {
            $errors.Add(("job '{0}' full offline verifier order failed: {1}" -f $jobName, $reason)) | Out-Null
        }
    }

    # Preserve exact parsed metadata for every workflow job. The two controlled
    # host-evidence jobs are replaced with their stricter verifier-order result,
    # while CI gate governance consumes the untouched generic metadata for jobs
    # such as frontend-gate, secret-scan and pester-release-tests.
    $allJobResults = @{}
    if ($null -ne $parsed.jobs) {
        if ($parsed.jobs -is [System.Collections.IDictionary]) {
            foreach ($key in @($parsed.jobs.Keys)) {
                $allJobResults[[string]$key] = $parsed.jobs[$key]
            }
        } else {
            foreach ($property in @($parsed.jobs.PSObject.Properties)) {
                $allJobResults[[string]$property.Name] = $property.Value
            }
        }
    }
    foreach ($key in @($jobResults.Keys)) {
        $allJobResults[[string]$key] = $jobResults[$key]
    }

    return [pscustomobject]@{
        Valid = ($errors.Count -eq 0)
        Engine = $engine
        Errors = @($errors)
        jobs = [hashtable]$allJobResults
        ActionReferences = @($parsed.action_references | Where-Object { $null -ne $_ })
        RunBlocks = @($parsed.run_blocks | Where-Object { $null -ne $_ })
        WorkflowDefaultsRunWorkingDirectoryPresent = [bool]$parsed.workflow_defaults_run_working_directory_present
        WorkflowDefaultsRunWorkingDirectoryKind = [string]$parsed.workflow_defaults_run_working_directory_kind
        WorkflowDefaultsRunWorkingDirectory = [string]$parsed.workflow_defaults_run_working_directory
        WorkflowDefaultsRunShellPresent = [bool]$parsed.workflow_defaults_run_shell_present
        WorkflowDefaultsRunShellKind = [string]$parsed.workflow_defaults_run_shell_kind
        WorkflowDefaultsRunShell = [string]$parsed.workflow_defaults_run_shell
        WorkflowEnvPresent = [bool]$parsed.workflow_env_present
        WorkflowEnvKind = [string]$parsed.workflow_env_kind
        WorkflowEnv = $parsed.workflow_env
        WorkflowEnvKeysUnique = [bool]$parsed.workflow_env_keys_unique
        WorkflowEnvRawKeys = @($parsed.workflow_env_raw_keys | Where-Object { $null -ne $_ })
        WorkflowOnPresent = [bool]$parsed.workflow_on_present
        WorkflowOnKind = [string]$parsed.workflow_on_kind
        WorkflowTriggers = @($parsed.workflow_triggers | Where-Object { $null -ne $_ })
        RootPermissionsPresent = [bool]$parsed.root_permissions_present
        RootPermissionsKind = [string]$parsed.root_permissions_kind
        RootPermissions = $parsed.root_permissions
        RootPermissionsKeysUnique = [bool]$parsed.root_permissions_keys_unique
        RootPermissionsRawKeys = @($parsed.root_permissions_raw_keys | Where-Object { $null -ne $_ })
        JobPermissionRecords = @($parsed.job_permission_records | Where-Object { $null -ne $_ })
        SkipBundleDefaultPresent = [bool]$parsed.skip_bundle_default_present
        SkipBundleDefaultKind = [string]$parsed.skip_bundle_default_kind
        SkipBundleDefault = [string]$parsed.skip_bundle_default
    }
}

function Test-ReleaseGateRunBlockIsUnconditional {
    <#
    .SYNOPSIS
    Returns true only when a parsed workflow run step cannot be skipped or allowed
    to fail by workflow/job/step metadata.

    .DESCRIPTION
    This is intentionally stricter than checking the run body. A real CommandAst
    inside `if: ${{ false }}` or a continue-on-error step is not a release gate.
    #>
    param(
        [Parameter(Mandatory = $true)]$RunBlock,
        [switch]$RequirePowerShellShell
    )

    function Get-RecordProp {
        param($Record, [string]$Name, $Default = $null)
        if ($null -ne $Record -and $Record.PSObject.Properties.Name -contains $Name) {
            return $Record.$Name
        }
        return $Default
    }

    # Controlled CI gates deliberately require fields to be absent, rather than
    # accepting parser-specific boolean spellings such as `off`/`no` as false.
    if ([bool](Get-RecordProp -Record $RunBlock -Name 'if_present' -Default $false) -or
        [bool](Get-RecordProp -Record $RunBlock -Name 'continue_on_error_present' -Default $false) -or
        [bool](Get-RecordProp -Record $RunBlock -Name 'job_if_present' -Default $false) -or
        [bool](Get-RecordProp -Record $RunBlock -Name 'job_continue_on_error_present' -Default $false) -or
        [bool](Get-RecordProp -Record $RunBlock -Name 'job_needs_present' -Default $false)) {
        return $false
    }
    if ($RequirePowerShellShell) {
        $shell = ([string](Get-RecordProp -Record $RunBlock -Name 'shell' -Default '')).Trim().ToLowerInvariant()
        if ($shell -notin @('pwsh', 'powershell')) { return $false }
    }
    return $true
}

function Test-ReleaseCiGateJobContract {
    <#
    .SYNOPSIS
    Binds a required CI gate to its intended checked-out job and working
    directory instead of accepting a command-shaped decoy anywhere in YAML.
    #>
    param(
        [Parameter(Mandatory = $true)]$WorkflowMetadata,
        [Parameter(Mandatory = $true)][ValidateSet('npm_ci', 'secret_scan')][string]$Gate
    )

    $errors = New-Object System.Collections.Generic.List[string]
    $jobName = if ($Gate -eq 'npm_ci') { 'frontend-gate' } else { 'secret-scan' }
    $expectedRunner = if ($Gate -eq 'npm_ci') { 'ubuntu-latest' } else { 'windows-latest' }

    function Get-RecordValue {
        param($Record, [string]$Name, $Default = $null)
        if ($null -ne $Record -and $Record.PSObject.Properties.Name -contains $Name) { return $Record.$Name }
        return $Default
    }
    function Test-FieldAbsent {
        param($Record, [string]$PresentName)
        return (-not [bool](Get-RecordValue -Record $Record -Name $PresentName -Default $false))
    }
    function Test-FieldExplicitlyAbsent {
        param($Record, [string]$PresentName)
        if ($null -eq $Record -or -not ($Record.PSObject.Properties.Name -contains $PresentName)) {
            return $false
        }
        return (-not [bool]$Record.$PresentName)
    }
    function Test-CheckoutStepContract {
        param($Step)
        if ($null -eq $Step -or
            -not [bool](Get-RecordValue -Record $Step -Name 'uses_present' -Default $false) -or
            -not [string]::Equals(([string](Get-RecordValue -Record $Step -Name 'uses_kind' -Default '')).Trim(), 'string', [System.StringComparison]::Ordinal) -or
            -not [string]::Equals(([string](Get-RecordValue -Record $Step -Name 'uses' -Default '')).Trim(), 'actions/checkout@v4', [System.StringComparison]::OrdinalIgnoreCase)) {
            return $false
        }
        if (-not (Test-FieldAbsent -Record $Step -PresentName 'if_present') -or
            -not (Test-FieldAbsent -Record $Step -PresentName 'continue_on_error_present') -or
            -not (Test-FieldAbsent -Record $Step -PresentName 'working_directory_present') -or
            -not (Test-FieldExplicitlyAbsent -Record $Step -PresentName 'run_present') -or
            -not (Test-FieldExplicitlyAbsent -Record $Step -PresentName 'env_present')) {
            return $false
        }
        $rawKeys = @((Get-RecordValue -Record $Step -Name 'with_raw_keys' -Default @()) | ForEach-Object { ([string]$_).Trim() })
        return ([bool](Get-RecordValue -Record $Step -Name 'with_keys_unique' -Default $false) -and
            @($rawKeys | Where-Object { -not [string]::Equals($_, 'fetch-depth', [System.StringComparison]::Ordinal) }).Count -eq 0)
    }
    function Test-SetupNodeStepContract {
        param($Step)
        if ($null -eq $Step) {
            return $false
        }
        foreach ($field in @('if_present', 'continue_on_error_present', 'working_directory_present', 'shell_present')) {
            if (-not (Test-FieldAbsent -Record $Step -PresentName $field)) { return $false }
        }
        if (-not (Test-FieldExplicitlyAbsent -Record $Step -PresentName 'env_present')) { return $false }

        if ([bool](Get-RecordValue -Record $Step -Name 'uses_present' -Default $false)) {
            return (
                [string]::Equals(([string](Get-RecordValue -Record $Step -Name 'uses_kind' -Default '')).Trim(), 'string', [System.StringComparison]::Ordinal) -and
                [string]::Equals(([string](Get-RecordValue -Record $Step -Name 'uses' -Default '')).Trim(), 'actions/setup-node@v4', [System.StringComparison]::OrdinalIgnoreCase) -and
                (Test-FieldExplicitlyAbsent -Record $Step -PresentName 'run_present')
            )
        }

        if (-not (Test-FieldExplicitlyAbsent -Record $Step -PresentName 'uses_present') -or
            -not [bool](Get-RecordValue -Record $Step -Name 'run_present' -Default $false) -or
            -not [string]::Equals(([string](Get-RecordValue -Record $Step -Name 'run_kind' -Default '')).Trim(), 'string', [System.StringComparison]::Ordinal)) {
            return $false
        }

        $expected = @'
set -euo pipefail
NODE_VERSION=22.12.0
NODE_DIR=/opt/hostedtoolcache/storyforge-node22
if [ ! -x "$NODE_DIR/bin/node" ]; then
  ARCHIVE="node-v${NODE_VERSION}-linux-x64.tar.xz"
  curl -fsSL "https://nodejs.org/dist/v${NODE_VERSION}/${ARCHIVE}" -o "/tmp/${ARCHIVE}"
  curl -fsSL "https://nodejs.org/dist/v${NODE_VERSION}/SHASUMS256.txt" -o /tmp/node-SHASUMS256.txt
  (cd /tmp && grep "  ${ARCHIVE}$" node-SHASUMS256.txt | sha256sum -c -)
  mkdir -p "$NODE_DIR"
  tar -xJf "/tmp/${ARCHIVE}" -C "$NODE_DIR" --strip-components=1
fi
echo "$NODE_DIR/bin" >> "$GITHUB_PATH"
"$NODE_DIR/bin/node" --version
'@
        $actualNormalized = (([string](Get-RecordValue -Record $Step -Name 'run' -Default '') -replace '\s+', ' ').Trim())
        $expectedNormalized = (($expected -replace '\s+', ' ').Trim())
        return [string]::Equals($actualNormalized, $expectedNormalized, [System.StringComparison]::Ordinal)
    }

    $job = Get-ReleaseWorkflowJobExact -WorkflowMetadata $WorkflowMetadata -JobName $jobName
    if ($null -eq $job -or -not [bool](Get-RecordValue -Record $job -Name 'present' -Default $false)) {
        $errors.Add("required CI job '$jobName' is missing") | Out-Null
        return [pscustomobject]@{ Valid = $false; Errors = @($errors); Job = $jobName }
    }
    if ([string](Get-RecordValue -Record $job -Name 'runs_on_kind' -Default '') -ne 'string' -or
        -not [string]::Equals(([string](Get-RecordValue -Record $job -Name 'runs_on' -Default '')).Trim(), $expectedRunner, [System.StringComparison]::OrdinalIgnoreCase)) {
        $errors.Add("CI job '$jobName' must run on $expectedRunner") | Out-Null
    }
    foreach ($field in @('if_present', 'continue_on_error_present', 'needs_present')) {
        if (-not (Test-FieldAbsent -Record $job -PresentName $field)) {
            $errors.Add("CI job '$jobName' must not set $field") | Out-Null
        }
    }
    if (-not (Test-FieldAbsent -Record $WorkflowMetadata -PresentName 'WorkflowDefaultsRunWorkingDirectoryPresent')) {
        $errors.Add('ci-gates workflow defaults.run.working-directory must be absent') | Out-Null
    }
    if (-not (Test-FieldExplicitlyAbsent -Record $WorkflowMetadata -PresentName 'WorkflowDefaultsRunShellPresent')) {
        $errors.Add('ci-gates workflow defaults.run.shell must be absent') | Out-Null
    }
    if (-not (Test-FieldExplicitlyAbsent -Record $WorkflowMetadata -PresentName 'WorkflowEnvPresent')) {
        $errors.Add('ci-gates workflow env must be absent') | Out-Null
    }

    $jobDefaultPresent = [bool](Get-RecordValue -Record $job -Name 'defaults_run_working_directory_present' -Default $false)
    $jobDefaultKind = [string](Get-RecordValue -Record $job -Name 'defaults_run_working_directory_kind' -Default '')
    $jobDefaultValue = ([string](Get-RecordValue -Record $job -Name 'defaults_run_working_directory' -Default '')).Trim()
    if ($Gate -eq 'npm_ci') {
        if (-not $jobDefaultPresent -or $jobDefaultKind -ne 'string' -or
            -not [string]::Equals($jobDefaultValue, 'frontend', [System.StringComparison]::Ordinal)) {
            $errors.Add("CI job '$jobName' must set defaults.run.working-directory exactly to frontend") | Out-Null
        }
    } elseif ($jobDefaultPresent) {
        $errors.Add("CI job '$jobName' must use repository-root working directory") | Out-Null
    }
    if (-not (Test-FieldExplicitlyAbsent -Record $job -PresentName 'defaults_run_shell_present')) {
        $errors.Add("CI job '$jobName' defaults.run.shell must be absent") | Out-Null
    }
    foreach ($field in @('env_present', 'container_present', 'services_present')) {
        if (-not (Test-FieldExplicitlyAbsent -Record $job -PresentName $field)) {
            $errors.Add("CI job '$jobName' must not set $field") | Out-Null
        }
    }

    $steps = @((Get-RecordValue -Record $job -Name 'steps' -Default @()))
    $checkoutSteps = @($steps | Where-Object { Test-CheckoutStepContract -Step $_ })
    if ($checkoutSteps.Count -ne 1) {
        $errors.Add("CI job '$jobName' must have exactly one unconditional repository checkout") | Out-Null
    }

    $gateSteps = @($steps | Where-Object {
            $run = [string](Get-RecordValue -Record $_ -Name 'run' -Default '')
            if ($Gate -eq 'npm_ci') {
                return Test-ReleaseExactNpmCiGateScript -ScriptText $run
            }
            return Test-ReleaseExactSecretScanGateScript -ScriptText $run
        })
    if ($gateSteps.Count -ne 1) {
        $errors.Add("CI job '$jobName' must have exactly one flat executable $Gate gate") | Out-Null
    } else {
        $gateStep = $gateSteps[0]
        if (-not (Test-FieldExplicitlyAbsent -Record $gateStep -PresentName 'uses_present') -or
            -not [bool](Get-RecordValue -Record $gateStep -Name 'run_present' -Default $false) -or
            -not [string]::Equals(([string](Get-RecordValue -Record $gateStep -Name 'run_kind' -Default '')).Trim(), 'string', [System.StringComparison]::Ordinal)) {
            $errors.Add("$Gate gate step must be a run-only YAML step") | Out-Null
        }
        foreach ($field in @('if_present', 'continue_on_error_present', 'working_directory_present')) {
            if (-not (Test-FieldAbsent -Record $gateStep -PresentName $field)) {
                $errors.Add("$Gate gate step must not set $field") | Out-Null
            }
        }
        if (-not (Test-FieldExplicitlyAbsent -Record $gateStep -PresentName 'env_present')) {
            $errors.Add("$Gate gate step must not set env_present") | Out-Null
        }
        if ($Gate -eq 'npm_ci') {
            if ([bool](Get-RecordValue -Record $gateStep -Name 'shell_present' -Default $false)) {
                $errors.Add('npm ci gate must use the Ubuntu default shell (no shell override)') | Out-Null
            }
        } elseif ([string](Get-RecordValue -Record $gateStep -Name 'shell_kind' -Default '') -ne 'string' -or
                  -not [string]::Equals(([string](Get-RecordValue -Record $gateStep -Name 'shell' -Default '')).Trim(), 'pwsh', [System.StringComparison]::OrdinalIgnoreCase)) {
            $errors.Add('secret scan gate must use shell: pwsh') | Out-Null
        }
        if ($checkoutSteps.Count -eq 1 -and
            [int](Get-RecordValue -Record $checkoutSteps[0] -Name 'index' -Default -1) -ge [int](Get-RecordValue -Record $gateStep -Name 'index' -Default -1)) {
            $errors.Add("CI job '$jobName' checkout must precede the $Gate gate") | Out-Null
        }

        # The gate must execute against the exact checkout it just received. An
        # arbitrary run step between checkout and `npm ci`/secret scan can rewrite
        # package metadata or the scan helper while leaving a command-shaped gate
        # behind. Govern that prefix as a fixed, minimal topology.
        $gateIndex = [int](Get-RecordValue -Record $gateStep -Name 'index' -Default -1)
        $checkoutIndex = if ($checkoutSteps.Count -eq 1) { [int](Get-RecordValue -Record $checkoutSteps[0] -Name 'index' -Default -1) } else { -1 }
        $preGateSteps = @($steps | Where-Object {
                [int](Get-RecordValue -Record $_ -Name 'index' -Default -1) -lt $gateIndex
            })
        $expectedPreGateCount = if ($Gate -eq 'npm_ci') { 2 } else { 1 }
        if ($checkoutIndex -ne 0 -or $preGateSteps.Count -ne $expectedPreGateCount) {
            $errors.Add("CI job '$jobName' must use the fixed trusted checkout-to-$Gate topology") | Out-Null
        }
        if (@($steps | Where-Object { -not (Test-FieldExplicitlyAbsent -Record $_ -PresentName 'env_present') }).Count -gt 0) {
            $errors.Add("CI job '$jobName' must not set step env overrides") | Out-Null
        }
        if ($Gate -eq 'npm_ci') {
            $setupSteps = @($steps | Where-Object {
                    [int](Get-RecordValue -Record $_ -Name 'index' -Default -1) -eq 1
                })
            if ($gateIndex -ne 2 -or $setupSteps.Count -ne 1 -or -not (Test-SetupNodeStepContract -Step $setupSteps[0])) {
                $errors.Add("CI job '$jobName' must run exactly checkout, a controlled Node 22 setup, then npm ci before any other executable step") | Out-Null
            }
        } elseif ($gateIndex -ne 1) {
            $errors.Add("CI job '$jobName' must run the secret scan immediately after checkout with no intervening executable step") | Out-Null
        }
    }

    return [pscustomobject]@{ Valid = ($errors.Count -eq 0); Errors = @($errors); Job = $jobName }
}

function Get-ReleaseWorkflowJobExact {
    <#
    .SYNOPSIS
    Finds a governed workflow job by its exact YAML key spelling.

    .DESCRIPTION
    PowerShell hashtable/property access is case-insensitive, while required CI
    check identities can be case-sensitive in remote governance. Enumerate the
    original metadata keys and reject casing drift rather than silently binding
    `Frontend-Gate` to the governed `frontend-gate` contract.
    #>
    param(
        [Parameter(Mandatory = $true)]$WorkflowMetadata,
        [Parameter(Mandatory = $true)][string]$JobName
    )

    if ($null -eq $WorkflowMetadata -or $null -eq $WorkflowMetadata.jobs) { return $null }
    $jobs = $WorkflowMetadata.jobs
    $matches = New-Object System.Collections.Generic.List[object]
    if ($jobs -is [System.Collections.IDictionary]) {
        foreach ($key in @($jobs.Keys)) {
            if ([string]::Equals([string]$key, $JobName, [System.StringComparison]::Ordinal)) {
                $matches.Add($jobs[$key]) | Out-Null
            }
        }
    } elseif ($null -ne $jobs) {
        foreach ($property in @($jobs.PSObject.Properties)) {
            if ([string]::Equals([string]$property.Name, $JobName, [System.StringComparison]::Ordinal)) {
                $matches.Add($property.Value) | Out-Null
            }
        }
    }
    if ($matches.Count -ne 1) { return $null }
    return $matches[0]
}

function Test-ReleasePesterYamlParserReadiness {
    <#
    .SYNOPSIS
    Ensures the Windows Pester job installs a fixed real YAML parser before it
    runs parser-backed release readiness tests.
    #>
    param([Parameter(Mandatory = $true)]$WorkflowMetadata)

    function Get-RecordValue {
        param($Record, [string]$Name, $Default = $null)
        if ($null -ne $Record -and $Record.PSObject.Properties.Name -contains $Name) { return $Record.$Name }
        return $Default
    }
    function Test-FieldAbsent {
        param($Record, [string]$PresentName)
        return (-not [bool](Get-RecordValue -Record $Record -Name $PresentName -Default $false))
    }
    function Test-FieldExplicitlyAbsent {
        param($Record, [string]$PresentName)
        if ($null -eq $Record -or -not ($Record.PSObject.Properties.Name -contains $PresentName)) { return $false }
        return (-not [bool]$Record.$PresentName)
    }
    function Test-StepUnconditionalAtRepositoryRoot {
        param($Step, [string]$Shell = '')
        if ($null -eq $Step) { return $false }
        foreach ($field in @('if_present', 'continue_on_error_present', 'working_directory_present')) {
            if (-not (Test-FieldAbsent -Record $Step -PresentName $field)) { return $false }
        }
        if ([string]::IsNullOrWhiteSpace($Shell)) {
            return ((-not [bool](Get-RecordValue -Record $Step -Name 'shell_present' -Default $false)) -and
                (Test-FieldExplicitlyAbsent -Record $Step -PresentName 'env_present'))
        }
        return ([string](Get-RecordValue -Record $Step -Name 'shell_kind' -Default '') -eq 'string' -and
            [string]::Equals(([string](Get-RecordValue -Record $Step -Name 'shell' -Default '')).Trim(), $Shell, [System.StringComparison]::OrdinalIgnoreCase) -and
            (Test-FieldExplicitlyAbsent -Record $Step -PresentName 'env_present'))
    }
    function Test-RunOnlyStep {
        param($Step)
        return ($null -ne $Step -and
            (Test-FieldExplicitlyAbsent -Record $Step -PresentName 'uses_present') -and
            [bool](Get-RecordValue -Record $Step -Name 'run_present' -Default $false) -and
            [string]::Equals(([string](Get-RecordValue -Record $Step -Name 'run_kind' -Default '')).Trim(), 'string', [System.StringComparison]::Ordinal))
    }
    function Test-UsesOnlyStep {
        param($Step, [Parameter(Mandatory = $true)][string]$ExpectedUses)
        return ($null -ne $Step -and
            [bool](Get-RecordValue -Record $Step -Name 'uses_present' -Default $false) -and
            [string]::Equals(([string](Get-RecordValue -Record $Step -Name 'uses_kind' -Default '')).Trim(), 'string', [System.StringComparison]::Ordinal) -and
            [string]::Equals(([string](Get-RecordValue -Record $Step -Name 'uses' -Default '')).Trim(), $ExpectedUses, [System.StringComparison]::OrdinalIgnoreCase) -and
            (Test-FieldExplicitlyAbsent -Record $Step -PresentName 'run_present'))
    }
    function Test-PinnedPyYamlInstallScript {
        param([AllowEmptyString()][string]$Run)
        $expected = @'
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
        return [string]::Equals((([string]$Run -replace '\s+', ' ').Trim()), (($expected -replace '\s+', ' ').Trim()), [System.StringComparison]::Ordinal)
    }
    function Test-PesterRunnerScript {
        param([AllowEmptyString()][string]$Run)
        $command = Get-ReleaseFlatSingleCommandAst -ScriptText $Run
        if ($null -eq $command) { return $false }
        return Test-ReleasePowerShellFileInvocationExactArguments `
            -CommandAst $command `
            -ExpectedRelativePath 'scripts/tests/run-release-build-tests.ps1' `
                -RequireNoProfile
    }
    function Test-PesterCheckoutStep {
        param($Step)
        if ($null -eq $Step -or
            -not (Test-UsesOnlyStep -Step $Step -ExpectedUses 'actions/checkout@v4') -or
            -not (Test-StepUnconditionalAtRepositoryRoot -Step $Step)) {
            return $false
        }
        $rawKeys = @((Get-RecordValue -Record $Step -Name 'with_raw_keys' -Default @()) | ForEach-Object { ([string]$_).Trim() })
        if (-not [bool](Get-RecordValue -Record $Step -Name 'with_keys_unique' -Default $false)) { return $false }
        foreach ($key in $rawKeys) {
            if (-not [string]::Equals($key, 'fetch-depth', [System.StringComparison]::Ordinal)) { return $false }
        }
        return ($rawKeys.Count -le 1)
    }

    $job = Get-ReleaseWorkflowJobExact -WorkflowMetadata $WorkflowMetadata -JobName 'pester-release-tests'
    if ($null -eq $job -or -not [bool](Get-RecordValue -Record $job -Name 'present' -Default $false) -or
        [string](Get-RecordValue -Record $job -Name 'runs_on_kind' -Default '') -ne 'string' -or
        -not [string]::Equals(([string](Get-RecordValue -Record $job -Name 'runs_on' -Default '')).Trim(), 'windows-latest', [System.StringComparison]::OrdinalIgnoreCase)) {
        return [pscustomobject]@{ Valid = $false; Errors = @('pester-release-tests must be an exact Windows job') }
    }
    foreach ($field in @('if_present', 'continue_on_error_present', 'needs_present', 'env_present', 'container_present', 'services_present', 'defaults_run_shell_present')) {
        if (-not (Test-FieldExplicitlyAbsent -Record $job -PresentName $field)) {
            return [pscustomobject]@{ Valid = $false; Errors = @("pester-release-tests must not set $field") }
        }
    }
    if ([bool](Get-RecordValue -Record $job -Name 'defaults_run_working_directory_present' -Default $true)) {
        return [pscustomobject]@{ Valid = $false; Errors = @('pester-release-tests must use repository-root working directory') }
    }
    foreach ($field in @('WorkflowDefaultsRunWorkingDirectoryPresent', 'WorkflowDefaultsRunShellPresent', 'WorkflowEnvPresent')) {
        if (-not (Test-FieldExplicitlyAbsent -Record $WorkflowMetadata -PresentName $field)) {
            return [pscustomobject]@{ Valid = $false; Errors = @("ci-gates must not set $field for parser-backed Pester execution") }
        }
    }
    $steps = @((Get-RecordValue -Record $job -Name 'steps' -Default @()))
    if ($steps.Count -ne 5) {
        return [pscustomobject]@{ Valid = $false; Errors = @('pester-release-tests is missing required parser setup steps') }
    }
    $checkout = @($steps | Where-Object { [int](Get-RecordValue -Record $_ -Name 'index' -Default -1) -eq 0 })
    $setupPython = @($steps | Where-Object { [int](Get-RecordValue -Record $_ -Name 'index' -Default -1) -eq 1 })
    $install = @($steps | Where-Object { [int](Get-RecordValue -Record $_ -Name 'index' -Default -1) -eq 2 })
    $runner = @($steps | Where-Object { [int](Get-RecordValue -Record $_ -Name 'index' -Default -1) -eq 3 })
    $secretScan = @($steps | Where-Object { [int](Get-RecordValue -Record $_ -Name 'index' -Default -1) -eq 4 })
    $valid = ($checkout.Count -eq 1 -and $setupPython.Count -eq 1 -and $install.Count -eq 1 -and $runner.Count -eq 1 -and $secretScan.Count -eq 1 -and
        (Test-PesterCheckoutStep -Step $checkout[0]) -and
        (Test-UsesOnlyStep -Step $setupPython[0] -ExpectedUses 'actions/setup-python@v5') -and
        (Test-StepUnconditionalAtRepositoryRoot -Step $setupPython[0]) -and
        (Test-RunOnlyStep -Step $install[0]) -and
        (Test-StepUnconditionalAtRepositoryRoot -Step $install[0] -Shell 'pwsh') -and
        (Test-PinnedPyYamlInstallScript -Run ([string](Get-RecordValue -Record $install[0] -Name 'run' -Default ''))) -and
        (Test-RunOnlyStep -Step $runner[0]) -and
        (Test-StepUnconditionalAtRepositoryRoot -Step $runner[0] -Shell 'pwsh') -and
        (Test-PesterRunnerScript -Run ([string](Get-RecordValue -Record $runner[0] -Name 'run' -Default ''))) -and
        (Test-RunOnlyStep -Step $secretScan[0]) -and
        (Test-StepUnconditionalAtRepositoryRoot -Step $secretScan[0] -Shell 'pwsh') -and
        (Test-ReleaseExactSecretScanGateScript -ScriptText ([string](Get-RecordValue -Record $secretScan[0] -Name 'run' -Default ''))))
    return [pscustomobject]@{
        Valid = $valid
        Errors = if ($valid) { @() } else { @('pester-release-tests must install and verify PyYAML==6.0.2 immediately before parser-backed Pester execution') }
    }
}

function Assert-ReleaseWorkflowStaticContract {
    <#
    .SYNOPSIS
    Static governance checks for tracked Gitea release workflows.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot
    )

    $checks = [ordered]@{
        actions_pinned = $false
        npm_ci = $false
        secret_scan = $false
        artifact_retention = $false
        host_only_default = $false
        real_yaml_parser_required = $false
        full_offline_verifier = $false
        least_privilege_permissions = $false
        ci_triggers = $false
        pester_yaml_parser = $false
    }
    $errors = New-Object System.Collections.Generic.List[string]

    $workflowDir = Join-Path $RepoRoot '.gitea\workflows'
    if (-not (Test-Path -LiteralPath $workflowDir -PathType Container)) {
        throw "Workflow directory missing: $workflowDir"
    }

    $files = @(Get-ChildItem -LiteralPath $workflowDir -Filter '*.yml' -File) +
             @(Get-ChildItem -LiteralPath $workflowDir -Filter '*.yaml' -File -ErrorAction SilentlyContinue)
    if ($files.Count -eq 0) {
        throw 'No workflow files found for static contract validation.'
    }

    $allText = ''
    $workflowMetadata = @{}
    $allActionReferenceRecords = @()
    $allRunBlockRecords = @()
    foreach ($f in $files) {
        $text = Get-Content -LiteralPath $f.FullName -Raw
        $allText += "`n" + $text
        $syntax = Test-ReleaseWorkflowSyntax -Path $f.FullName
        if ($syntax.Engine -notmatch 'pyyaml|node-yaml') {
            $errors.Add(("Workflow {0} did not use a real YAML parser (engine={1})" -f $f.Name, $syntax.Engine)) | Out-Null
        }
        if (-not $syntax.Valid) {
            $errors.Add(("Workflow {0} failed YAML validation: {1}" -f $f.Name, ($syntax.Errors -join '; '))) | Out-Null
        }
        $metadata = Test-ReleaseHostEvidenceVerifierOrder -WorkflowPath $f.FullName
        $workflowMetadata[$f.Name] = $metadata
        if ($metadata.Engine -notmatch 'pyyaml|node-yaml') {
            $errors.Add(("Workflow {0} could not extract runtime metadata with a real YAML parser (engine={1})" -f $f.Name, $metadata.Engine)) | Out-Null
        }
        $allActionReferenceRecords += @($metadata.ActionReferences | Where-Object { $null -ne $_ })
        $allRunBlockRecords += @($metadata.RunBlocks | Where-Object { $null -ne $_ })
    }

    $requiredPins = @(
        'actions/checkout@v4',
        'actions/setup-node@v4',
        'dtolnay/rust-toolchain@stable'
    )
    $allowedActionReferences = @(
        'actions/checkout@v4',
        'actions/setup-node@v4',
        'actions/setup-python@v5',
        'actions/upload-artifact@v4',
        'dtolnay/rust-toolchain@stable'
    )
    $actualActionReferences = @()
    $pinOk = $true
    foreach ($actionRecord in $allActionReferenceRecords) {
        $usesKind = if ($actionRecord.PSObject.Properties.Name -contains 'uses_kind') { [string]$actionRecord.uses_kind } else { 'string' }
        $scope = if ($actionRecord.PSObject.Properties.Name -contains 'scope') { [string]$actionRecord.scope } else { 'step' }
        if ($usesKind -ne 'string' -or [string]::IsNullOrWhiteSpace([string]$actionRecord.uses)) {
            $pinOk = $false
            $errors.Add(("Action reference must be a non-empty YAML string (job={0}, index={1})" -f $actionRecord.job, $actionRecord.index)) | Out-Null
            continue
        }
        if (-not [string]::Equals($scope, 'step', [System.StringComparison]::Ordinal)) {
            $pinOk = $false
            $errors.Add(("Reusable/job-level uses is not allowed in release workflows (job={0})" -f $actionRecord.job)) | Out-Null
            continue
        }
        $actualActionReferences += [string]$actionRecord.uses
    }
    foreach ($pin in $requiredPins) {
        if ($actualActionReferences -notcontains $pin) {
            $pinOk = $false
            $errors.Add("Missing pinned action reference: $pin") | Out-Null
        }
    }
    foreach ($actionReference in $actualActionReferences) {
        if ($allowedActionReferences -notcontains $actionReference) {
            $pinOk = $false
            $errors.Add("Action reference is not in the fixed allowlist: $actionReference") | Out-Null
        }
    }
    $checks['actions_pinned'] = $pinOk

    $permissionsOk = $true
    foreach ($workflowName in @($workflowMetadata.Keys)) {
        $metadata = $workflowMetadata[$workflowName]
        $rootEntries = $null
        if ($null -ne $metadata) { $rootEntries = $metadata.RootPermissions }
        $rootKeys = if ($null -ne $metadata) { @($metadata.RootPermissionsRawKeys | ForEach-Object { [string]$_ }) } else { @() }
        $contentsProperties = @()
        if ($null -ne $rootEntries) {
            $contentsProperties = @($rootEntries.PSObject.Properties | Where-Object { [string]::Equals([string]$_.Name, 'contents', [System.StringComparison]::Ordinal) })
        }
        $contentsProperty = if ($contentsProperties.Count -eq 1) { $contentsProperties[0] } else { $null }
        $rootContents = if ($null -ne $contentsProperty) { [string]$contentsProperty.Value } else { '' }
        $rootExact = ($null -ne $metadata -and [bool]$metadata.RootPermissionsPresent -and
            [string]$metadata.RootPermissionsKind -eq 'mapping' -and [bool]$metadata.RootPermissionsKeysUnique -and
            @($rootKeys).Count -eq 1 -and @($rootKeys)[0] -eq 'contents' -and
            [string]::Equals($rootContents.Trim(), 'read', [System.StringComparison]::Ordinal))
        if (-not $rootExact) {
            $permissionsOk = $false
            $errors.Add(("Workflow {0} must declare exactly top-level permissions: contents: read" -f $workflowName)) | Out-Null
        }
        foreach ($jobPermission in @($metadata.JobPermissionRecords)) {
            if ($null -ne $jobPermission -and [bool]$jobPermission.present) {
                $permissionsOk = $false
                $errors.Add(("Workflow {0} job {1} must not override permissions" -f $workflowName, $jobPermission.job)) | Out-Null
            }
        }
    }
    $checks['least_privilege_permissions'] = $permissionsOk

    $ciMetadata = if ($workflowMetadata.ContainsKey('ci-gates.yml')) { $workflowMetadata['ci-gates.yml'] } else { $null }
    # Windows-only gates (Pester, secret scan, workflow-syntax) live in
    # windows-gates.yml, not ci-gates.yml: a job-level `if` guard with no
    # matching runner never reaches `skipped` on Gitea 1.26, so they were moved
    # to a separate dispatch/tag-triggered workflow to keep ci-gates terminal.
    $windowsMetadata = if ($workflowMetadata.ContainsKey('windows-gates.yml')) { $workflowMetadata['windows-gates.yml'] } else { $null }
    if ($null -eq $windowsMetadata) {
        $errors.Add('windows-gates.yml is missing; Windows-only release gates have no workflow.') | Out-Null
    }
    $ciTriggers = if ($null -ne $ciMetadata -and $ciMetadata.PSObject.Properties.Name -contains 'WorkflowTriggers') {
        @($ciMetadata.WorkflowTriggers | ForEach-Object { [string]$_ })
    } else { @() }
    $ciOnPresent = ($null -ne $ciMetadata -and $ciMetadata.PSObject.Properties.Name -contains 'WorkflowOnPresent' -and [bool]$ciMetadata.WorkflowOnPresent)
    $ciOnKind = if ($null -ne $ciMetadata -and $ciMetadata.PSObject.Properties.Name -contains 'WorkflowOnKind') { [string]$ciMetadata.WorkflowOnKind } else { '' }
    $checks['ci_triggers'] = ($ciOnPresent -and $ciOnKind -eq 'mapping' -and
        $ciTriggers -contains 'push' -and $ciTriggers -contains 'pull_request')
    if (-not $checks['ci_triggers']) {
        $errors.Add('ci-gates.yml must declare mapping triggers for both push and pull_request.') | Out-Null
    }
    $npmGate = Test-ReleaseCiGateJobContract -WorkflowMetadata $ciMetadata -Gate 'npm_ci'
    $checks['npm_ci'] = [bool]$npmGate.Valid
    if (-not $checks['npm_ci']) {
        foreach ($gateError in @($npmGate.Errors)) { $errors.Add([string]$gateError) | Out-Null }
    }
    $hasNpmInstall = @($allRunBlockRecords | Where-Object {
            Test-ReleaseRunContainsTopLevelNpmCommand -ScriptText ([string]$_.run) -Subcommand 'install'
        }).Count -gt 0
    if ($hasNpmInstall) {
        $checks['npm_ci'] = $false
        $errors.Add('Workflows must not use npm install for release gates.') | Out-Null
    }

    # secret_scan and pester_yaml_parser gates now bind to windows-gates.yml,
    # where those jobs actually live. The gate semantics are unchanged: the
    # job must still exist, run on windows-latest, and follow the controlled
    # step contract; it is not disguised as executed by a push.
    if ($null -eq $windowsMetadata) {
        $checks['secret_scan'] = $false
        $checks['pester_yaml_parser'] = $false
    } else {
        $secretGate = Test-ReleaseCiGateJobContract -WorkflowMetadata $windowsMetadata -Gate 'secret_scan'
        $checks['secret_scan'] = [bool]$secretGate.Valid
        if (-not $checks['secret_scan']) {
            foreach ($gateError in @($secretGate.Errors)) { $errors.Add([string]$gateError) | Out-Null }
        }

        $pesterParser = Test-ReleasePesterYamlParserReadiness -WorkflowMetadata $windowsMetadata
        $checks['pester_yaml_parser'] = [bool]$pesterParser.Valid
        if (-not $checks['pester_yaml_parser']) {
            foreach ($parserError in @($pesterParser.Errors)) { $errors.Add([string]$parserError) | Out-Null }
        }
    }

    $hostWf = Join-Path $workflowDir 'release-host-evidence.yml'
    if (Test-Path -LiteralPath $hostWf) {
        $hostMetadata = $workflowMetadata['release-host-evidence.yml']
        # Job/step-order aware check via a real YAML parser. Fresh producer
        # binding prevents a reusable runner workspace from selecting an older
        # internally-valid evidence directory after the build step is skipped.
        $order = Test-ReleaseHostEvidenceVerifierOrder `
            -WorkflowPath $hostWf `
            -RequireRetentionDays14 `
            -RequireCheckout `
            -RequireFreshProducer
        $checks['full_offline_verifier'] = [bool]$order.Valid
        $skipDefaultIsTrue = $false
        if ($null -ne $hostMetadata -and [bool]$hostMetadata.SkipBundleDefaultPresent) {
            # Avoid YAML 1.1/1.2 ambiguity (yes/on/off): the dispatch input is
            # deliberately the literal string 'true'.
            $skipDefaultIsTrue = ([string]$hostMetadata.SkipBundleDefaultKind -eq 'string' -and
                [string]::Equals(([string]$hostMetadata.SkipBundleDefault).Trim(), 'true', [System.StringComparison]::Ordinal))
        }
        $windowsHostProducerOk = $false
        if ($order.jobs.ContainsKey('windows-host-evidence')) {
            $windowsHostProducerOk = [bool]$order.jobs['windows-host-evidence'].ProducerHostOnlyOk
        }
        $hostOnly = $skipDefaultIsTrue -and $windowsHostProducerOk
        $checks['host_only_default'] = $hostOnly
        if (-not $hostOnly) {
            $errors.Add('release-host-evidence must default to host-only (skip_bundle=true, no -BuildApk).') | Out-Null
        }

        $retentionOk = $true
        foreach ($hostJobName in @('windows-host-evidence', 'android-host-evidence')) {
            if (-not $order.jobs.ContainsKey($hostJobName) -or -not [bool]$order.jobs[$hostJobName].RetentionOk) {
                $retentionOk = $false
                break
            }
        }
        $checks['artifact_retention'] = $retentionOk
        if (-not $retentionOk) {
            $errors.Add('Each controlled host evidence upload must set retention-days: 14.') | Out-Null
        }
        if (-not $order.Valid) {
            foreach ($e in @($order.Errors)) {
                $errors.Add([string]$e) | Out-Null
            }
            if ($order.Engine -eq 'none') {
                $errors.Add('release-host-evidence full offline verifier order requires a real YAML parser.') | Out-Null
            }
        }
    } else {
        $errors.Add('release-host-evidence.yml is missing.') | Out-Null
        $checks['full_offline_verifier'] = $false
    }

    $ciWf = Join-Path $workflowDir 'ci-gates.yml'
    if (-not (Test-Path -LiteralPath $ciWf)) {
        $errors.Add('ci-gates.yml is missing.') | Out-Null
    }
    # The real-YAML-parser requirement (PyYAML==6.0.2 install + Test-ReleaseWorkflowSyntax
    # invocation + pyyaml|node-yaml engine check) now lives in windows-gates.yml,
    # which is where the workflow-syntax job runs. Search across all workflow text
    # so the requirement is enforced regardless of which file holds the job.
    $checks['real_yaml_parser_required'] = ($allText -match 'PyYAML' -and
        $allText -match 'Test-ReleaseWorkflowSyntax' -and
        $allText -match 'pyyaml\|node-yaml')
    if (-not $checks['real_yaml_parser_required']) {
        $errors.Add('A release workflow must require a real YAML parser for workflow syntax validation.') | Out-Null
    }

    return [pscustomobject]@{
        Valid = ($errors.Count -eq 0)
        ErrorCount = $errors.Count
        Errors = @($errors)
        # Hashtable so callers can index checks['name'] under Windows PowerShell 5.
        checks = [hashtable]$checks
    }
}
