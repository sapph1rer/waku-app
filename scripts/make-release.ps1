# Build Waku and publish a GitHub Release the app can update itself from.
#
#   1. set the new version in crates/waku-app/Cargo.toml   (version = "0.2.0")
#   2. powershell -ExecutionPolicy Bypass -File scripts\make-release.ps1 -Notes "what changed"
#        -NoPublish   only build dist\ (no GitHub)
#        -Draft       publish as a draft release
#
# Needs: cargo, and the GitHub CLI (`gh auth login` once) unless -NoPublish.
# Result: dist\waku-windows-x64.zip (waku-app.exe, waku-browser.exe, libmpv-2.dll) + .sha256 — the app only
# installs a zip whose checksum file is published beside it.
param(
    [string]$Version = "",
    [string]$Notes = "",
    [switch]$NoPublish,
    [switch]$Draft
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$cargoVer = (Select-String -Path "crates/waku-app/Cargo.toml" -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
if (-not $Version) { $Version = $cargoVer }
$Version = $Version.TrimStart("v")
if ($Version -ne $cargoVer) { throw "crates/waku-app/Cargo.toml has version $cargoVer - set version = `"$Version`" there first (the app compares it with the release tag)" }

Write-Host "== building Waku $Version"
cargo build --release -p waku-app -p waku-browser
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$dist = "dist/waku-windows-x64"
Remove-Item -Recurse -Force "dist" -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $dist | Out-Null
Copy-Item "target/release/waku-app.exe", "target/release/waku-browser.exe" $dist
$mpv = @("target/release/libmpv-2.dll", "../rmp/third_party/libmpv/libmpv-2.dll") | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $mpv) { throw "libmpv-2.dll not found (target/release or ../rmp/third_party/libmpv)" }
Copy-Item $mpv $dist

$zip = "dist/waku-windows-x64.zip"
Compress-Archive -Path "$dist/*" -DestinationPath $zip
$hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
Set-Content -NoNewline -Encoding ascii "$zip.sha256" "$hash  waku-windows-x64.zip"
Write-Host "== $zip  sha256 $hash"

if ($NoPublish) { Write-Host "built only (-NoPublish)"; exit 0 }
if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { throw "GitHub CLI (gh) not found - install it, run 'gh auth login', or use -NoPublish and upload dist\ by hand" }
if (-not $Notes) { $Notes = "Waku $Version" }
$args = @("release", "create", "v$Version", $zip, "$zip.sha256", "--title", "Waku v$Version", "--notes", $Notes)
if ($Draft) { $args += "--draft" }
gh @args
if ($LASTEXITCODE -ne 0) { throw "gh release create failed" }
Write-Host "== published v$Version - running copies offer it in Settings > Updates"
