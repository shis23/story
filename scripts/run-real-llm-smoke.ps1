<#
.SYNOPSIS
Runs selected ignored real-LLM smoke suites for StoryForge.

.DESCRIPTION
Runs ignored tests in the harness-real-llm crate using explicit LLM environment
credentials. The runner requires LLM_BASE_URL, LLM_API_KEY, and LLM_MODEL to be
set in the current environment so the tests do not fall back to local connection
files. LLM_TOOL_MODE is optional and is passed through when present.

.PARAMETER Suite
Comma-separated suite names to run. Available suites:
knowledge, t1, t2, t3, c1, c6, c7, i1, all. Defaults to knowledge.

.PARAMETER List
Prints available suites and exits.

.PARAMETER DryRun
Prints the cargo commands without running them.

.PARAMETER ContinueOnFailure
Runs remaining suites after a failure. By default the runner fails fast.

.EXAMPLE
powershell -ExecutionPolicy Bypass -File scripts/run-real-llm-smoke.ps1 -List

.EXAMPLE
powershell -ExecutionPolicy Bypass -File scripts/run-real-llm-smoke.ps1 -DryRun

.EXAMPLE
powershell -ExecutionPolicy Bypass -File scripts/run-real-llm-smoke.ps1 -Suite knowledge,t1

