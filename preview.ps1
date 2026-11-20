<#
.SYNOPSIS
  Builds Kerneloscope and boots it in QEMU (Windows host). Nothing on your real disks is touched.
  The virtual EFI partition holds copies of kerneloscope.efi as stand-in "Windows" / "Linux kernel" /
  distro loaders, so choosing an entry really chain-loads another EFI program: a second copy of the
  menu starts (with its intro), which shows the hand-off worked.
  Needs QEMU for Windows (winget install SoftwareFreedomConservancy.QEMU) and the Rust UEFI target.

.EXAMPLE
  ./preview.ps1                                    # interactive window (arrows / Enter / Tab / Esc)
  ./preview.ps1 -Screenshot shots\main.png         # headless; saves 3 frames (main-1/2/3.png) to show the motion
  ./preview.ps1 -Screenshot shots\sub.png -Keys right,tab
  ./preview.ps1 -Distros bazzite,ubuntu,arch       # add mock entries
#>
param(
    [string]$Screenshot,
    [string[]]$Keys = @(),
    [string[]]$Distros = @(),
    [int]$WaitSeconds = 25,
    [int]$Frames = 3,
    [int]$FrameIntervalMs = 1500,
    [switch]$WaitForMenu,           # start capturing as soon as the menu reports it is up (to catch the intro)
    [switch]$NoDebug,               # run like the real install: no diagnostics on the console
    [string[]]$ConfLines = @(),     # extra menu.conf settings for this run, e.g. 'intro off'
    [int]$SettleSeconds = 0,        # extra wait after the keys before capturing (e.g. to watch a chain-loaded OS start)
    [int]$Timeout = -1,             # override menu.conf's countdown (0 = wait forever); headless runs default to 0
    [int]$Width = 2560, [int]$Height = 1600,
    [switch]$NoBuild,
    [string]$BuildDir = 'build',    # working folder (virtual EFI partition, firmware vars, serial.log); use another to run two previews at once
    [string]$Display = 'sdl',       # interactive window backend: sdl (GPU-scaled, light) or gtk
    [ValidateSet('whpx', 'tcg')] [string]$Accel = 'whpx',   # whpx = Windows hypervisor (fast); QEMU falls back to tcg (software) if unavailable
    [string]$Qemu = 'C:\Program Files\qemu'
)
$ErrorActionPreference = 'Stop'
# Interactive window: pick a virtual screen that fits the real display at 1:1 pixels (about 85% of it), so
# the window is never rescaled. Rescaling is what makes edges look blocky/blurry in the preview; the real
# boot screen has no window and no scaling.
if (-not $Screenshot -and -not $PSBoundParameters.ContainsKey('Width') -and -not $PSBoundParameters.ContainsKey('Height')) {
    $vc = Get-CimInstance Win32_VideoController -ErrorAction SilentlyContinue | Where-Object CurrentHorizontalResolution | Select-Object -First 1
    if ($vc) {
        $Width = [int][Math]::Floor($vc.CurrentHorizontalResolution * 0.85 / 8) * 8
        $Height = [int][Math]::Floor($vc.CurrentVerticalResolution * 0.85 / 8) * 8
        Write-Host "Preview screen: ${Width}x${Height} (85% of your $($vc.CurrentHorizontalResolution)x$($vc.CurrentVerticalResolution) display, shown 1:1)"
    }
}
$root   = $PSScriptRoot
$build  = Join-Path $root $BuildDir
$esp    = Join-Path $build 'esp'

if (-not $NoBuild) {
    Push-Location $root
    try { cargo build --release --target x86_64-unknown-uefi; if ($LASTEXITCODE) { throw 'cargo build failed' } } finally { Pop-Location }
}
$efi = Join-Path $root 'target\x86_64-unknown-uefi\release\kerneloscope.efi'

# ---- virtual EFI partition
if (Test-Path $esp) { Remove-Item -Recurse -Force $esp }
New-Item -ItemType Directory -Force "$esp\EFI\BOOT", "$esp\EFI\kerneloscope", "$esp\EFI\Microsoft\Boot", "$esp\EFI\refind" | Out-Null
Copy-Item $efi "$esp\EFI\BOOT\BOOTX64.EFI"
Copy-Item $efi "$esp\EFI\kerneloscope\kerneloscope.efi"
# stand-ins for the loaders named in menu.conf: copies of the menu itself
Copy-Item $efi "$esp\EFI\Microsoft\Boot\bootmgfw.efi"
Copy-Item $efi "$esp\vmlinuz-linux-cachyos"
Copy-Item $efi "$esp\EFI\refind\refind_x64.efi"
$conf = Get-Content (Join-Path $root 'menu.conf') -Raw
$names = @{ bazzite = 'Bazzite'; ubuntu = 'Ubuntu'; arch = 'Arch Linux'; linuxmint = 'Linux Mint'; pop = 'Pop!_OS'; fedora = 'Fedora'; debian = 'Debian'; nobara = 'Nobara'; opensuse = 'openSUSE' }
foreach ($d in $Distros) {
    New-Item -ItemType Directory -Force "$esp\EFI\$d" | Out-Null
    Copy-Item $efi "$esp\EFI\$d\grubx64.efi"
    $t = if ($names[$d]) { $names[$d] } else { $d }
    $conf += "`nentry `"$t`"`n  icon $d`n  loader \EFI\$d\grubx64.efi`n  sub `"$t (recovery mode)`"`n    options `"single`"`n"
}
if ($Timeout -lt 0 -and $Screenshot) { $Timeout = 0 }
if ($Timeout -ge 0) { $conf = $conf -replace '(?m)^timeout\s+\d+', "timeout $Timeout" }
# test the requested screen size (on real hardware the default is `resolution native`)
$extra = (@("resolution $($Width)x$($Height)") + $(if ($NoDebug) { @() } else { @('debug on') }) + $ConfLines) -join "`n"
$conf = $conf -replace '(?m)^timeout', "$extra`ntimeout"
$conf | Set-Content -Encoding utf8NoBOM "$esp\EFI\kerneloscope\menu.conf"

