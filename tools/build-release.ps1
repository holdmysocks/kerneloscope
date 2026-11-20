<#
.SYNOPSIS
  Builds the distributable dist/kerneloscope.efi.
  Rust embeds source paths in panic messages; this rewrites the local ones (your cargo home and this
  folder) to neutral prefixes, so the published binary doesn't contain your user name or folder layout.
.EXAMPLE
  ./tools/build-release.ps1
#>
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $HOME '.cargo' }
$env:RUSTFLAGS = "--remap-path-prefix=$cargoHome=/cargo --remap-path-prefix=$root=/kerneloscope"
Push-Location $root
try {
    cargo build --release --target x86_64-unknown-uefi
    if ($LASTEXITCODE) { throw 'cargo build failed' }
} finally { Pop-Location; Remove-Item Env:RUSTFLAGS }
$efi = Join-Path $root 'target\x86_64-unknown-uefi\release\kerneloscope.efi'
New-Item -ItemType Directory -Force (Join-Path $root 'dist') | Out-Null
Copy-Item $efi (Join-Path $root 'dist\kerneloscope.efi') -Force
# safety check: no local paths left in the binary
$text = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($efi))
$leaks = @($HOME, $root) | Where-Object { $text.Contains($_) }
if ($leaks) { throw "local paths still embedded: $($leaks -join ', ')" }
Write-Host "dist\kerneloscope.efi updated ($([Math]::Round((Get-Item $efi).Length / 1KB)) KB), no local paths embedded"
