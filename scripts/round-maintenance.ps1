param(
  [Parameter(Mandatory = $true)][int]$Round,
  [int]$MinMinutes = 20
)

$ErrorActionPreference = "Continue"

$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$roundTag = ("{0:D2}" -f $Round)
$startAt = Get-Date

$maintenanceRoot = Join-Path $projectRoot "docs\maintenance"
$roundDir = Join-Path $maintenanceRoot ("round-" + $roundTag)
New-Item -ItemType Directory -Path $roundDir -Force | Out-Null

function Invoke-Step {
  param(
    [string]$Name,
    [string]$Command,
    [string]$OutputFile
  )

  $stepStart = Get-Date
  Push-Location $projectRoot
  try {
    $output = cmd /c $Command 2>&1
    $code = $LASTEXITCODE
  } catch {
    $output = @($_.Exception.Message)
    $code = 1
  } finally {
    Pop-Location
  }

  $elapsedMs = [int]((Get-Date) - $stepStart).TotalMilliseconds
  $header = @(
    "# step: $Name",
    "# command: $Command",
    "# exit_code: $code",
    "# elapsed_ms: $elapsedMs",
    ""
  )
  $allLines = $header + $output
  [System.IO.File]::WriteAllLines($OutputFile, $allLines, [System.Text.UTF8Encoding]::new($false))

  return @{
    name = $Name
    code = $code
    elapsed_ms = $elapsedMs
    output_file = $OutputFile
  }
}

$steps = @()

$steps += Invoke-Step -Name "mojibake_autofix" `
  -Command 'node scripts/apply-known-mojibake-fixes.js' `
  -OutputFile (Join-Path $roundDir "01-mojibake-autofix.log")

$steps += Invoke-Step -Name "encoding_check" `
  -Command 'npm run check:encoding' `
  -OutputFile (Join-Path $roundDir "02-encoding.log")

$steps += Invoke-Step -Name "i18n_check" `
  -Command 'npm run check:i18n' `
  -OutputFile (Join-Path $roundDir "03-i18n.log")

$steps += Invoke-Step -Name "comment_provenance_scan" `
  -Command 'rg -n "reference|copied|inspired by|forked from|based on" src src-tauri/src' `
  -OutputFile (Join-Path $roundDir "04-comment-provenance-scan.log")

$steps += Invoke-Step -Name "rust_check" `
  -Command 'cd /d src-tauri && cargo check --all-targets' `
  -OutputFile (Join-Path $roundDir "05-cargo-check.log")

$steps += Invoke-Step -Name "rust_clippy_dead_code" `
  -Command 'cd /d src-tauri && cargo clippy --all-targets -- -W dead_code -W unused' `
  -OutputFile (Join-Path $roundDir "06-cargo-clippy.log")

$steps += Invoke-Step -Name "garbled_scan" `
  -Command 'rg -n "\?\?\?\?" src src-tauri/src' `
  -OutputFile (Join-Path $roundDir "07-garbled-scan.log")

$bugPath = Join-Path $projectRoot "bug.md"
if (-not (Test-Path $bugPath)) {
  $bugInit = @(
    "# Bug Audit",
    "",
    "Audit findings only. Do not fix directly in this file.",
    ""
  )
  [System.IO.File]::WriteAllLines($bugPath, $bugInit, [System.Text.UTF8Encoding]::new($false))
}

$clippyLog = Join-Path $roundDir "06-cargo-clippy.log"
$clippyWarnings = @()
if (Test-Path $clippyLog) {
  $clippyWarnings = Select-String -Path $clippyLog -Pattern "^warning:" | ForEach-Object { $_.Line.Trim() }
}

$i18nLog = Join-Path $roundDir "03-i18n.log"
$i18nSummary = "i18n check unavailable"
if (Test-Path $i18nLog) {
  $i18nSummary = ((Get-Content $i18nLog | Select-Object -Last 1) -join "") -replace "\s+$", ""
}

$issueLines = @()
$issueLines += ($clippyWarnings | Select-Object -First 30 | ForEach-Object { "- " + $_ })
if ($issueLines.Count -eq 0) {
  $issueLines += "- no clippy warning captured in this round"
}

$roundBug = @(
  "",
  "## Round $roundTag - $(Get-Date -Format "yyyy-MM-dd HH:mm:ss")",
  "",
  "### Audit Summary (no fixes in this section)",
  "- clippy warning count: $([string]$clippyWarnings.Count)",
  "- i18n summary: $i18nSummary",
  "- garbled scan log: docs/maintenance/round-$roundTag/07-garbled-scan.log",
  "",
  "### Potential Issues"
)
Add-Content -Path $bugPath -Value (($roundBug + $issueLines) -join [Environment]::NewLine) -Encoding UTF8

$techDocPath = Join-Path $projectRoot "docs\TECHNICAL_RELEASE.md"
if (-not (Test-Path $techDocPath)) {
  $techInit = @(
    "# AG Proxy Manager Technical Documentation",
    "",
    "## Maintenance Round Log",
    ""
  )
  [System.IO.File]::WriteAllLines($techDocPath, $techInit, [System.Text.UTF8Encoding]::new($false))
}

$techRound = @(
  "### Round $roundTag - $(Get-Date -Format "yyyy-MM-dd HH:mm:ss")",
  "- Steps: encoding, i18n, dead code audit, garbled scan, comment provenance scan.",
  "- Artifact path: docs/maintenance/round-$roundTag/",
  "- Key logs:",
  "  - 02-encoding.log",
  "  - 03-i18n.log",
  "  - 06-cargo-clippy.log",
  "  - 07-garbled-scan.log",
  ""
)
Add-Content -Path $techDocPath -Value ($techRound -join [Environment]::NewLine) -Encoding UTF8

$summary = [ordered]@{
  round = $Round
  started_at = $startAt.ToString("s")
  finished_at = (Get-Date).ToString("s")
  duration_seconds = [int]((Get-Date) - $startAt).TotalSeconds
  steps = $steps
}

$summaryJson = Join-Path $roundDir "summary.json"
$summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $summaryJson -Encoding utf8

$elapsedSeconds = [int]((Get-Date) - $startAt).TotalSeconds
$minimumSeconds = $MinMinutes * 60
if ($elapsedSeconds -lt $minimumSeconds) {
  Start-Sleep -Seconds ($minimumSeconds - $elapsedSeconds)
}

Write-Output ("ROUND_DONE " + $roundTag)
