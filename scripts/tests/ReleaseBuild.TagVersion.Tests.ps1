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

$VerifyScript = Join-Path $RepoRoot 'scripts\release-build\verify-tag-version.ps1'
if (-not (Test-Path -LiteralPath $VerifyScript)) {
    throw "Missing verify-tag-version.ps1 at $VerifyScript"
}

function Write-TagVersionFixtureFile {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Content
    )
    $dir = Split-Path -Parent $Path
    if (-not (Test-Path -LiteralPath $dir)) {
        $null = New-Item -ItemType Directory -Path $dir -Force
    }
    $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($Path, $Content, $utf8NoBom)
}

function New-TagVersionFixtureRepo {
    param(
        [string]$ConfVersion = '0.1.2',
        [string]$WorkspaceVersion = $ConfVersion,
        [switch]$SkipConf,
        [switch]$SkipCargo
    )

    $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("sf-tagver-{0}" -f [guid]::NewGuid().ToString('N'))
    $null = New-Item -ItemType Directory -Path $dir -Force

    if (-not $SkipConf) {
        $conf = @(
            '{'
            '  "productName": "storyforge",'
            ('  "version": "' + $ConfVersion + '",')
            '  "identifier": "com.storyforge.app"'
            '}'
        ) -join "`r`n"
        Write-TagVersionFixtureFile -Path (Join-Path $dir 'crates\tauri-app\tauri.conf.json') -Content ($conf + "`r`n")
    }

    if (-not $SkipCargo) {
        $cargo = @(
            '[workspace]'
            'resolver = "2"'
            ''
            '[workspace.package]'
            ('version = "' + $WorkspaceVersion + '"')
            'edition = "2024"'
        ) -join "`r`n"
        Write-TagVersionFixtureFile -Path (Join-Path $dir 'Cargo.toml') -Content ($cargo + "`r`n")
    }

    return $dir
}

function Remove-TagVersionFixtureRepo {
    param([string]$Path)
    if ($Path -and (Test-Path -LiteralPath $Path)) {
        Remove-Item -LiteralPath $Path -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Invoke-TagVersionScript {
    param(
        [string]$Tag,
        [string]$Root,
        [switch]$OmitTag
    )

    $allArgs = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $VerifyScript)
    if (-not $OmitTag) {
        $allArgs += @('-Tag', $Tag)
    }
    if ($Root) {
        $allArgs += @('-RepoRoot', $Root)
    }

    $output = & powershell.exe @allArgs 2>&1
    return [pscustomobject]@{
        ExitCode = $LASTEXITCODE
        Output   = (@($output) -join "`n")
    }
}

Describe 'ReleaseBuild release tag vs manifest version (G-07)' {
    It 'accepts a v-prefixed tag that matches the manifest version' {
        $repo = New-TagVersionFixtureRepo -ConfVersion '0.1.2'
        try {
            $result = Invoke-TagVersionScript -Tag 'v0.1.2' -Root $repo
            $result.ExitCode | Should Be 0
            $result.Output | Should Match 'verify-tag-version\] OK'
            $result.Output | Should Match 'source crates/tauri-app/tauri\.conf\.json = 0\.1\.2'
            $result.Output | Should Match 'source Cargo\.toml \(workspace\.package\) = 0\.1\.2'
        } finally {
            Remove-TagVersionFixtureRepo -Path $repo
        }
    }

    It 'accepts a tag without the v prefix' {
        $repo = New-TagVersionFixtureRepo -ConfVersion '0.1.2'
        try {
            $result = Invoke-TagVersionScript -Tag '0.1.2' -Root $repo
            $result.ExitCode | Should Be 0
            $result.Output | Should Match 'tagVersion=0\.1\.2'
        } finally {
            Remove-TagVersionFixtureRepo -Path $repo
        }
    }

    It 'accepts a prerelease suffix on the tag for a release manifest version' {
        $repo = New-TagVersionFixtureRepo -ConfVersion '0.1.2'
        try {
            $result = Invoke-TagVersionScript -Tag 'v0.1.2-rc.1' -Root $repo
            $result.ExitCode | Should Be 0
            $result.Output | Should Match 'tagVersion=0\.1\.2-rc\.1'
        } finally {
            Remove-TagVersionFixtureRepo -Path $repo
        }
    }

    It 'rejects a tag whose version differs from the manifest' {
        $repo = New-TagVersionFixtureRepo -ConfVersion '0.1.2'
        try {
            $result = Invoke-TagVersionScript -Tag 'v0.1.3' -Root $repo
            $result.ExitCode | Should Be 1
            $result.Output | Should Match 'FAIL tag version 0\.1\.3 does not match manifest version 0\.1\.2'
        } finally {
            Remove-TagVersionFixtureRepo -Path $repo
        }
    }

    It 'rejects manifest drift between tauri.conf.json and the workspace version' {
        $repo = New-TagVersionFixtureRepo -ConfVersion '0.1.2' -WorkspaceVersion '0.1.3'
        try {
            $result = Invoke-TagVersionScript -Tag 'v0.1.2' -Root $repo
            $result.ExitCode | Should Be 1
            $result.Output | Should Match 'FAIL version sources disagree'
        } finally {
            Remove-TagVersionFixtureRepo -Path $repo
        }
    }

    It 'rejects a tag that is not MAJOR.MINOR.PATCH' {
        $repo = New-TagVersionFixtureRepo -ConfVersion '0.1.2'
        try {
            $result = Invoke-TagVersionScript -Tag 'v1.2' -Root $repo
            $result.ExitCode | Should Be 1
            $result.Output | Should Match 'FAIL tag is not MAJOR\.MINOR\.PATCH'
        } finally {
            Remove-TagVersionFixtureRepo -Path $repo
        }
    }

    It 'requires the tag to repeat a prerelease suffix that is in the manifest' {
        $repo = New-TagVersionFixtureRepo -ConfVersion '0.1.2-rc.1'
        try {
            $result = Invoke-TagVersionScript -Tag 'v0.1.2' -Root $repo
            $result.ExitCode | Should Be 1
            $result.Output | Should Match 'FAIL tag prerelease'
        } finally {
            Remove-TagVersionFixtureRepo -Path $repo
        }
    }

    It 'fails closed when a version source file is missing' {
        $repo = New-TagVersionFixtureRepo -SkipConf
        try {
            $result = Invoke-TagVersionScript -Tag 'v0.1.2' -Root $repo
            $result.ExitCode | Should Be 1
            $result.Output | Should Match 'FAIL missing version source'
        } finally {
            Remove-TagVersionFixtureRepo -Path $repo
        }
    }

    It 'exits 2 with usage text when no tag is supplied' {
        $result = Invoke-TagVersionScript -OmitTag -Root $RepoRoot
        $result.ExitCode | Should Be 2
        $result.Output | Should Match 'usage: -Tag <tag>'
    }

    It 'matches the real repository manifests for the current application version' {
        $confRaw = [System.IO.File]::ReadAllText((Join-Path $RepoRoot 'crates\tauri-app\tauri.conf.json'))
        $match = [regex]::Match($confRaw, '"version"\s*:\s*"(?<value>[^"]+)"')
        $match.Success | Should Be $true
        $currentVersion = $match.Groups['value'].Value

        $result = Invoke-TagVersionScript -Tag ('v' + $currentVersion) -Root $RepoRoot
        $result.ExitCode | Should Be 0
        $result.Output | Should Match 'verify-tag-version\] OK'
    }
}
