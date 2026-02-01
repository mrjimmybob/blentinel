param(
    [string]$Target = "x86_64-unknown-linux-gnu",
    [switch]$Help
)

function Show-Help {
    Write-Host "Publish Script" -ForegroundColor Cyan
    Write-Host "==============" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "Usage: .\publish.ps1 -Target <triple>"
    Write-Host ""
    Write-Host "Examples:"
    Write-Host "  .\publish.ps1 -Target x86_64-unknown-linux-gnu"
    Write-Host "  .\publish.ps1 -Target aarch64-unknown-linux-gnu"
    Write-Host "  .\publish.ps1 -Target x86_64-pc-windows-msvc"
}

if ($args -contains "--help") { $Help = $true }
if ($Help) { Show-Help; exit 0 }

$arch = $Target
$publishRoot = "publish\$arch"
$hubOut = "$publishRoot\hub"
$probeOut = "$publishRoot\probe"

New-Item -ItemType Directory -Force -Path $hubOut   | Out-Null
New-Item -ItemType Directory -Force -Path $probeOut | Out-Null

Write-Host "Publishing for $Target" -ForegroundColor Green

# Build hub
.\leptos-build.ps1 -Release -Target $Target
if ($LASTEXITCODE -ne 0) { exit 1 }

# Build probe
.\build_probe.ps1 -Release -Target $Target
if ($LASTEXITCODE -ne 0) { exit 1 }

$bprofile = "release"
$targetDir = "target\$Target\$bprofile"

# Executable names
$hubExe = if ($Target -like "*windows*") { "hub.exe" } else { "hub" }
$probeExe = if ($Target -like "*windows*") { "probe.exe" } else { "probe" }

Copy-Item "$targetDir\$hubExe"   "$hubOut\"   -Force
Copy-Item "$targetDir\$probeExe" "$probeOut\" -Force

# Strip binaries (Linux only)
if (-not ($Target -like "*windows*")) {
    if (Get-Command strip -ErrorAction SilentlyContinue) {
        Write-Host "Stripping binaries..." -ForegroundColor Yellow
        strip "$hubOut\$hubExe"
        strip "$probeOut\$probeExe"
    }
}

# Zip output
$zipPath = "publish\$arch.zip"
if (Test-Path $zipPath) { Remove-Item $zipPath -Force }

Compress-Archive -Path $publishRoot\* -DestinationPath $zipPath

Write-Host "`nPublish output:" -ForegroundColor Cyan
Write-Host "  $publishRoot"
Write-Host "  $zipPath"

Write-Host "`nPublish completed." -ForegroundColor Green