.EXAMPLE
powershell -ExecutionPolicy Bypass -File scripts/run-real-llm-smoke.ps1 -Suite all -ContinueOnFailure
#>
[CmdletBinding()]
param(
    [string]$Suite = 'knowledge',
    [switch]$List,
    [switch]$DryRun,
    [switch]$ContinueOnFailure
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

$script:StepNumber = 0

$SuiteDefinitions = [ordered]@{
    knowledge = @{
        Description = 'Knowledge propagation postprocess/writeback real LLM'
        Filter = 'knowledge_propagation'
    }
    t1 = @{
        Description = 'First-turn campaign writing real LLM'
        Filter = 't1_'
    }
    t2 = @{
        Description = 'Multi-turn append real LLM'
        Filter = 't2_'
    }
    t3 = @{
        Description = 'Regenerate flows real LLM'
        Filter = 't3_'
    }
    c1 = @{
        Description = 'Command layer character extraction real LLM'
        Filter = 'c1_extract_characters_real_llm'
    }
    c6 = @{
        Description = 'Command layer meta chat real LLM'
        Filter = 'c6_meta_chat_real_llm'
    }
    c7 = @{
        Description = 'Command layer MVU analysis real LLM'
        Filter = 'c7_mvu_analyze_real_llm'
    }
    i1 = @{
        Description = 'Adversarial knowledge boundary real LLM'
        Filter = 'i1_adversarial_knowledge_boundary'
    }
    m5 = @{
        Description = 'M5 memory/cache/far-floor/compress real LLM matrix'
        Filter = 'm5_'
    }
    eval = @{
        Description = 'M5 evidence: production pipeline write + synthetic Chronicle fixture + faithful Accept across H_anchor+E'
        Filter = 'eval_real_llm_pipeline_write_synthetic_chronicle_accept_across_epoch'
    }
    endurance = @{
        Description = '100-turn endurance: production pipeline + faithful Accept + checkpoint/resume + sanitized evidence'
        Filter = 'endurance_real_llm'
    }
}

function Find-RepoRoot {
    $start = (Get-Location).ProviderPath

    try {
        $gitRoot = (& git -C $start rev-parse --show-toplevel 2>$null)
        if ($LASTEXITCODE -eq 0 -and $gitRoot) {
            return (Resolve-Path -LiteralPath $gitRoot).ProviderPath
        }
    } catch {
        # Fall through to marker-based discovery.
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

function Format-Command {
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

function Format-Timestamp {
    param(
        [Parameter(Mandatory = $true)]
        [DateTime]$Value
    )

    return $Value.ToString('yyyy-MM-dd HH:mm:ss zzz')
}

function Get-RequiredEnv {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    $value = [Environment]::GetEnvironmentVariable($Name)
    if ([string]::IsNullOrWhiteSpace($value)) {
        return $null
    }

    return $value
}

function Assert-LlmEnvironment {
    $required = @('LLM_BASE_URL', 'LLM_API_KEY', 'LLM_MODEL')
    $missing = @()

    foreach ($name in $required) {
        if ($null -eq (Get-RequiredEnv -Name $name)) {
            $missing += $name
        }
    }

    if ($missing.Count -gt 0) {
        throw ("Missing required LLM environment variable(s): {0}. Set LLM_BASE_URL, LLM_API_KEY, and LLM_MODEL in the current environment before running real LLM smoke tests." -f ($missing -join ', '))
    }
}

function Redact-Endpoint {
    param(
        [Parameter(Mandatory = $true)]
        [string]$BaseUrl
    )

    try {
        $uri = [Uri]$BaseUrl
        $builder = [System.UriBuilder]::new($uri)
        if (-not [string]::IsNullOrEmpty($builder.UserName)) {
            $builder.UserName = '***'
        }
        if (-not [string]::IsNullOrEmpty($builder.Password)) {
            $builder.Password = '***'
        }
        $builder.Query = $null
        return $builder.Uri.GetLeftPart([UriPartial]::Path)
    } catch {
        return '<redacted endpoint>'
    }
}

function Resolve-Suites {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RequestedSuites
    )

    $names = @($RequestedSuites -split ',' |
        ForEach-Object { $_.Trim().ToLowerInvariant() } |
        Where-Object { $_.Length -gt 0 })

    if ($names.Count -eq 0) {
        throw 'No suite names were provided.'
    }

    $resolved = New-Object System.Collections.Generic.List[string]
    foreach ($name in $names) {
        if ($name -eq 'all') {
            foreach ($suiteName in $SuiteDefinitions.Keys) {
                if (-not $resolved.Contains($suiteName)) {
                    $resolved.Add($suiteName)
                }
            }
            continue
        }

        if (-not $SuiteDefinitions.Contains($name)) {
            throw ("Unknown suite '{0}'. Use -List to print available suites." -f $name)
        }

        if (-not $resolved.Contains($name)) {
            $resolved.Add($name)
        }
    }

    return $resolved.ToArray()
}

function Show-Suites {
    Write-Host 'Available real LLM smoke suites:'
    foreach ($name in $SuiteDefinitions.Keys) {
        Write-Host ("  {0,-11} {1}" -f $name, $SuiteDefinitions[$name].Description)
        Write-Host ("              filter: {0}" -f $SuiteDefinitions[$name].Filter)
    }
    Write-Host ("  {0,-9} {1}" -f 'all', 'Run every suite above in order')
}

function Start-SmokeStep {
    param(
        [Parameter(Mandatory = $true)]
        [string]$SuiteName,

        [Parameter(Mandatory = $true)]
        [int]$TotalSteps
    )

    $script:StepNumber += 1
    Write-Host ''
    Write-Host ("[{0}/{1}] suite {2}" -f $script:StepNumber, $TotalSteps, $SuiteName) -ForegroundColor Cyan
}

function Invoke-SmokeSuite {
    param(
        [Parameter(Mandatory = $true)]
        [string]$SuiteName,

        [Parameter(Mandatory = $true)]
        [string]$RepoRoot,

        [Parameter(Mandatory = $true)]
        [int]$TotalSteps
    )

    Start-SmokeStep -SuiteName $SuiteName -TotalSteps $TotalSteps

    $filter = $SuiteDefinitions[$SuiteName].Filter
    $command = @('cargo', 'test', '-p', 'harness-real-llm', $filter, '--', '--ignored', '--nocapture')
    $suiteStartedAt = Get-Date

    if ($DryRun) {
        Write-Host ("DRY RUN: cd {0}" -f $RepoRoot)
        Write-Host ("DRY RUN: {0}" -f (Format-Command -Command $command))
        return @{
            Suite = $SuiteName
            Filter = $filter
            Status = 'DRY-RUN'
            ExitCode = 0
            StartedAt = $suiteStartedAt
            EndedAt = Get-Date
        }
    }

    Push-Location -LiteralPath $RepoRoot
    try {
        $executable = $command[0]
        $arguments = $command[1..($command.Count - 1)]

        Write-Host ("RUN: {0}" -f (Format-Command -Command $command))
        # cargo writes build progress to stderr; under ErrorActionPreference=Stop
        # the merged 2>&1 would turn those lines into terminating errors. Lower
        # to Continue while we stream native output through Out-Host (which keeps
        # it off the function return pipeline so $result stays a clean hashtable).
        $prevErrorPreference = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try {
            & $executable @arguments 2>&1 | Out-Host
        } finally {
            $ErrorActionPreference = $prevErrorPreference
        }
        $exitCode = $LASTEXITCODE
    } finally {
        Pop-Location
    }

    $suiteEndedAt = Get-Date
    if ($exitCode -eq 0) {
        if ($SuiteName -eq 'm5' -or $SuiteName -eq 'eval' -or $SuiteName -eq 'endurance') {
            # M5/eval 是真实模型探索性探针：exit 0 = 探针执行通过，不等于完整验收通过
            Write-Host ("OK: suite {0} probe execution passed (acceptance may still be Partial Evidence / Inconclusive)." -f $SuiteName) -ForegroundColor Green
            $status = 'PROBE_PASS'
        } else {
            Write-Host ("OK: suite {0} passed." -f $SuiteName) -ForegroundColor Green
            $status = 'PASS'
        }
    } else {
        Write-Host ("FAIL: suite {0} failed with exit code {1}." -f $SuiteName, $exitCode) -ForegroundColor Red
        $status = 'FAIL'
    }

    return @{
        Suite = $SuiteName
        Filter = $filter
        Status = $status
        ExitCode = $exitCode
        StartedAt = $suiteStartedAt
        EndedAt = $suiteEndedAt
    }
}

try {
    if ($List) {
        Show-Suites
        exit 0
    }

    if (-not $DryRun) {
        Assert-LlmEnvironment
    }

    $repoRoot = Find-RepoRoot
    $suites = @(Resolve-Suites -RequestedSuites $Suite)
    $startedAt = Get-Date
    $endpoint = if ($DryRun) { '<not required for dry run>' } else { Redact-Endpoint -BaseUrl (Get-RequiredEnv -Name 'LLM_BASE_URL') }
    $model = if ($DryRun) { '<not required for dry run>' } else { Get-RequiredEnv -Name 'LLM_MODEL' }
    $toolMode = Get-RequiredEnv -Name 'LLM_TOOL_MODE'
    if ($null -eq $toolMode) {
        $toolMode = '<default>'
    }

    $evalEnabled = Get-RequiredEnv -Name 'STORYFORGE_EVAL_REAL_LLM'
    if ($null -eq $evalEnabled) {
        $evalEnabled = '<unset>'
    }
    $evalMaxCalls = Get-RequiredEnv -Name 'STORYFORGE_EVAL_MAX_CALLS'
    if ($null -eq $evalMaxCalls) {
        $evalMaxCalls = '40'
    }
    $evalMaxTurns = Get-RequiredEnv -Name 'STORYFORGE_EVAL_MAX_TURNS'
    if ($null -eq $evalMaxTurns) {
        $evalMaxTurns = '24'
    }
    $evalTimeout = Get-RequiredEnv -Name 'STORYFORGE_EVAL_TIMEOUT_SECS'
    if ($null -eq $evalTimeout) {
        $evalTimeout = '180'
    }

    Write-Host ("StoryForge real LLM smoke root: {0}" -f $repoRoot)
    Write-Host ("Started: {0}" -f (Format-Timestamp -Value $startedAt))
    Write-Host ("Endpoint: {0}" -f $endpoint)
    Write-Host ("Model: {0}" -f $model)
    Write-Host ("Tool mode: {0}" -f $toolMode)
    Write-Host ("Suites: {0}" -f ($suites -join ', '))
    Write-Host ("Eval real switch: STORYFORGE_EVAL_REAL_LLM={0}" -f $evalEnabled)
    Write-Host ("Eval budget: max_calls={0} max_turns={1} timeout_secs={2}" -f $evalMaxCalls, $evalMaxTurns, $evalTimeout)
    if ($suites -contains 'eval') {
        $evalOn = $false
        if ($null -ne (Get-RequiredEnv -Name 'STORYFORGE_EVAL_REAL_LLM')) {
            $v = (Get-RequiredEnv -Name 'STORYFORGE_EVAL_REAL_LLM').Trim().ToLowerInvariant()
            if ($v -in @('1', 'true', 'yes', 'on')) {
                $evalOn = $true
            }
        }
        if (-not $evalOn -and -not $DryRun) {
            throw 'Suite eval requires STORYFORGE_EVAL_REAL_LLM=1 (explicit paid-model authorization). Deterministic eval tests run via cargo test without this switch.'
        }
        $evalFixture = Get-RequiredEnv -Name 'STORYFORGE_EVAL_FIXTURE_CARD'
        if (-not $DryRun) {
            if ($null -eq $evalFixture) {
                throw 'Suite eval requires STORYFORGE_EVAL_FIXTURE_CARD pointing to an existing character-card PNG; the repository has no default fixture.'
            }
            if (-not (Test-Path -LiteralPath $evalFixture -PathType Leaf)) {
                throw ("Suite eval fixture does not exist: {0}" -f $evalFixture)
            }
        }
        $parsedTurns = 0
        $parsedCalls = 0
        $parsedTimeout = 0
        if (-not [int]::TryParse($evalMaxTurns, [ref]$parsedTurns) -or $parsedTurns -le 15) {
            throw 'Suite eval requires STORYFORGE_EVAL_MAX_TURNS >= 16 so accepted turns strictly cross H_anchor+E=15.'
        }
        if (-not [int]::TryParse($evalMaxCalls, [ref]$parsedCalls) -or $parsedCalls -lt $parsedTurns) {
            throw 'Suite eval requires STORYFORGE_EVAL_MAX_CALLS >= STORYFORGE_EVAL_MAX_TURNS (absolute minimum; 96 calls is the recommended 16-turn starting budget).'
        }
        if (-not [int]::TryParse($evalTimeout, [ref]$parsedTimeout) -or $parsedTimeout -lt 1) {
            throw 'Suite eval requires STORYFORGE_EVAL_TIMEOUT_SECS >= 1.'
        }
        if ($parsedCalls -lt ($parsedTurns * 3)) {
            Write-Warning 'Eval max_calls is below 3x max_turns; the production pipeline may fail closed before completing all Accept rounds.'
        }
    }

    if ($suites -contains 'endurance') {
        $endOn = $false
        if ($null -ne (Get-RequiredEnv -Name 'STORYFORGE_EVAL_REAL_LLM')) {
            $v = (Get-RequiredEnv -Name 'STORYFORGE_EVAL_REAL_LLM').Trim().ToLowerInvariant()
            if ($v -in @('1', 'true', 'yes', 'on')) {
                $endOn = $true
            }
        }
        if (-not $endOn -and -not $DryRun) {
            throw 'Suite endurance requires STORYFORGE_EVAL_REAL_LLM=1 (explicit paid-model authorization).'
        }
        # endurance builds the character in code — no fixture PNG required.
        $endStage = Get-RequiredEnv -Name 'STORYFORGE_EVAL_ENDURANCE_STAGE'
        if ($null -eq $endStage) {
            $endStage = 'canary'
        }
        Write-Host ("Endurance stage: STORYFORGE_EVAL_ENDURANCE_STAGE={0}" -f $endStage)
        if ($endStage -eq 'full') {
            $endMaxCalls = $evalMaxCalls
            $endParsedCalls = 0
            if ( [int]::TryParse($endMaxCalls, [ref]$endParsedCalls) -and $endParsedCalls -lt 700) {
                Write-Warning 'Endurance full stage requires STORYFORGE_EVAL_MAX_CALLS >= 700 for 100 accepted turns.'
            }
        }

        # Durable evidence root preflight (no model calls). Prefer EVIDENCE_ROOT;
        # legacy EVIDENCE_DIR is accepted. Temp roots require explicit allow flag.
        $evidenceRoot = Get-RequiredEnv -Name 'STORYFORGE_EVAL_EVIDENCE_ROOT'
        $evidenceDir = Get-RequiredEnv -Name 'STORYFORGE_EVAL_EVIDENCE_DIR'
        $allowEphemeral = Get-RequiredEnv -Name 'STORYFORGE_EVAL_ALLOW_EPHEMERAL_EVIDENCE'
        $allowEphemeralOn = $false
        if ($null -ne $allowEphemeral) {
            $ae = $allowEphemeral.Trim().ToLowerInvariant()
            if ($ae -in @('1', 'true', 'yes', 'on')) { $allowEphemeralOn = $true }
        }
        if ($null -eq $evidenceRoot -and $null -eq $evidenceDir) {
            if (-not $allowEphemeralOn) {
                throw 'Suite endurance requires STORYFORGE_EVAL_EVIDENCE_ROOT (preferred) or STORYFORGE_EVAL_EVIDENCE_DIR. Temp roots need STORYFORGE_EVAL_ALLOW_EPHEMERAL_EVIDENCE=1.'
            }
            Write-Warning 'No durable evidence root set; ephemeral evidence is explicitly allowed for this process only.'
        } else {
            $candidate = if ($null -ne $evidenceRoot) { $evidenceRoot } else { $evidenceDir }
            if ($candidate -match '(^|[\\/])\.\.([\\/]|$)') {
                throw 'Evidence root path must not contain parent-directory traversal segments.'
            }
            $full = [System.IO.Path]::GetFullPath($candidate)
            $repoFull = [System.IO.Path]::GetFullPath($repoRoot).TrimEnd('\', '/')
            if ($full.Equals($repoFull, [System.StringComparison]::OrdinalIgnoreCase) -or
                $full.StartsWith(($repoFull + [System.IO.Path]::DirectorySeparatorChar), [System.StringComparison]::OrdinalIgnoreCase)) {
                throw 'Evidence root must not live inside the repository.'
            }
            $tempFull = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd('\', '/')
            $isTemp = $full.StartsWith(($tempFull + [System.IO.Path]::DirectorySeparatorChar), [System.StringComparison]::OrdinalIgnoreCase) -or
                $full -match '(?i)([\\/]tmp[\\/]|[\\/]temp[\\/]|:\\tmp\\|:\\temp\\)'
            if ($isTemp -and -not $allowEphemeralOn) {
                throw 'Ephemeral/temp evidence roots are rejected unless STORYFORGE_EVAL_ALLOW_EPHEMERAL_EVIDENCE=1.'
            }
            Write-Host ("Endurance evidence root preflight OK (basename={0})" -f ([System.IO.Path]::GetFileName($full)))
        }
    }
    if ($DryRun) {
        Write-Host 'Dry run enabled; commands will be printed but not executed.'
    }
    if ($ContinueOnFailure) {
        Write-Host 'ContinueOnFailure enabled; remaining suites will run after failures.'
    }

    $results = New-Object System.Collections.Generic.List[hashtable]
    foreach ($suiteName in $suites) {
        $result = Invoke-SmokeSuite -SuiteName $suiteName -RepoRoot $repoRoot -TotalSteps $suites.Count
        $results.Add($result)

        if ($result.ExitCode -ne 0 -and -not $ContinueOnFailure) {
            break
        }
    }

    $endedAt = Get-Date
    Write-Host ''
    Write-Host ("Ended: {0}" -f (Format-Timestamp -Value $endedAt))
    Write-Host 'Suite results:'
    foreach ($result in $results) {
        Write-Host ("  {0,-11} {1,-7} filter={2} exit={3}" -f $result.Suite, $result.Status, $result.Filter, $result.ExitCode)
    }

    $failures = @($results | Where-Object { $_.ExitCode -ne 0 })
    if ($failures.Count -gt 0) {
        throw ("Real LLM smoke failed: {0}" -f (($failures | ForEach-Object { $_.Suite }) -join ', '))
    }

    Write-Host ''
    $ranM5 = @($results | Where-Object { $_.Suite -eq 'm5' }).Count -gt 0
    $m5ProbeOk = @($results | Where-Object { $_.Suite -eq 'm5' -and $_.ExitCode -eq 0 }).Count -gt 0
    $ranEval = @($results | Where-Object { $_.Suite -eq 'eval' }).Count -gt 0
    $evalProbeOk = @($results | Where-Object { $_.Suite -eq 'eval' -and $_.ExitCode -eq 0 }).Count -gt 0
    if ($DryRun) {
        Write-Host 'Real LLM smoke dry run completed.' -ForegroundColor Green
    } elseif (($ranM5 -and $m5ProbeOk) -or ($ranEval -and $evalProbeOk)) {
        Write-Host 'PROBE EXECUTION PASS' -ForegroundColor Green
        if ($ranM5 -and $m5ProbeOk) {
            Write-Host 'M5 ACCEPTANCE: INCONCLUSIVE / Partial Evidence' -ForegroundColor Yellow
        }
        if ($ranEval -and $evalProbeOk) {
            Write-Host 'EVAL M5: PIPELINE WRITE + SYNTHETIC CHRONICLE FIXTURE + ACCEPT PROBE PASS' -ForegroundColor Yellow
            Write-Host 'Production Summarizer/postprocess evidence remains incomplete.' -ForegroundColor Yellow
        }
        Write-Host 'Do not treat this as full parameter calibration or complete M5 acceptance.' -ForegroundColor Yellow
    } else {
        Write-Host 'Real LLM smoke passed.' -ForegroundColor Green
    }
    exit 0
} catch {
    Write-Host ''
    # Write-Error 在 ErrorActionPreference=Stop 下会再次抛出并丢失原始异常上下文;
    # 直接写 host 流 + 显式退出码,保证失败信息可见且退出码确定。
    Write-Host ("ERROR: {0}" -f $_.Exception.Message) -ForegroundColor Red
    exit 1
}
