# Install or upgrade the Blentinel hub on Windows.
#
# The hub has no Windows-service mode yet (DECISIONS.md D-34), so it runs as a
# scheduled task that starts at boot as SYSTEM and restarts if it stops.
#
#   program files  C:\Program Files\Blentinel\Hub     replaced on every upgrade
#   state + config C:\ProgramData\Blentinel\Hub       never overwritten
#
# Run from the unpacked package folder in an elevated PowerShell:
#   .\install_hub_service.ps1

$ErrorActionPreference = "Stop"
$taskName   = "Blentinel Hub"
$installDir = "C:\Program Files\Blentinel\Hub"
$stateDir   = "C:\ProgramData\Blentinel\Hub"
$config     = Join-Path $stateDir "blentinel_hub.toml"
$exe        = Join-Path $installDir "hub.exe"

Set-Location $PSScriptRoot
if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "Please run this script in an elevated (Administrator) PowerShell."
}

# Stop a running hub so its files can be replaced
if (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue) {
    Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
}
Get-Process hub -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe } | Stop-Process -Force
Start-Sleep -Seconds 1

# Program files: only what this package ships
New-Item -ItemType Directory -Force -Path $installDir | Out-Null
Copy-Item ".\hub.exe" $exe -Force
if (Test-Path (Join-Path $installDir "pkg")) { Remove-Item (Join-Path $installDir "pkg") -Recurse -Force }
Copy-Item ".\pkg" (Join-Path $installDir "pkg") -Recurse
if (Test-Path ".\favicon.ico") { Copy-Item ".\favicon.ico" $installDir -Force }

# State directory and configuration (first install only)
New-Item -ItemType Directory -Force -Path $stateDir | Out-Null
$firstInstall = -not (Test-Path $config)
if ($firstInstall) {
    $stateToml = $stateDir.Replace('\', '/')
    (Get-Content ".\blentinel_hub.toml") -replace '^state_dir = .*', "state_dir = `"$stateToml`"" |
        Set-Content -Path $config -Encoding utf8
}

# Scheduled task: at boot, as SYSTEM, restart on failure, no time limit
$action    = New-ScheduledTaskAction -Execute $exe -Argument "--config `"$config`"" -WorkingDirectory $stateDir
$trigger   = New-ScheduledTaskTrigger -AtStartup
$principal = New-ScheduledTaskPrincipal -UserId "SYSTEM" -LogonType ServiceAccount -RunLevel Highest
$settings  = New-ScheduledTaskSettingsSet -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) `
                 -ExecutionTimeLimit ([TimeSpan]::Zero) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $trigger -Principal $principal `
    -Settings $settings -Description "Blentinel central monitoring hub" -Force | Out-Null
Start-ScheduledTask -TaskName $taskName

Write-Host "Blentinel hub installed: $installDir (state and config: $stateDir)" -ForegroundColor Green
if ($firstInstall) {
    Write-Host ""
    Write-Host "First install:"
    Write-Host "  - It listens on the host/port in $config (127.0.0.1:3000 by default)."
    Write-Host "    Set host = `"0.0.0.0`" so probes on other machines can reach it, then run:"
    Write-Host "    Stop-ScheduledTask '$taskName'; Start-ScheduledTask '$taskName'"
    Write-Host "  - Open the web UI and create the first Owner with the setup code:"
    Write-Host "    Get-Content '$stateDir\setup.code'"
}
