param(
  [int]$Rounds = 30,
  [int]$MinMinutes = 20
)

$ErrorActionPreference = "Continue"

$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$maintenanceRoot = Join-Path $projectRoot "docs\\maintenance"
New-Item -ItemType Directory -Path $maintenanceRoot -Force | Out-Null

$loopLog = Join-Path $maintenanceRoot "loop.log"
$statusPath = Join-Path $maintenanceRoot "loop-status.json"

function Write-LoopLog {
  param([string]$Line)
  $msg = ("[{0}] {1}" -f (Get-Date -Format "yyyy-MM-dd HH:mm:ss"), $Line)
  Add-Content -Path $loopLog -Value $msg -Encoding UTF8
  Write-Output $msg
}

Write-LoopLog "maintenance loop started: rounds=$Rounds, min_minutes=$MinMinutes"

for ($i = 1; $i -le $Rounds; $i++) {
  $roundTag = ("{0:D2}" -f $i)
  $status = [ordered]@{
    running = $true
    current_round = $i
    total_rounds = $Rounds
    min_minutes = $MinMinutes
    started_at = (Get-Date).ToString("s")
  }
  $status | ConvertTo-Json -Depth 4 | Out-File -FilePath $statusPath -Encoding utf8

  Write-LoopLog "round $roundTag started"
  $roundCmd = "powershell -ExecutionPolicy Bypass -File scripts/round-maintenance.ps1 -Round $i -MinMinutes $MinMinutes"
  Push-Location $projectRoot
  try {
    cmd /c $roundCmd 2>&1 | Add-Content -Path (Join-Path $maintenanceRoot ("round-" + $roundTag + "-runner.log")) -Encoding UTF8
    $code = $LASTEXITCODE
  } finally {
    Pop-Location
  }
  Write-LoopLog "round $roundTag finished with exit_code=$code"
}

$doneStatus = [ordered]@{
  running = $false
  current_round = $Rounds
  total_rounds = $Rounds
  min_minutes = $MinMinutes
  finished_at = (Get-Date).ToString("s")
}
$doneStatus | ConvertTo-Json -Depth 4 | Out-File -FilePath $statusPath -Encoding utf8

Write-LoopLog "maintenance loop finished"
