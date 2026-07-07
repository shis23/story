param(
  [string]$FixturePath = "test-card.png"
)

$ErrorActionPreference = "Stop"

$root = Resolve-Path (Join-Path $PSScriptRoot "..")
$fixture = if ([System.IO.Path]::IsPathRooted($FixturePath)) {
  $FixturePath
} else {
  Join-Path $root $FixturePath
}

if (-not (Test-Path -LiteralPath $fixture -PathType Leaf)) {
  throw "Complex card fixture not found: $fixture"
}

$resolvedFixture = (Resolve-Path -LiteralPath $fixture).Path

Write-Host "StoryForge real-card smoke"
Write-Host "Root: $root"
Write-Host "Fixture: $resolvedFixture"

$previousFixture = $env:SF_COMPLEX_CARD_FIXTURE
try {
  $env:SF_COMPLEX_CARD_FIXTURE = $resolvedFixture
  Push-Location $root
  try {
    & cargo test -p storyforge-infra-import test_real_complex_card_fixture_preserves_core_st_fields -- --ignored --nocapture
    if ($LASTEXITCODE -ne 0) {
      throw "infra-import real-card smoke failed with exit code $LASTEXITCODE"
    }

    & cargo test -p storyforge test_real_complex_card_fixture_can_create_campaign_and_roundtrip_bundle -- --ignored --nocapture
    if ($LASTEXITCODE -ne 0) {
      throw "tauri-app real-card campaign bundle smoke failed with exit code $LASTEXITCODE"
    }
  } finally {
    Pop-Location
  }
} finally {
  if ($null -eq $previousFixture) {
    Remove-Item Env:SF_COMPLEX_CARD_FIXTURE -ErrorAction SilentlyContinue
  } else {
    $env:SF_COMPLEX_CARD_FIXTURE = $previousFixture
  }
}

Write-Host "Real-card smoke passed."
