[CmdletBinding(PositionalBinding = $false)]
param(
    [switch]$Release,
    [string]$Target,
    [switch]$Publish,
    [switch]$Clean,
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
    Write-Host "  -Target <target>      Cross-compile target for probe only"
    Write-Host "  -Publish              Run publish_hub.ps1 and publish_probe.ps1"
    Write-Host "  -Help                 Show this help"
}

if ($Help) { Show-Help; exit 0 }


if ($Clean) {
    Write-Host "=== Cleaning HUB ===" -ForegroundColor Yellow
    .\build_hub.ps1 -Clean
    if ($LASTEXITCODE -ne 0) { exit 1 }

    Write-Host "`n=== Cleaning PROBE ===" -ForegroundColor Yellow
    if ($Target) {
        .\build_probe.ps1 -Clean -Target $Target
    }
    else {
        .\build_probe.ps1 -Clean
    }
    if ($LASTEXITCODE -ne 0) { exit 1 }

    Write-Host "`nAll clean operations completed." -ForegroundColor Cyan
    exit 0
}


# =========================
# PUBLISH MODE
# =========================
if ($Publish) {
    Write-Host "=== Publishing HUB ===" -ForegroundColor Yellow
    .\publish_hub.ps1
    if ($LASTEXITCODE -ne 0) { exit 1 }

    Write-Host "`n=== Publishing PROBE ===" -ForegroundColor Yellow
    if ($PSBoundParameters.ContainsKey("Target")) {
        .\publish_probe.ps1 -Target $Target
    }
    else {
        .\publish_probe.ps1
    }
    if ($LASTEXITCODE -ne 0) { exit 1 }

    Write-Host "`nPublish completed successfully." -ForegroundColor Cyan
    exit 0
}


# =========================
# BUILD MODE
# =========================
Write-Host "=== Building HUB ===" -ForegroundColor Green
if ($Release) {
    .\build_hub.ps1 -Release
}
else {
    .\build_hub.ps1
}
if ($LASTEXITCODE -ne 0) { exit 1 }

Write-Host "`n=== Building PROBE ===" -ForegroundColor Green
if ($PSBoundParameters.ContainsKey("Target")) {
    if ($Release) {
        .\build_probe.ps1 -Release -Target $Target
    }
    else {
        .\build_probe.ps1 -Target $Target
    }
}
else {
    if ($Release) {
        .\build_probe.ps1 -Release
    }
    else {
        .\build_probe.ps1
    }
}
if ($LASTEXITCODE -ne 0) { exit 1 }

Write-Host "`nAll builds completed successfully." -ForegroundColor Cyan
