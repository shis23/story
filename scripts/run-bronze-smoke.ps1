<#
.SYNOPSIS
Runs deterministic Bronze release evidence smokes without a paid LLM.

.DESCRIPTION
Exercises the release-checklist Bronze backend contracts that can be proven
without a desktop GUI or real model:
  - bronze_deterministic harness (B1/B2/B3/B4 store-level + recover/hash/regen)
  - writeback isolation presence/collision regressions
  - B5 Meta backend smoke
  - diagnostic/export redaction tests already covered by tauri-app lib filters

This script is not a substitute for real Tauri GUI screenshots or real LLM
T1/T2/T3 quality evidence.

.PARAMETER DryRun
Prints the cargo commands without running them.

.EXAMPLE
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-bronze-smoke.ps1
#>
[CmdletBinding()]
param(
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
$script:StepNumber = 0
$TotalSteps = 4

function Find-RepoRoot {
    $start = (Get-Location).ProviderPath
    $gitRoot = (& git -C $start rev-parse --show-toplevel 2>$null)
    if ($LASTEXITCODE -eq 0 -and $gitRoot) {
        return (Resolve-Path -LiteralPath $gitRoot).ProviderPath
    }

    throw "Unable to locate the StoryForge repository root from '$start'."
}

function Format-Command {
    param([Parameter(Mandatory = $true)][string[]]$Command)

    return ($Command | ForEach-Object {
        if ($_ -match '\s') {
            '"' + ($_ -replace '"', '\"') + '"'
        } else {
            $_
        }
    }) -join ' '
}

function Ensure-FrontendDistPlaceholder {
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    $distDir = Join-Path $RepoRoot 'frontend\dist'
    $indexPath = Join-Path $distDir 'index.html'
    if (Test-Path -LiteralPath $indexPath) {
        return
    }

    New-Item -ItemType Directory -Force -Path $distDir | Out-Null
    @(
        '<!doctype html>'
        '<html><head><meta charset="utf-8"><title>StoryForge test placeholder</title></head>'
        '<body><div id="app">StoryForge test placeholder</div></body></html>'
    ) | Set-Content -LiteralPath $indexPath -Encoding utf8
    Write-Host "Created local frontend/dist placeholder for Tauri build context." -ForegroundColor Yellow
}

function Invoke-CargoSmoke {
    param(
        [Parameter(Mandatory = $true)][string]$Label,
        [Parameter(Mandatory = $true)][string[]]$Command,
        [Parameter(Mandatory = $true)][string]$RepoRoot
    )

    $script:StepNumber += 1
    Write-Host ''
    Write-Host ("[{0}/{1}] {2}" -f $script:StepNumber, $TotalSteps, $Label) -ForegroundColor Cyan
    if ($DryRun) {
        Write-Host ("DRY RUN: cd {0}" -f $RepoRoot)
        Write-Host ("DRY RUN: {0}" -f (Format-Command -Command $Command))
        return
    }

    Push-Location -LiteralPath $RepoRoot
    try {
        Write-Host ("RUN: {0}" -f (Format-Command -Command $Command))
        $output = & $Command[0] $Command[1..($Command.Count - 1)] 2>&1
        $exitCode = $LASTEXITCODE
        $output | Out-Host
        if ($exitCode -ne 0) {
            throw "Bronze smoke step '$Label' failed with exit code $exitCode."
        }
        # Gate 8 审查 P2-D4: filter 无匹配时 cargo 报 "0 tests" 且 exit 0，
        # 脚本会静默假绿——显式断言至少运行 1 个测试。
        $outputText = $output -join "`n"
        if ($outputText -match 'running 0 tests' -or $outputText -match 'test result: ok\. 0 passed') {
            throw "Bronze smoke step '$Label' ran 0 tests; update or remove the stale filter/command."
        }
    } finally {
        Pop-Location
    }
}

try {
    $repoRoot = Find-RepoRoot
    $timestamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $artifactDir = Join-Path $repoRoot ("artifacts\bronze\{0}" -f $timestamp)
    New-Item -ItemType Directory -Force -Path $artifactDir | Out-Null

    Write-Host ("StoryForge Bronze smoke root: {0}" -f $repoRoot)
    Write-Host ("Artifacts: {0}" -f $artifactDir)
    if ($DryRun) {
        Write-Host 'Dry run enabled; commands will be printed but not executed.'
    } else {
        Ensure-FrontendDistPlaceholder -RepoRoot $repoRoot
    }

    Invoke-CargoSmoke `
        -RepoRoot $repoRoot `
        -Label 'bronze deterministic store/turn evidence' `
        -Command @('cargo', 'test', '-p', 'harness-real-llm', '--test', 'bronze_deterministic', '--', '--nocapture')

    Invoke-CargoSmoke `
        -RepoRoot $repoRoot `
        -Label 'writeback isolation / same-name collision regressions' `
        -Command @('cargo', 'test', '-p', 'harness-real-llm', '--test', 'writeback_isolation', '--', '--nocapture')

    Invoke-CargoSmoke `
        -RepoRoot $repoRoot `
        -Label 'B5 Meta backend smoke' `
        -Command @('powershell', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $repoRoot 'scripts\run-meta-smoke.ps1'))

    Invoke-CargoSmoke `
        -RepoRoot $repoRoot `
        -Label 'diagnostic export redaction (B6 auto subitem)' `
        -Command @('cargo', 'test', '-p', 'storyforge', '--lib', 'test_diagnostic', '--', '--nocapture')

    $summaryPath = Join-Path $artifactDir 'SUMMARY.txt'
    @(
        "StoryForge Bronze smoke"
        "timestamp=$timestamp"
        "root=$repoRoot"
        "steps=bronze_deterministic,writeback_isolation,meta_smoke,diagnostic_redaction"
        "result=passed"
        "note=Not a substitute for real Tauri GUI screenshots or paid-model T1/T2/T3 quality evidence."
    ) | Set-Content -LiteralPath $summaryPath -Encoding utf8

    Write-Host ''
    Write-Host 'Bronze smoke passed.' -ForegroundColor Green
    Write-Host ("Summary: {0}" -f $summaryPath)
    exit 0
} catch {
    Write-Host ''
    Write-Error $_.Exception.Message
    exit 1
}
