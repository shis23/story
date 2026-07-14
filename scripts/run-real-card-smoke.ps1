param(
  [string]$FixturePath = "test-card.png",
  [switch]$SkipTauriOnLoaderError
)

$ErrorActionPreference = "Stop"

function Invoke-CargoStep {
  param(
    [string]$Label,
    [string[]]$CargoArgs
  )

  Write-Host ""
  Write-Host "==> $Label"
  & cargo @CargoArgs
  if ($LASTEXITCODE -ne 0) {
    throw "$Label failed with exit code $LASTEXITCODE"
  }
}

function Test-TauriLibHarness {
  Write-Host ""
  Write-Host "==> tauri-app lib test harness preflight"
  $previousErrorAction = $ErrorActionPreference
  $ErrorActionPreference = "Continue"
  try {
    $output = & cargo test -p storyforge --lib -- --list 2>&1
    $exitCode = $LASTEXITCODE
  } finally {
    $ErrorActionPreference = $previousErrorAction
  }
  $output | ForEach-Object { Write-Host $_ }

  if ($exitCode -eq 0) {
    return $true
  }

  $joined = ($output | Out-String)
  if ($joined -match "STATUS_ENTRYPOINT_NOT_FOUND|0xc0000139") {
    $message = @"
tauri-app lib test harness failed before running Rust tests with STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139).
This is a Windows native loader/runtime issue, not a real-card assertion failure.
Run this script on a machine where `cargo test -p storyforge --lib -- --list` starts successfully,
or pass -SkipTauriOnLoaderError to run only the Tauri-free real-card import smoke on this machine.
"@
    if ($SkipTauriOnLoaderError) {
      Write-Warning $message
      return $false
    }
    throw $message
  }

  throw "tauri-app lib test harness preflight failed with exit code $exitCode"
}

$root = Resolve-Path (Join-Path $PSScriptRoot "..")
$fixture = if ([System.IO.Path]::IsPathRooted($FixturePath)) {
  $FixturePath
} else {
  Join-Path $root $FixturePath
}

if (-not (Test-Path -LiteralPath $fixture -PathType Leaf)) {
  throw "Complex card fixture not found: use -FixturePath or place test-card.png at the repository root."
}

$resolvedFixture = (Resolve-Path -LiteralPath $fixture).Path

Write-Host "StoryForge real-card smoke"
Write-Host "Fixture: configured (path withheld)"

$previousFixture = $env:SF_COMPLEX_CARD_FIXTURE
try {
  $env:SF_COMPLEX_CARD_FIXTURE = $resolvedFixture
  Push-Location $root
  try {
    Invoke-CargoStep `
      -Label "infra-import real-card field preservation smoke" `
      -CargoArgs @("test", "-p", "storyforge-infra-import", "test_real_complex_card_fixture_preserves_core_st_fields", "--", "--ignored", "--nocapture")

    $tauriHarnessAvailable = Test-TauriLibHarness
    if ($tauriHarnessAvailable) {
      Invoke-CargoStep `
        -Label "tauri-app real-card campaign bundle smoke" `
        -CargoArgs @("test", "-p", "storyforge", "test_real_complex_card_fixture_can_create_campaign_and_roundtrip_bundle", "--", "--ignored", "--nocapture")

      Invoke-CargoStep `
        -Label "tauri-app real-card offline MVU plumbing smoke" `
        -CargoArgs @("test", "-p", "storyforge", "test_real_complex_card_offline_mvu_plumbing_smoke", "--", "--ignored", "--nocapture")
    } else {
      Write-Warning "Skipped tauri-app real-card smoke steps because the lib test harness cannot start in this Windows environment."
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

Write-Host "Real-card smoke completed."