# ---- firmware
$code = Join-Path $build 'OVMF_CODE.fd'; $vars = Join-Path $build 'OVMF_VARS.fd'
if (-not (Test-Path $code)) { Copy-Item (Join-Path $Qemu 'share\edk2-x86_64-code.fd') $code }
if (-not (Test-Path $vars)) { Copy-Item (Join-Path $Qemu 'share\edk2-i386-vars.fd') $vars }

$qemuArgs = @('-machine', 'q35', '-m', '1024', '-accel', $Accel, '-accel', 'tcg', '-smp', '2',
    '-drive', 'if=pflash,format=raw,readonly=on,file=OVMF_CODE.fd',
    '-drive', 'if=pflash,format=raw,file=OVMF_VARS.fd',
    '-drive', 'file=fat:rw:esp,format=raw,if=ide',
    '-vga', 'none', '-device', "VGA,vgamem_mb=64,edid=on,xres=$Width,yres=$Height",
    '-net', 'none', '-boot', 'menu=off', '-serial', 'file:serial.log')

Push-Location $build
try {
    if (-not $Screenshot) {
        $disp = if ($Display -eq 'gtk') { 'gtk,zoom-to-fit=on' } else { $Display }
        # Windows display scaling (e.g. 150%) would otherwise stretch QEMU's window; mark it DPI-aware so
        # guest pixels map 1:1 to screen pixels, and ask SDL for smooth filtering if the window is resized.
        $env:__COMPAT_LAYER = 'HighDpiAware'
        $env:SDL_RENDER_SCALE_QUALITY = 'linear'
        & (Join-Path $Qemu 'qemu-system-x86_64.exe') @qemuArgs -display $disp -name 'Kerneloscope preview'; return }
    $port = 4456
    $p = Start-Process -PassThru -WindowStyle Hidden -RedirectStandardError (Join-Path $build 'qemu-err.txt') (Join-Path $Qemu 'qemu-system-x86_64.exe') `
        -ArgumentList ($qemuArgs + @('-display', 'none', '-qmp', "tcp:127.0.0.1:$port,server=on,wait=off"))
    if ($WaitForMenu) {
        $deadline = (Get-Date).AddSeconds(120)
        $log = Join-Path $build 'serial.log'
        $ready = { (Test-Path $log) -and [bool](Select-String -Path $log -Pattern 'bootable entries' -Quiet) }
        while ((Get-Date) -lt $deadline -and -not (& $ready)) { Start-Sleep -Milliseconds 200 }
    } else { Start-Sleep -Seconds $WaitSeconds }
    if ($p.HasExited) { throw "QEMU exited early: $(Get-Content (Join-Path $build 'qemu-err.txt') -Raw)" }
    $tcp = [Net.Sockets.TcpClient]::new('127.0.0.1', $port)
    $s = $tcp.GetStream(); $rd = [IO.StreamReader]::new($s); $wr = [IO.StreamWriter]::new($s); $wr.AutoFlush = $true
    function Q($obj) { $wr.WriteLine(($obj | ConvertTo-Json -Compress -Depth 6)); Start-Sleep -Milliseconds 300 }
    $null = $rd.ReadLine(); Q @{ execute = 'qmp_capabilities' }
    foreach ($k in $Keys) { Q @{ execute = 'send-key'; arguments = @{ keys = @(@{ type = 'qcode'; data = $k }) } }; Start-Sleep -Seconds 3 }
    Start-Sleep -Seconds $SettleSeconds
    $base = [IO.Path]::GetFullPath((Join-Path $root $Screenshot)); New-Item -ItemType Directory -Force (Split-Path $base) | Out-Null
    for ($i = 1; $i -le $Frames; $i++) {
        $f = if ($Frames -eq 1) { $base } else { $base -replace '\.png$', "-$i.png" }
        Q @{ execute = 'screendump'; arguments = @{ filename = $f; format = 'png' } }
        Write-Host "Saved $f"; Start-Sleep -Milliseconds $FrameIntervalMs
    }
    Q @{ execute = 'quit' }; $tcp.Close(); $p.WaitForExit(10000) | Out-Null
}
finally { Pop-Location }
