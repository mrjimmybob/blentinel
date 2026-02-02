
param(
    [switch]$Release,
    [string]$Target,
    [switch]$Help
)

function Show-Help {
    Write-Host "Build All Script" -ForegroundColor Cyan
    Write-Host "================" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "Usage: .\build_all.ps1 [options]"
    Write-Host ""
    Write-Host "Options:"
    Write-Host "  -Release              Build in release mode"
    Write-Host "  -Target <target>      Cross-compile target (e.g. x86_64-unknown-linux-musl)"
    Write-Host "  -Help                 Show this help"
    Write-Host ""
}

if ($args -contains "--help") { $Help = $true }

if ($Help) {
    Show-Help
    exit 0
}

Write-Host "=== Building HUB ===" -ForegroundColor Green
$hubArgs = @()
if ($Release) { $hubArgs += "-Release" }
if ($Target) { $hubArgs += "-Target"; $hubArgs += $Target }

.\build_hub.ps1 @hubArgs
if ($LASTEXITCODE -ne 0) { exit 1 }

Write-Host "`n=== Building PROBE ===" -ForegroundColor Green
$probeArgs = @()
if ($Release) { $probeArgs += "-Release" }
if ($Target) { $probeArgs += "-Target"; $probeArgs += $Target }

.\build_probe.ps1 @probeArgs
if ($LASTEXITCODE -ne 0) { exit 1 }

Write-Host "`nAll builds completed successfully." -ForegroundColor Cyan
