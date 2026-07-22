# Run automated card-shell / worldinfo evidence suite (no GUI screenshots).
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

Write-Host "== domain card_shell =="
cargo test -p storyforge-domain card_shell -- --nocapture
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "== tauri card_shell_cache =="
cargo test -p storyforge --lib card_shell -- --nocapture
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "== campaign world_info =="
cargo test -p storyforge --lib campaign_store::tests::test_campaign_world_info -- --nocapture
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "== frontend unit tests =="
Push-Location frontend
node --test tests/card-shell-display.test.mjs tests/tavern-helper-scripts.test.mjs tests/shell-variable-outbox.test.mjs tests/campaign-tab-refresh.test.mjs
$code = $LASTEXITCODE
Pop-Location
if ($code -ne 0) { exit $code }

Write-Host "OK: card-shell evidence suite passed"
