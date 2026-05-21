$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$Version = "0.1.2"
$DistDir = Join-Path $RepoRoot "dist"
$InstallerManifest = Join-Path $PSScriptRoot "installer\Cargo.toml"
$IpSshBinary = Join-Path $RepoRoot "target\release\ipssh.exe"
$InstallerBinary = Join-Path $DistDir "ipssh-$Version-windows-x64-installer.exe"
$CargoInstallerBinary = Join-Path $PSScriptRoot "installer\target\release\ipssh-installer.exe"

Set-Location $RepoRoot
cargo build --release

if (-not (Test-Path $IpSshBinary)) {
    throw "Expected binary was not produced: $IpSshBinary"
}

New-Item -ItemType Directory -Force -Path $DistDir | Out-Null

$env:IPSSH_BIN = $IpSshBinary
cargo build --release --manifest-path $InstallerManifest
Copy-Item -Force $CargoInstallerBinary $InstallerBinary

Write-Host "Created installer: $InstallerBinary"
