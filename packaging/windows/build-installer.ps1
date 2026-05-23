$ErrorActionPreference = "Stop"

$projectRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$issPath = Join-Path $projectRoot "packaging\windows\claudie.iss"
$distPath = Join-Path $projectRoot "dist"

function Find-InnoCompiler {
    $fromPath = Get-Command "iscc.exe" -ErrorAction SilentlyContinue
    if ($fromPath) {
        return $fromPath.Source
    }

    $candidates = @(
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
    )

    foreach ($candidate in $candidates) {
        if ($candidate -and (Test-Path $candidate)) {
            return $candidate
        }
    }

    throw "Inno Setup 6 was not found. Install it from https://jrsoftware.org/isinfo.php, then run this script again."
}

Set-Location $projectRoot
New-Item -ItemType Directory -Force -Path $distPath | Out-Null

Write-Host "Building release binary..."
cargo build --release

$iscc = Find-InnoCompiler
Write-Host "Building installer with $iscc..."
& $iscc $issPath

$installer = Join-Path $distPath "claudie-setup.exe"
Write-Host ""
Write-Host "Installer ready:"
Write-Host $installer
