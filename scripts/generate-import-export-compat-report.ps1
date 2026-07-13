# Generates a machine-readable ST import/export compatibility report via unit tests.
# Writes both JSON inventory + Markdown summary to artifacts/import-export-compat/.
# Scoped gate helper for codex/import-fixture-corpus. No real LLM / GUI.
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

# Fixed property seeds (PROPERTY_SEEDS in crates/infra-import/src/compat.rs).
$PropertySeeds = @(
  "0xC0A75EED"
  "0x5EED1234"
  "0x5EED5678"
  "0x5EED9ABC"
  "0x5EEDDEF0"
)

$Inventory = [ordered]@{
  format_version = 1
  matrix_version = 1
  generated_by = "scripts/generate-import-export-compat-report.ps1"
  generated_at = (Get-Date -Format "yyyy-MM-ddTHH:mm:ssZ")
  fixtures = @(
    "crates/infra-import/fixtures/st_v2_minimal.json"
    "crates/infra-import/fixtures/st_v3_matrix.json"
    "crates/infra-import/fixtures/st_v3_matrix.bom.json"
    "crates/infra-import/fixtures/st_v3_large_worldbook.json"
    "crates/infra-import/fixtures/st_v3_reasoning_regex.json"
    "crates/infra-import/fixtures/st_v3_mvu_tavernhelper.json"
  )
  property_seeds = $PropertySeeds
  generators = @(
    "generate_edge_card_json"
    "generate_large_worldbook_card"
    "generate_reasoning_regex_card"
    "generate_mvu_tavernhelper_card"
  )
  comparison_legs = @(
    "source -> first import (compare_source_to_first_import)"
    "first import -> export -> reimport (compare_character_roundtrip)"
  )
  areas = @(
    "ST V2/V3 card JSON"
    "PNG tEXt chara round-trip"
    "character_book routes/metadata/aliases (constant/selective/both/disabled)"
    "large worldbook (>=100 entries, all routes, key/keysecondary aliases)"
    "reasoning regex placement (code 6) with minDepth/maxDepth"
    "MVU/stat_data schema + tavern_helper + multi-definition payloads"
    "regex_scripts scoped metadata"
    "unknown data-level fields via raw_card_json"
    "UTF-8 BOM JSON"
    "malformed/truncated/oversized/bad-CRC rejection (fail closed)"
    "size-bomb guard (MAX_IMPORT_SIZE 100 MiB, MAX_CHUNK_SIZE 64 MiB)"
    "no-partial-store invariant (parse is all-or-nothing)"
    "multi-seed bounded property suite (5 fixed seeds)"
    "privacy-safe real-card evidence (counts + SHA-256 fingerprint)"
  )
  intentional_normalizations = @(
    "Character.id regenerated on each import"
    "world book position normalizes string labels to numeric codes on export (one-way)"
    "disabled world book entries (`disable: true`) filtered on import"
    "legacy `key`/`keysecondary` normalized to canonical `keys`/`secondary_keys` on first import"
    "Campaign/card/instance IDs rewritten on bundle import"
    "RegexScript is import-only in this crate (no export conversion)"
  )
  product_gaps = @(
    "Campaign multi-character ST export intentionally emits one PNG per instance + shared lorebook (not one flattened card)"
    "Turn/Attempt runtime records are not part of Campaign JSON Bundle v2"
    "MVU JS analysis still requires optional LLM path; pure schema extraction is deterministic"
    "UI-only import dialogs and real user-card matrix still need manual validation"
    "Real complex card fixture (test-card.png) is ignored; run scripts/run-real-card-smoke.ps1 locally"
  )
} | ConvertTo-Json -Depth 6

$InventoryPath = Join-Path $OutDir "compat-inventory-$Stamp.json"
Set-Content -Path $InventoryPath -Value $Inventory -Encoding utf8

# Markdown summary mirroring the JSON inventory.
$MdLines = @()
$MdLines += "# Import/Export Compatibility Inventory"
$MdLines += ""
$MdLines += "- generated_by: ``scripts/generate-import-export-compat-report.ps1``"
$MdLines += "- format_version: 1"
$MdLines += "- matrix_version: 1"
$MdLines += "- generated_at: $(Get-Date -Format 'yyyy-MM-ddTHH:mm:ssZ')"
$MdLines += ""
$MdLines += "## Sanitized fixtures"
$MdLines += ""
$MdLines += "| fixture |"
$MdLines += "|---------|"
foreach ($f in $Inventory.fixtures) { $MdLines += "| $f |" }
$MdLines += ""
$MdLines += "## Property seeds"
$MdLines += ""
foreach ($s in $PropertySeeds) { $MdLines += "- ``$s``" }
$MdLines += ""
$MdLines += "## Intentional normalizations"
$MdLines += ""
foreach ($n in $Inventory.intentional_normalizations) { $MdLines += "- $n" }
$MdLines += ""
$MdLines += "## Product gaps / boundaries"
$MdLines += ""
foreach ($g in $Inventory.product_gaps) { $MdLines += "- $g" }
$MdLines += ""
$Md = $MdLines -join "`n"

$MdPath = Join-Path $OutDir "compat-inventory-$Stamp.md"
Set-Content -Path $MdPath -Value $Md -Encoding utf8

Write-Host "Wrote $InventoryPath"
Write-Host "Wrote $MdPath"
Write-Host "Log: $Log"
exit 0
