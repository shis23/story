# Generates a machine-readable ST import/export compatibility report via unit tests.
# Scoped gate helper for codex/import-export-hardening. No real LLM / GUI.
$ErrorActionPreference = "Continue"
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

$OutDir = Join-Path $Root "artifacts/import-export-compat"
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$Stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$Log = Join-Path $OutDir "compat-$Stamp.log"

Write-Host "Running storyforge-infra-import compat suite..."
$prev = $ErrorActionPreference
$ErrorActionPreference = "Continue"
cargo test -p storyforge-infra-import --lib -- --nocapture 2>&1 | Tee-Object -FilePath $Log
$exit = $LASTEXITCODE
$ErrorActionPreference = $prev
if ($exit -ne 0) {
  Write-Error "compat suite failed; see $Log"
  exit $exit
}

$Inventory = @{
  format_version = 1
  generated_by = "scripts/generate-import-export-compat-report.ps1"
  fixtures = @(
    "crates/infra-import/fixtures/st_v2_minimal.json"
    "crates/infra-import/fixtures/st_v3_matrix.json"
    "crates/infra-import/fixtures/st_v3_matrix.bom.json"
  )
  areas = @(
    "ST V2/V3 card JSON"
    "PNG tEXt chara round-trip"
    "character_book routes/metadata/aliases"
    "regex_scripts scoped metadata"
    "MVU/stat_data schema detection"
    "unknown data-level fields via raw_card_json"
    "UTF-8 BOM JSON"
    "malformed/truncated/oversized rejection"
    "bounded property suite seed 0xC0A75EED"
    "Campaign bundle atomic import and multi-character variables/provenance"
  )
  intentional_normalizations = @(
    "Character.id regenerated on each import"
    "world book position may normalize string labels to numeric codes on export"
    "disabled world book entries filtered on import"
    "Campaign/card/instance IDs rewritten on bundle import"
  )
  product_gaps = @(
    "Campaign multi-character ST export intentionally emits one PNG per instance + shared lorebook (not one flattened card)"
    "Turn/Attempt runtime records are not part of Campaign JSON Bundle v2"
    "MVU JS analysis still requires optional LLM path; pure schema extraction is deterministic"
    "UI-only import dialogs and real user-card matrix still need manual validation"
  )
} | ConvertTo-Json -Depth 6

$InventoryPath = Join-Path $OutDir "compat-inventory-$Stamp.json"
Set-Content -Path $InventoryPath -Value $Inventory -Encoding utf8
Write-Host "Wrote $InventoryPath"
Write-Host "Log: $Log"
exit 0
