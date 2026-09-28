<#
.SYNOPSIS
Verifies that a release tag matches the application manifest version (G-07).

.DESCRIPTION
G-07 (review-2026-09-13): .github/workflows/release.yml builds and publishes a
release for any pushed v* tag without checking that the tag agrees with the
application version that is about to be shipped. This script is the offline,
unit-testable check used by that workflow and by
scripts/tests/ReleaseBuild.TagVersion.Tests.ps1.

Version sources (both are read; they must agree with each other):
  - crates/tauri-app/tauri.conf.json  -> "version"
  - Cargo.toml                        -> [workspace.package] version
    (crates/tauri-app/Cargo.toml inherits it via version.workspace = true)

Rules:
  - the tag may carry a leading v/V prefix; it is stripped before comparing,
  - the tag must parse as MAJOR.MINOR.PATCH with an optional -prerelease suffix,
  - the tag numeric core must equal the manifest numeric core,
  - a tag-only prerelease suffix is allowed (v0.1.2-rc.1 for manifest 0.1.2),
  - when the manifest itself carries a prerelease suffix, the tag must carry
    the same one,
  - missing files, unparsable values and source drift all fail closed.

Exit codes:
  0 = tag matches every version source
  1 = mismatch, drift, unparsable value, or missing source
  2 = usage error (empty -Tag)

.EXAMPLE
pwsh -NoProfile -File scripts/release-build/verify-tag-version.ps1 -Tag v0.1.2

.EXAMPLE
pwsh -NoProfile -File scripts/release-build/verify-tag-version.ps1 -Tag v0.1.2 -RepoRoot C:\repo
#>
[CmdletBinding()]
param(
    [string]$Tag = '',
    [string]$RepoRoot = '',
    [switch]$PassThru
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

function ConvertTo-ReleaseVersionParts {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$Value,
        [switch]$KeepPrefix
    )

    $text = $Value.Trim()
    if (-not $KeepPrefix -and $text -match '^[vV]') {
        $text = $text.Substring(1)
    }

    $match = [regex]::Match($text, '^(?<base>\d+\.\d+\.\d+)(?:-(?<pre>[0-9A-Za-z][0-9A-Za-z.\-]*))?$')
    if (-not $match.Success) {
        return $null
    }

    return [pscustomobject]@{
        Raw        = $text
        Base       = $match.Groups['base'].Value
        Prerelease = $match.Groups['pre'].Value
    }
}

function Resolve-ReleaseRepoRoot {
    [CmdletBinding()]
    param([string]$Explicit)

    if (-not [string]::IsNullOrWhiteSpace($Explicit)) {
        if (-not (Test-Path -LiteralPath $Explicit)) {
            throw "repository root does not exist: $Explicit"
        }
        return (Resolve-Path -LiteralPath $Explicit).ProviderPath
    }

    $gitRoot = (& git rev-parse --show-toplevel 2>$null)
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace([string]$gitRoot)) {
        throw 'unable to locate the repository root; pass -RepoRoot'
    }
    return (Resolve-Path -LiteralPath ([string]$gitRoot).Trim()).ProviderPath
}

function Read-ReleaseVersionSources {
    [CmdletBinding()]
    param([Parameter(Mandatory = $true)][string]$Root)

    $sources = New-Object 'System.Collections.Specialized.OrderedDictionary'

    $confPath = Join-Path $Root 'crates\tauri-app\tauri.conf.json'
    if (-not (Test-Path -LiteralPath $confPath)) {
        throw "missing version source: $confPath"
    }
    $confRaw = [System.IO.File]::ReadAllText($confPath)
    $confMatch = [regex]::Match($confRaw, '"version"\s*:\s*"(?<value>[^"]+)"')
    if (-not $confMatch.Success) {
        throw "no version field in $confPath"
    }
    $sources['crates/tauri-app/tauri.conf.json'] = $confMatch.Groups['value'].Value

    $cargoPath = Join-Path $Root 'Cargo.toml'
    if (-not (Test-Path -LiteralPath $cargoPath)) {
        throw "missing version source: $cargoPath"
    }
    $cargoRaw = [System.IO.File]::ReadAllText($cargoPath)
    $workspaceSection = [regex]::Match($cargoRaw, '(?ms)^\[workspace\.package\](?<body>.*?)(?=^\[|\z)')
    $cargoMatch = $null
    if ($workspaceSection.Success) {
        $cargoMatch = [regex]::Match($workspaceSection.Groups['body'].Value, '(?m)^\s*version\s*=\s*"(?<value>[^"]+)"')
    }
    if ($null -eq $cargoMatch -or -not $cargoMatch.Success) {
        $cargoMatch = [regex]::Match($cargoRaw, '(?m)^\s*version\s*=\s*"(?<value>[^"]+)"')
    }
    if (-not $cargoMatch.Success) {
        throw "no workspace version in $cargoPath"
    }
    $sources['Cargo.toml (workspace.package)'] = $cargoMatch.Groups['value'].Value

    return $sources
}

