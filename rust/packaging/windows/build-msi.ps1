param(
    [switch]$SkipBuild,
    # Reuse an existing staged tree (e.g. rust/target/win-stage/Audacity-RS from build-cross.sh).
    [string]$StageDir,
    # Where the runtime DLLs live (default: $env:AU_DLL_DIR, else C:\msys64\mingw64\bin).
    [string]$DllDir
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
        # Same set as build-cross.sh: the GUI app plus every helper exe.
        cargo build --release --workspace --features au-app/gui
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed." }
    }
    finally {
        Pop-Location
    }
}

$Metadata = (& cargo metadata --manifest-path (Join-Path $RustDir "Cargo.toml") --no-deps --format-version 1) | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw "cargo metadata failed." }
$AppPackage = $Metadata.packages | Where-Object { $_.name -eq "au-app" } | Select-Object -First 1
$Version = if ($AppPackage) { $AppPackage.version } else { "0.0.0" }

# Stage exactly what build-cross.sh ships (exes, runtime DLLs, Nyquist runtime, plug-ins, locales)
# so the MSI and the portable zip have the same contents.
if (-not $StageDir) {
    $StageDir = Join-Path $RustDir "target/win-stage/Audacity-RS"
    if (Test-Path $StageDir) { Remove-Item -Recurse -Force $StageDir }
    New-Item -ItemType Directory -Force -Path $StageDir | Out-Null
    foreach ($exe in @("audacity-rs", "au-nyquist-engine", "au-nyquist-worker", "au-plugin-editor", "au-plugin-scan")) {
        $p = Join-Path $BinDir "$exe.exe"
        if (-not (Test-Path $p)) { throw "Missing build output: $p" }
        Copy-Item $p $StageDir
    }
    # Runtime DLLs the exes import, resolved recursively from $DllDir (MSYS2 mingw64\bin by default).
    if (-not $DllDir) { $DllDir = if ($env:AU_DLL_DIR) { $env:AU_DLL_DIR } else { "C:\msys64\mingw64\bin" } }
    $Objdump = Get-Command objdump -ErrorAction SilentlyContinue
    if (-not $Objdump) { $Objdump = Get-Command (Join-Path $DllDir "objdump.exe") -ErrorAction SilentlyContinue }
    if (-not $Objdump) { throw "objdump (MSYS2 mingw-w64-x86_64-binutils) is required to collect runtime DLLs." }
    $queue = [System.Collections.Generic.Queue[string]]::new()
    Get-ChildItem $StageDir -Filter *.exe | ForEach-Object { $queue.Enqueue($_.FullName) }
    while ($queue.Count -gt 0) {
        $f = $queue.Dequeue()
        $deps = & $Objdump.Source -p $f | Select-String 'DLL Name:\s*(\S+)' | ForEach-Object { $_.Matches[0].Groups[1].Value }
        foreach ($d in $deps) {
            $src = Join-Path $DllDir $d; $dst = Join-Path $StageDir $d
            if ((Test-Path $src) -and -not (Test-Path $dst)) { Copy-Item $src $dst; $queue.Enqueue($dst) }
        }
    }
    $Nyq = Join-Path $StageDir "nyquist"
    Copy-Item -Recurse (Join-Path $RepoRoot "share/nyquist-runtime") $Nyq
    Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $Nyq "CMakeLists.txt")
    $Plug = Join-Path $StageDir "plug-ins"; New-Item -ItemType Directory -Force -Path $Plug | Out-Null
    Copy-Item (Join-Path $RepoRoot "share/nyquist-plug-ins/*.ny") $Plug
    $Msgfmt = Get-Command msgfmt -ErrorAction SilentlyContinue
    foreach ($po in Get-ChildItem (Join-Path $RepoRoot "au3/locale") -Filter *.po) {
        $dir = Join-Path $StageDir "locale/$($po.BaseName)/LC_MESSAGES"; New-Item -ItemType Directory -Force -Path $dir | Out-Null
        if ($Msgfmt) { & $Msgfmt.Source -o (Join-Path $dir "audacity.mo") $po.FullName 2>$null }
        if (-not $Msgfmt -or $LASTEXITCODE -ne 0) { Copy-Item $po.FullName (Join-Path $dir "audacity.po") }
    }
}
foreach ($exe in @("audacity-rs", "au-nyquist-engine", "au-nyquist-worker", "au-plugin-editor", "au-plugin-scan")) {
    if (-not (Test-Path (Join-Path $StageDir "$exe.exe"))) { throw "Stage is missing $exe.exe: $StageDir" }
}

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
& wix build $WxsPath -arch x64 -d "Version=$Version" -d "StageDir=$StageDir" -o $MsiPath
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
