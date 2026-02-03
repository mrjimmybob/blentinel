param(
    [switch]$Release,
    [string]$Target = "",
    [switch]$Clean,
    [switch]$Help,
    [switch]$Watch
)

function Show-Help {
    Write-Host "Probe Build Script" -ForegroundColor Cyan
    Write-Host "===================" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "Usage: .\build_probe.ps1 [options]"
    Write-Host ""
    Write-Host "Options:"
    Write-Host "  -Release              Build in release mode (optimized)"
    Write-Host "  -Target <target>      Specify target (e.g., x86_64-pc-windows-msvc)"
    Write-Host "  -Watch                Watch for changes and rebuild"
    Write-Host "  -Help                 Show this help message"
    Write-Host ""
    Write-Host "Examples:"
    Write-Host "  .\build_probe.ps1"
    Write-Host "  .\build_probe.ps1 -Release"
    Write-Host "  .\build_probe.ps1 -Target x86_64-unknown-linux-musl"
    Write-Host "  .\build_probe.ps1 -Watch"
    Write-Host ""
}

# Support --help (bash habit)
if ($args -contains "--help") {
    $Help = $true
}

# Catch accidental "--something" passed as Target
if ($Target -and $Target.StartsWith("-")) {
    Write-Host "Invalid target value: $Target" -ForegroundColor Red
    exit 1
}

if ($Help) {
    Show-Help
    exit 0
}

# Build argument list
$buildArgs = @("build", "-p", "probe")

if ($Release) {
    $buildArgs += "--release"
}

if ($Target -ne "") {
    $buildArgs += "--target"
    $buildArgs += $Target
}


if ($Clean) {
    Write-Host "Cleaning PROBE..." -ForegroundColor Yellow

    if ($Target -and $Target -ne "") {
        Write-Host "cargo clean -p probe --target $Target"
        cargo clean -p probe --target $Target
    }
    else {
        Write-Host "cargo clean -p probe"
        cargo clean -p probe
    }

    Write-Host "Probe clean complete." -ForegroundColor Green
    exit 0
}



if ($Watch) {
    Write-Host "Building probe (watch mode)..." -ForegroundColor Green
    Write-Host "Command: cargo watch -x `"$($buildArgs -join ' ')`"" -ForegroundColor Cyan
    cargo watch -x ($buildArgs -join " ")
    exit $LASTEXITCODE
}

Write-Host "Building probe..." -ForegroundColor Green

$startTime = Get-Date

# Decide which tool to use
$useZig = $false
if ($Target -and ($Target -like "*linux*")) {
    $useZig = $true
}

if ($useZig) {
    Write-Host "Using cargo-zigbuild for Linux target" -ForegroundColor Yellow
    $zigArgs = @("-p", "probe")
    if ($Release) { $zigArgs += "--release" }
    if ($Target) {
        $zigArgs += "--target"
        $zigArgs += $Target
    }

    Write-Host "Command: cargo zigbuild $($zigArgs -join ' ')" -ForegroundColor Cyan
    cargo zigbuild @zigArgs
}
else {
    Write-Host "Command: cargo $($buildArgs -join ' ')" -ForegroundColor Cyan
    cargo @buildArgs
}


$buildResult = $LASTEXITCODE
$endTime = Get-Date
$duration = $endTime - $startTime

if ($buildResult -eq 0) {
    Write-Host "`nBuild completed successfully!" -ForegroundColor Green
    Write-Host "Duration: $($duration.ToString('mm\:ss'))" -ForegroundColor Cyan

    $profileDir = if ($Release) { "release" } else { "debug" }

    if ($Target -and $Target -ne "") {
        $outputDir = "target\$Target\$profileDir"
    }
    else {
        $outputDir = "target\$profileDir"
    }
    if (Test-Path $outputDir) {
        Write-Host "`nOutput directory: $outputDir" -ForegroundColor Yellow
    }
}
else {
    Write-Host "`nBuild failed with exit code: $buildResult" -ForegroundColor Red
    exit $buildResult
}
