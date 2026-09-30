$serviceName = "BlentinelProbe"
$installDir  = "C:\Blentinel\probe"
$exeName     = "probe.exe"
$exePath     = Join-Path $installDir $exeName
# No config file: the probe's checks are managed on the hub (D-49).

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
Write-Host "If it isn't enrolled yet, it waits for a code from the hub's Add probe:"
Write-Host "  & `"$exePath`" enroll <code>"
Write-Host "Then add its checks on the hub; the probe picks them up with its next report."
