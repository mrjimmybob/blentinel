[CmdletBinding(PositionalBinding = $false)]
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
    Write-Host "  -Target <target>      Cross-compile target for probe only"
    Write-Host "  -Help                 Show this help"
}

if ($Help) { Show-Help; exit 0 }

Write-Host "=== Building HUB ===" -ForegroundColor Green

if ($PSBoundParameters.ContainsKey("Release")) {
    .\build_hub.ps1 -Release
}
else {
    .\build_hub.ps1
}

if ($LASTEXITCODE -ne 0) { exit 1 }

Write-Host "`n=== Building PROBE ===" -ForegroundColor Green

if ($PSBoundParameters.ContainsKey("Target")) {
    if ($PSBoundParameters.ContainsKey("Release")) {
        .\build_probe.ps1 -Release -Target $Target
    }
    else {
        .\build_probe.ps1 -Target $Target
    }
}
else {
    if ($PSBoundParameters.ContainsKey("Release")) {
        .\build_probe.ps1 -Release
    }
    else {
        .\build_probe.ps1
    }
}

if ($LASTEXITCODE -ne 0) { exit 1 }

Write-Host "`nAll builds completed successfully." -ForegroundColor Cyan
