<#
.SYNOPSIS
  Boots the menu headless in QEMU at a range of screen sizes/shapes and saves a screenshot of each
  (after the intro) to shots/screens/<W>x<H>.png. Also prints the frame-rate log for each size.
.EXAMPLE
  ./tools/test-screens.ps1
  ./tools/test-screens.ps1 -Sizes 3440x1440,5120x1440
#>
param(
    [string[]]$Sizes = @('2560x1600', '1920x1080', '3440x1440', '5120x1440', '3840x2160', '1280x1024', '1366x768'),
    [string[]]$Distros = @('bazzite', 'ubuntu', 'arch')
)
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..')
foreach ($s in $Sizes) {
    $w, $h = $s -split 'x' | ForEach-Object { [int]$_ }
    $log = Join-Path $root 'build-test\serial.log'
    if (Test-Path $log) { Remove-Item $log }
    & (Join-Path $root 'preview.ps1') -NoBuild -BuildDir 'build-test' -WaitForMenu -SettleSeconds 7 -Frames 1 -Width $w -Height $h `
        -Distros $Distros -Screenshot "shots\screens\$s.png" | Out-Null
    $lines = Select-String $log -Pattern 'kerneloscope[^\r\n]*' -AllMatches | ForEach-Object { $_.Matches.Value }
    $screen = ($lines | Where-Object { $_ -match 'screen ' }) -replace 'kerneloscope: ', ''
    $perf = ($lines | Where-Object { $_ -match 'perf' } | Select-Object -Last 1) -replace 'kerneloscope perf: ', ''
    $warn = ($lines | Where-Object { $_ -match 'not offered' }) -replace 'kerneloscope: ', ''
    "{0,-10} {1,-18} {2} {3}" -f $s, $screen, $perf, $warn
}
