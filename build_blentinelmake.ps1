# Quick build script for blentinelmake
Write-Host "Building blentinelmake..." -ForegroundColor Green

cargo build -p blentinelmake --release

if ($LASTEXITCODE -eq 0) {
    Write-Host "`nblentinelmake built successfully!" -ForegroundColor Green
    Write-Host "Binary location: target\release\blentinelmake.exe" -ForegroundColor Cyan
    Write-Host "`nTo use it:" -ForegroundColor Yellow
    Write-Host "  .\target\release\blentinelmake.exe --help" -ForegroundColor White
    Write-Host "  .\target\release\blentinelmake.exe probe build --release" -ForegroundColor White
    Write-Host "  .\target\release\blentinelmake.exe hub publish" -ForegroundColor White
} else {
    Write-Host "Build failed!" -ForegroundColor Red
    exit 1
}