function New-ReleaseTagVersionResult {
    [CmdletBinding()]
    param(
        [bool]$Ok,
        [string]$TagValue,
        [string]$TagVersion,
        [string]$TagBase,
        [string]$TagPrerelease,
        [string]$Version,
        [string]$Base,
        [string]$Prerelease,
        [object]$Sources,
        [string]$Reason
    )

    return [pscustomobject]@{
        Ok           = $Ok
        Tag          = $TagValue
        TagVersion   = $TagVersion
        TagBase      = $TagBase
        TagPrerelease = $TagPrerelease
        Version      = $Version
        Base         = $Base
        Prerelease   = $Prerelease
        Sources      = $Sources
        Reason       = $Reason
    }
}

function Invoke-ReleaseTagVersionCheck {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$TagValue,
        [Parameter(Mandatory = $true)][string]$Root
    )

    $sources = Read-ReleaseVersionSources -Root $Root

    $distinct = @($sources.Values | Sort-Object -Unique)
    if (@($distinct).Count -ne 1) {
        $pairs = (@($sources.Keys | ForEach-Object { "$_=$($sources[$_])" })) -join ', '
        return New-ReleaseTagVersionResult -Ok $false -TagValue $TagValue -TagVersion '' -TagBase '' `
            -TagPrerelease '' -Version '' -Base '' -Prerelease '' -Sources $sources `
            -Reason "version sources disagree: $pairs"
    }

    $manifest = ConvertTo-ReleaseVersionParts -Value ([string]$distinct[0]) -KeepPrefix
    if ($null -eq $manifest) {
        return New-ReleaseTagVersionResult -Ok $false -TagValue $TagValue -TagVersion '' -TagBase '' `
            -TagPrerelease '' -Version ([string]$distinct[0]) -Base '' -Prerelease '' -Sources $sources `
            -Reason "manifest version is not MAJOR.MINOR.PATCH[-prerelease]: $($distinct[0])"
    }

    $tag = ConvertTo-ReleaseVersionParts -Value $TagValue
    if ($null -eq $tag) {
        return New-ReleaseTagVersionResult -Ok $false -TagValue $TagValue -TagVersion '' -TagBase '' `
            -TagPrerelease '' -Version $manifest.Raw -Base $manifest.Base -Prerelease $manifest.Prerelease `
            -Sources $sources -Reason "tag is not MAJOR.MINOR.PATCH[-prerelease]: $TagValue"
    }

    $reason = ''
    if ($tag.Base -ne $manifest.Base) {
        $reason = "tag version $($tag.Base) does not match manifest version $($manifest.Base)"
    } elseif ($manifest.Prerelease -ne '' -and $tag.Prerelease -ne $manifest.Prerelease) {
        $reason = "tag prerelease '$($tag.Prerelease)' does not match manifest prerelease '$($manifest.Prerelease)'"
    }

    return New-ReleaseTagVersionResult -Ok ($reason -eq '') -TagValue $TagValue -TagVersion $tag.Raw `
        -TagBase $tag.Base -TagPrerelease $tag.Prerelease -Version $manifest.Raw -Base $manifest.Base `
        -Prerelease $manifest.Prerelease -Sources $sources -Reason $reason
}

if ([string]::IsNullOrWhiteSpace($Tag)) {
    Write-Host '[verify-tag-version] usage: -Tag <tag> [-RepoRoot <path>]'
    exit 2
}

try {
    $resolvedRoot = Resolve-ReleaseRepoRoot -Explicit $RepoRoot
    $verification = Invoke-ReleaseTagVersionCheck -TagValue $Tag -Root $resolvedRoot
} catch {
    Write-Host ("[verify-tag-version] FAIL {0}" -f $_.Exception.Message)
    exit 1
}

Write-Host ("[verify-tag-version] repo={0}" -f $resolvedRoot)
Write-Host ("[verify-tag-version] tag={0} tagVersion={1}" -f $verification.Tag, $verification.TagVersion)
foreach ($sourceName in @($verification.Sources.Keys)) {
    Write-Host ("[verify-tag-version] source {0} = {1}" -f $sourceName, $verification.Sources[$sourceName])
}

if ($verification.Ok) {
    Write-Host ("[verify-tag-version] OK tag {0} matches manifest version {1}" -f $verification.Tag, $verification.Version)
    if ($PassThru) {
        $verification
    }
    exit 0
}

Write-Host ("[verify-tag-version] FAIL {0}" -f $verification.Reason)
exit 1
