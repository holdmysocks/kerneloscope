<#
.SYNOPSIS
  Updates the installed Kerneloscope from Windows (no need to boot into Linux).
  Finds the EFI partition that already holds \EFI\kerneloscope\ (never the Windows boot disk), mounts it
  on a temporary drive letter, backs up the installed kerneloscope.efi as kerneloscope.efi.bak, copies in
  dist\kerneloscope.efi, then removes the drive letter. menu.conf and saved settings are left alone.
  Needs administrator rights (Windows will show a UAC prompt).

.EXAMPLE
  ./tools/update-from-windows.ps1
#>
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..')
$newEfi = Join-Path $root 'dist\kerneloscope.efi'

$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) {
    # relaunch elevated in a visible window so the result can be read
    Start-Process pwsh -Verb RunAs -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-NoExit', '-File', "`"$PSCommandPath`"")
    Write-Host 'Opened an administrator window (approve the UAC prompt); the result is shown there.'
    return
}

Start-Transcript -Path (Join-Path $env:TEMP 'kerneloscope-update.log') -Force | Out-Null
$espGuid = '{c12a7328-f81f-11d2-ba4b-00a0c93ec93b}'
$bootDisks = (Get-Disk | Where-Object { $_.IsBoot -or $_.IsSystem }).Number
$candidates = Get-Partition | Where-Object { $_.GptType -eq $espGuid -and $_.DiskNumber -notin $bootDisks }
if (-not $candidates) { throw 'No EFI partition found outside the Windows boot disk.' }

$letter = (68..90 | ForEach-Object { [char]$_ } | Where-Object { -not (Test-Path "$($_):\") } | Select-Object -Last 1)
$done = $false
foreach ($p in $candidates) {
    $disk = Get-Disk -Number $p.DiskNumber
    Write-Host ("Checking disk {0} ({1}, {2:N0} GB) partition {3}..." -f $p.DiskNumber, $disk.FriendlyName, ($disk.Size / 1GB), $p.PartitionNumber)
    Add-PartitionAccessPath -DiskNumber $p.DiskNumber -PartitionNumber $p.PartitionNumber -AccessPath "$($letter):\"
    try {
        $dir = "$($letter):\EFI\kerneloscope"
        if (-not (Test-Path "$dir\kerneloscope.efi")) { Write-Host '  no Kerneloscope install here, skipping'; continue }
        Copy-Item "$dir\kerneloscope.efi" "$dir\kerneloscope.efi.bak" -Force
        Copy-Item $newEfi "$dir\kerneloscope.efi" -Force
        $same = (Get-FileHash $newEfi).Hash -eq (Get-FileHash "$dir\kerneloscope.efi").Hash
        Write-Host "  kerneloscope.efi updated (previous version kept as kerneloscope.efi.bak), verified: $same"
        $done = $true
    } finally {
        Remove-PartitionAccessPath -DiskNumber $p.DiskNumber -PartitionNumber $p.PartitionNumber -AccessPath "$($letter):\"
    }
}
if (-not $done) { throw 'Kerneloscope is not installed on any EFI partition found (run install-menu.sh from Linux first).' }
Write-Host 'Done. The temporary drive letter has been removed.'
Stop-Transcript | Out-Null
