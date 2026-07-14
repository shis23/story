# Generates an auditable ST import/export compatibility report from a real
# CompatReport (rows + findings + summary), not a hard-coded inventory.
# Writes both JSON and Markdown under artifacts/import-export-compat/.
# Scoped gate helper for codex/import-fixture-corpus. No real LLM / GUI.
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

$OutDir = Join-Path $Root "artifacts/import-export-compat"
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$Stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$Log = Join-Path $OutDir "compat-$Stamp.log"

# Point the corpus-report emitter at our stamped artifacts dir.
$env:SF_COMPAT_REPORT_DIR = $OutDir
$env:SF_COMPAT_REPORT_STAMP = $Stamp

Write-Host "Running storyforge-infra-import compat suite + corpus report emitter..."
$prev = $ErrorActionPreference
$ErrorActionPreference = "Continue"
cargo test -p storyforge-infra-import --lib -- --nocapture 2>&1 | Tee-Object -FilePath $Log
$exit = $LASTEXITCODE
$ErrorActionPreference = $prev
if ($exit -ne 0) {
  Write-Error "compat suite failed; see $Log"
  exit $exit
}

$JsonPath = Join-Path $OutDir "compat-report-$Stamp.json"
$MdPath = Join-Path $OutDir "compat-report-$Stamp.md"
if (-not (Test-Path -LiteralPath $JsonPath)) {
  Write-Error "expected real CompatReport JSON missing: $JsonPath"
  exit 2
}
if (-not (Test-Path -LiteralPath $MdPath)) {
  Write-Error "expected real CompatReport Markdown missing: $MdPath"
  exit 2
}

$JsonText = Get-Content -LiteralPath $JsonPath -Raw -Encoding utf8
$Report = $JsonText | ConvertFrom-Json
if ($null -eq $Report.rows -or $Report.rows.Count -lt 1) {
  Write-Error "CompatReport JSON has empty rows; not auditable"
  exit 3
}
if ($null -eq $Report.findings -or $Report.findings.Count -lt 1) {
  Write-Error "CompatReport JSON has empty findings; not auditable"
  exit 3
}
if ($null -eq $Report.summary) {
  Write-Error "CompatReport JSON missing summary"
  exit 3
}

$MdText = Get-Content -LiteralPath $MdPath -Raw -Encoding utf8
foreach ($needle in @("# Compatibility Report", "## Matrix rows", "## Summary", "st_v3_large_worldbook", "st_v2_minimal")) {
  if ($MdText -notlike "*$needle*") {
    Write-Error "CompatReport Markdown missing expected content: $needle"
    exit 4
  }
}

# Stable "latest" copies for consumers that do not want the stamp.
Copy-Item -LiteralPath $JsonPath -Destination (Join-Path $OutDir "compat-report-latest.json") -Force
Copy-Item -LiteralPath $MdPath -Destination (Join-Path $OutDir "compat-report-latest.md") -Force

Write-Host "Wrote $JsonPath"
Write-Host "Wrote $MdPath"
Write-Host "Rows: $($Report.rows.Count); findings: $($Report.findings.Count); loss=$($Report.summary.loss) preserved=$($Report.summary.preserved)"
Write-Host "Log: $Log"
exit 0
