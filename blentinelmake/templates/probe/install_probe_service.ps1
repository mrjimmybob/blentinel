$serviceName = "BlentinelProbe"
$installDir  = "C:\Blentinel\probe"
$exeName     = "probe.exe"
$exePath     = Join-Path $installDir $exeName
$configPath  = Join-Path $installDir "blentinel_probe.toml"

Write-Host "Installing Blentinel Probe service..." -ForegroundColor Green

# Remove old service FIRST (avoids locked binaries)
$svc = Get-Service $serviceName -ErrorAction SilentlyContinue
if ($svc) {
    Write-Host "Removing existing service..."
    Stop-Service $serviceName -ErrorAction SilentlyContinue
    Start-Sleep 1
    sc.exe delete $serviceName | Out-Null
    Start-Sleep 1
}

# Install files
New-Item -ItemType Directory -Force -Path $installDir | Out-Null
Copy-Item ".\$exeName" $exePath -Force

# Preserve user config if already present
if (-not (Test-Path $configPath)) {
    Write-Host "Copying default configuration..."
    Copy-Item ".\blentinel_probe.toml" $configPath
}

# Create service
sc.exe create $serviceName binPath= "`"$exePath`"" start= auto | Out-Null
sc.exe description $serviceName "Blentinel network probe" | Out-Null
sc.exe failure $serviceName reset= 90000 actions= restart/300000/restart/300000/restart/300000 | Out-Null

# Attempt start
try {
    Start-Service $serviceName -ErrorAction Stop
    Start-Sleep 2
}
catch {
    Write-Host ""
    Write-Host "Service installed but FAILED to start." -ForegroundColor Yellow
    Write-Host "This usually means the probe config is missing or invalid."
    Write-Host ""
    Write-Host "Config file:"
    Write-Host "  $configPath"
    Write-Host ""
    Write-Host "Check logs:"
    Write-Host "  Event Viewer → Windows Logs → System"
    exit 0
}

# Verify running state
$svc = Get-Service $serviceName

if ($svc.Status -ne "Running") {
    Write-Host "Service installed but not running." -ForegroundColor Yellow
    Write-Host "Check logs:"
    Write-Host "  Event Viewer → Windows Logs → System"
    exit 0
}

Write-Host "Blentinel Probe installed and running." -ForegroundColor Green
