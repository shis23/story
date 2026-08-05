<#
.SYNOPSIS
Runs deterministic B5 Meta Agent smoke checks.

.DESCRIPTION
Exercises the release-checklist B5 backend contract without a desktop UI or LLM:
generation provenance explanation, campaign repair proposal, typed patch preview,
accept, and dismiss.

.PARAMETER DryRun
Prints the cargo commands without running them.

.EXAMPLE
powershell -ExecutionPolicy Bypass -File scripts/run-meta-smoke.ps1
#>
[CmdletBinding()]
param(
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
$script:StepNumber = 0

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

function Invoke-CargoSmoke {
    param(
        [Parameter(Mandatory = $true)][string]$Label,
        [Parameter(Mandatory = $true)][string]$Filter,
        [Parameter(Mandatory = $true)][string]$RepoRoot
    )

    $script:StepNumber += 1
    $command = @('cargo', 'test', '-p', 'storyforge', $Filter, '--lib')

    Write-Host ''
    Write-Host ("[{0}/4] {1}" -f $script:StepNumber, $Label) -ForegroundColor Cyan
    if ($DryRun) {
        Write-Host ("DRY RUN: cd {0}" -f $RepoRoot)
        Write-Host ("DRY RUN: {0}" -f (Format-Command -Command $command))
        return
    }

    Push-Location -LiteralPath $RepoRoot
    try {
        Write-Host ("RUN: {0}" -f (Format-Command -Command $command))
        $output = & $command[0] $command[1..($command.Count - 1)] 2>&1
        $exitCode = $LASTEXITCODE
        $output | Out-Host
        if ($exitCode -ne 0) {
            throw "Meta smoke step '$Label' failed with exit code $exitCode."
        }
        # Gate 8 审查 P2-D4: filter 无匹配时 cargo 报 "0 tests" 且 exit 0，
        # 脚本会静默假绿——显式断言至少运行 1 个测试。
        $outputText = $output -join "`n"
        if ($outputText -match 'running 0 tests' -or $outputText -match 'test result: ok\. 0 passed') {
            throw "Meta smoke step '$Label' ran 0 tests (filter '$Filter' matched nothing); update or remove the stale filter."
        }
    } finally {
        Pop-Location
    }
}

try {
    $repoRoot = Find-RepoRoot
    Write-Host ("StoryForge B5 Meta smoke root: {0}" -f $repoRoot)
    if ($DryRun) {
        Write-Host 'Dry run enabled; commands will be printed but not executed.'
    }

    Invoke-CargoSmoke -RepoRoot $repoRoot -Label 'generation provenance explain' -Filter 'test_conv_generation_explainer_reads_provenance_async'
    Invoke-CargoSmoke -RepoRoot $repoRoot -Label 'campaign health repair proposal' -Filter 'test_meta_propose_campaign_repairs_orphan_knowledge'
    Invoke-CargoSmoke -RepoRoot $repoRoot -Label 'typed patch preview and accept' -Filter 'test_meta_accept_typed_patch_prune_task_refs'
    Invoke-CargoSmoke -RepoRoot $repoRoot -Label 'typed patch dismiss' -Filter 'test_meta_dismiss_typed_patch'

    Write-Host ''
    Write-Host 'B5 Meta smoke passed.' -ForegroundColor Green
    exit 0
} catch {
    Write-Host ''
    Write-Error $_.Exception.Message
    exit 1
}
