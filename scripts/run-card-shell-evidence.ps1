# Run automated card-shell / worldinfo evidence suite (no GUI screenshots).
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

# False-green guard: a cargo test filter that matches nothing exits 0 with
# "0 tests"; fail instead of silently passing (Gate 8 review P2-D4).
function Invoke-CardShellCargoTest {
    param([string]$Name, [string[]]$Args)
    Write-Host "== $Name =="
    $output = & cargo @Args 2>&1
    $code = $LASTEXITCODE
    $output | Out-Host
    if ($code -ne 0) { exit $code }
    $text = $output -join "`n"
    $passed = 0
    $failed = 0
    foreach ($m in [regex]::Matches($text, 'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed')) {
        $passed += [int]$m.Groups[2].Value
        $failed += [int]$m.Groups[3].Value
    }
    if ($passed -eq 0 -and $failed -eq 0) {
        throw "cargo test ($Name) ran 0 tests; update or remove the stale filter."
    }
}

Invoke-CardShellCargoTest -Name 'domain card_shell' -Args @('test', '-p', 'storyforge-domain', 'card_shell', '--', '--nocapture')
Invoke-CardShellCargoTest -Name 'tauri card_shell_cache' -Args @('test', '-p', 'storyforge', '--lib', 'card_shell', '--', '--nocapture')
Invoke-CardShellCargoTest -Name 'campaign world_info' -Args @('test', '-p', 'storyforge', '--lib', 'campaign_store::tests::test_campaign_world_info', '--', '--nocapture')

Write-Host "== frontend unit tests =="
Push-Location frontend
node --test tests/card-shell-display.test.mjs tests/tavern-helper-scripts.test.mjs tests/shell-variable-outbox.test.mjs tests/campaign-tab-refresh.test.mjs
$code = $LASTEXITCODE
Pop-Location
if ($code -ne 0) { exit $code }

Write-Host "OK: card-shell evidence suite passed"
