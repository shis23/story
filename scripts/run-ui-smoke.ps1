param(
  [int]$Port = 0
)

$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$frontendDir = Join-Path $repoRoot 'frontend'
$timestamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$artifactDir = Join-Path $repoRoot "artifacts\ui-smoke\$timestamp"
$serverLog = Join-Path $artifactDir 'vite-server.log'
$serverErr = Join-Path $artifactDir 'vite-server.err.log'

New-Item -ItemType Directory -Force $artifactDir | Out-Null

function Get-FreeTcpPort {
  $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
  try {
    $listener.Start()
    return $listener.LocalEndpoint.Port
  } finally {
    $listener.Stop()
  }
}

if ($Port -le 0) {
  $Port = Get-FreeTcpPort
}

$playwrightPackage = Join-Path $frontendDir 'node_modules\@playwright\test\package.json'
$playwrightBin = Join-Path $frontendDir 'node_modules\.bin\playwright.cmd'
if (-not (Test-Path $playwrightPackage) -or -not (Test-Path $playwrightBin)) {
  $message = @(
    'Playwright dependency is not installed locally; refusing to install from the network.',
    "Expected: $playwrightPackage",
    "Artifact directory: $artifactDir",
    'Install @playwright/test in frontend when dependencies are available, then re-run this script.'
  ) -join [Environment]::NewLine
  $message | Tee-Object -FilePath (Join-Path $artifactDir 'SKIPPED.txt')
  exit 2
}

$env:UI_SMOKE_ARTIFACT_DIR = $artifactDir
$env:PLAYWRIGHT_BASE_URL = "http://127.0.0.1:$Port"
$server = $null

try {
  $server = Start-Process -FilePath 'npm.cmd' `
    -ArgumentList @('run', 'dev', '--', '--host', '127.0.0.1', '--port', "$Port", '--strictPort') `
    -WorkingDirectory $frontendDir `
    -RedirectStandardOutput $serverLog `
    -RedirectStandardError $serverErr `
    -WindowStyle Hidden `
    -PassThru

  $deadline = (Get-Date).AddSeconds(30)
  do {
    if ($server.HasExited) {
      throw "Vite dev server exited early with code $($server.ExitCode). See $serverLog and $serverErr"
    }
    try {
      $response = Invoke-WebRequest -Uri $env:PLAYWRIGHT_BASE_URL -UseBasicParsing -TimeoutSec 2
      if ($response.StatusCode -ge 200 -and $response.StatusCode -lt 500) {
        break
      }
    } catch {
      Start-Sleep -Milliseconds 500
    }
  } while ((Get-Date) -lt $deadline)

  if ((Get-Date) -ge $deadline) {
    throw "Timed out waiting for Vite at $($env:PLAYWRIGHT_BASE_URL)"
  }

  & $playwrightBin test -c playwright.config.mjs
  $exitCode = $LASTEXITCODE
  if ($exitCode -ne 0) {
    throw "Playwright UI smoke failed with exit code $exitCode"
  }

  Write-Host "UI smoke artifacts: $artifactDir"
} finally {
  if ($server -and -not $server.HasExited) {
    Stop-Process -Id $server.Id -Force
    $server.WaitForExit()
  }
}
