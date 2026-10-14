param(
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = (Resolve-Path (Join-Path $ScriptDir "../../..")).Path
$RustDir = Join-Path $RepoRoot "rust"
$PackagesDir = Join-Path $RustDir "target/packages"
$BinDir = Join-Path $RustDir "target/release"

if (-not $SkipBuild) {
    Push-Location $RustDir
    try {
        cargo build --release -p au-app --features gui
        if ($LASTEXITCODE -ne 0) { throw "Building au-app failed." }
        cargo build --release -p au-plugin-host --bin au-nyquist-worker
        if ($LASTEXITCODE -ne 0) { throw "Building au-nyquist-worker failed." }
    }
    finally {
        Pop-Location
    }
}

$Metadata = (& cargo metadata --manifest-path (Join-Path $RustDir "Cargo.toml") --no-deps --format-version 1) | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw "cargo metadata failed." }
$AppPackage = $Metadata.packages | Where-Object { $_.name -eq "au-app" } | Select-Object -First 1
$Version = if ($AppPackage) { $AppPackage.version } else { "0.0.0" }

$MainExe = Join-Path $BinDir "audacity-rs.exe"
$WorkerExe = Join-Path $BinDir "au-nyquist-worker.exe"
if (-not (Test-Path $MainExe)) { throw "Missing build output: $MainExe" }
if (-not (Test-Path $WorkerExe)) { throw "Missing build output: $WorkerExe" }

$Wix = Get-Command wix -ErrorAction SilentlyContinue
if (-not $Wix) {
    dotnet tool install --global wix --version "5.*"
    if ($LASTEXITCODE -ne 0) { throw "Installing the WiX tool failed." }
    $Wix = Get-Command wix -ErrorAction SilentlyContinue
}
if (-not $Wix) { throw "The wix command is not available on PATH." }

New-Item -ItemType Directory -Force -Path $PackagesDir | Out-Null
$MsiPath = Join-Path $PackagesDir "Audacity-RS-$Version-x64.msi"
$WxsPath = Join-Path $ScriptDir "audacity-rs.wxs"
& wix build $WxsPath -arch x64 -d "Version=$Version" -d "BinDir=$BinDir" -o $MsiPath
if ($LASTEXITCODE -ne 0) { throw "WiX MSI build failed." }

if ($env:WINDOWS_CERT_PFX_BASE64 -and $env:WINDOWS_CERT_PASSWORD) {
    $PfxPath = Join-Path ([System.IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString() + ".pfx")
    try {
        [System.IO.File]::WriteAllBytes($PfxPath, [Convert]::FromBase64String($env:WINDOWS_CERT_PFX_BASE64))
        $SignTool = Get-Command signtool.exe -ErrorAction SilentlyContinue
        if (-not $SignTool) { throw "signtool.exe is required to sign the MSI." }
        & $SignTool.Source sign /f $PfxPath /p $env:WINDOWS_CERT_PASSWORD /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 $MsiPath
        if ($LASTEXITCODE -ne 0) { throw "Signing the MSI failed." }
    }
    finally {
        Remove-Item -Force -ErrorAction SilentlyContinue $PfxPath
    }
}
else {
    Write-Host "unsigned"
}

Write-Output $MsiPath
