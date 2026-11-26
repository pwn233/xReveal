# Builds the Windows release assets (docs/spec/v1.md §6): a per-user installer and a portable .zip,
# in dist\. The C runtime is linked statically, so nothing else needs installing.
# Usage: scripts/package-windows.ps1 [-Version x.y.z]   (default: Cargo.toml's version)
param([string]$Version = "")
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

if (-not $Version) {
    $Version = (Select-String -Path Cargo.toml -Pattern '^version = "(.*)"' | Select-Object -First 1).Matches.Groups[1].Value
}
$Version = $Version.TrimStart("v")

$env:RUSTFLAGS = "-C target-feature=+crt-static"
cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$out = "dist"
if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory $out | Out-Null
$exe = (Resolve-Path "target\release\xreveal.exe").Path
$base = "xReveal-$Version-windows-x64"

# Portable: the .exe on its own.
Compress-Archive -Path $exe -DestinationPath (Join-Path $out "$base.zip")

# Installer: NSIS (preinstalled on GitHub's Windows runners; otherwise `choco install nsis`).
$makensis = (Get-Command makensis -ErrorAction SilentlyContinue).Source
if (-not $makensis) { $makensis = "${env:ProgramFiles(x86)}\NSIS\makensis.exe" }
$setup = Join-Path (Resolve-Path $out).Path "$base-setup.exe"
& $makensis /V2 "/DVERSION=$Version" "/DEXE=$exe" "/DOUT=$setup" packaging\windows\installer.nsi
if ($LASTEXITCODE -ne 0) { throw "makensis failed" }
Write-Host "package-windows: $setup $out\$base.zip"
