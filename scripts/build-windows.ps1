# ============================================================
#  VEDIT — one-command Windows build + installer packaging
#
#  Usage (PowerShell, from the VEDIT root):
#    .\scripts\build-windows.ps1                 # build vedit.exe + collect portable dist
#    .\scripts\build-windows.ps1 -BuildInstaller # also compile VEDIT-Setup-<ver>.exe (needs Inno Setup 6)
#
#  What it does:
#    1. cargo build --release (vedit.exe)
#    2. downloads a Windows ffmpeg build (if dist\VEDIT\ffmpeg is empty)
#    3. assembles dist\VEDIT\  (portable app: vedit.exe + ffmpeg\)
#    4. optionally compiles the Inno Setup installer
# ============================================================

[CmdletBinding()]
param(
    [switch]$BuildInstaller,
    [switch]$SkipFfmpegDownload,
    [string]$Version = "0.2.0"
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot

Write-Host "=== VEDIT build script ===" -ForegroundColor Yellow

# --- 1. Rust toolchain -------------------------------------------------------
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host "Rust not found. Install it from https://rustup.rs then re-run." -ForegroundColor Red
    Write-Host "You also need the MSVC C++ build tools (Visual Studio Build Tools, 'Desktop development with C++')." -ForegroundColor Red
    exit 1
}

Write-Host "[1/4] Building vedit (release)..." -ForegroundColor Cyan
Push-Location $Root
cargo build --release -p vedit-gui
if ($LASTEXITCODE -ne 0) { Pop-Location; Write-Host "cargo build failed" -ForegroundColor Red; exit 1 }
Pop-Location

# --- 2. ffmpeg ---------------------------------------------------------------
$FfmpegDir = Join-Path $Root "dist\VEDIT\ffmpeg"
$NeedFfmpeg = -not (Test-Path (Join-Path $FfmpegDir "ffmpeg.exe"))
if ($NeedFfmpeg -and -not $SkipFfmpegDownload) {
    Write-Host "[2/4] Downloading ffmpeg (Windows essentials build)..." -ForegroundColor Cyan
    $Zip = Join-Path $env:TEMP "ffmpeg-release-essentials.zip"
    Invoke-WebRequest -Uri "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip" -OutFile $Zip
    $Tmp = Join-Path $env:TEMP "ffmpeg-extract"
    if (Test-Path $Tmp) { Remove-Item $Tmp -Recurse -Force }
    Expand-Archive -Path $Zip -DestinationPath $Tmp -Force
    $Bin = Get-ChildItem $Tmp -Recurse -Filter "ffmpeg.exe" | Select-Object -First 1
    $SrcDir = Split-Path -Parent $Bin.FullName
    New-Item -ItemType Directory -Force -Path $FfmpegDir | Out-Null
    Copy-Item (Join-Path $SrcDir "ffmpeg.exe") $FfmpegDir
    Copy-Item (Join-Path $SrcDir "ffprobe.exe") $FfmpegDir
    Remove-Item $Zip -Force -ErrorAction SilentlyContinue
    Remove-Item $Tmp -Recurse -Force -ErrorAction SilentlyContinue
} elseif ($SkipFfmpegDownload) {
    Write-Host "[2/4] Skipping ffmpeg download (-SkipFfmpegDownload)." -ForegroundColor DarkYellow
    if ($NeedFfmpeg) {
        Write-Host "      Place ffmpeg.exe + ffprobe.exe into $FfmpegDir yourself." -ForegroundColor DarkYellow
    }
} else {
    Write-Host "[2/4] ffmpeg already present in dist\VEDIT\ffmpeg." -ForegroundColor DarkCyan
}

# --- 3. portable dist --------------------------------------------------------
Write-Host "[3/4] Assembling portable dist..." -ForegroundColor Cyan
Copy-Item (Join-Path $Root "target\release\vedit.exe") (Join-Path $Root "dist\VEDIT\vedit.exe") -Force
Copy-Item (Join-Path $Root "README.md") (Join-Path $Root "dist\VEDIT\README.md") -Force -ErrorAction SilentlyContinue
Copy-Item (Join-Path $Root "LICENSE") (Join-Path $Root "dist\VEDIT\LICENSE") -Force -ErrorAction SilentlyContinue

# Quick smoke test: does the editor see its ffmpeg?
$env:VEDIT_FFMPEG_DIR = $FfmpegDir
Write-Host "      dist\VEDIT is ready (vedit.exe + ffmpeg\)." -ForegroundColor Green

# --- 4. installer ------------------------------------------------------------
if ($BuildInstaller) {
    Write-Host "[4/4] Compiling installer with Inno Setup..." -ForegroundColor Cyan
    $Iscc = Get-Command iscc -ErrorAction SilentlyContinue
    $IsccPath = @(
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1

    if ($Iscc) { $IsccCmd = $Iscc.Source }
    elseif ($IsccPath) { $IsccCmd = $IsccPath }
    else {
        Write-Host "Inno Setup 6 not found. Install it from https://jrsoftware.org/isdl.php" -ForegroundColor Red
        Write-Host "  winget install JRSoftware.InnoSetup   (or choco install innosetup)" -ForegroundColor Red
        exit 1
    }
    & $IsccCmd /DAppVersion=$Version (Join-Path $Root "installer\vedit.iss")
    if ($LASTEXITCODE -ne 0) { Write-Host "ISCC failed" -ForegroundColor Red; exit 1 }
    Write-Host "      installer written to installer\Output\VEDIT-Setup-$Version.exe" -ForegroundColor Green
} else {
    Write-Host "[4/4] Skipped installer (use -BuildInstaller to build VEDIT-Setup-$Version.exe)." -ForegroundColor DarkCyan
}

Write-Host "Done. Portable app: dist\VEDIT\vedit.exe" -ForegroundColor Yellow
