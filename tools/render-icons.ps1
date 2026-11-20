<#
.SYNOPSIS
  Renders Kerneloscope's icons: OS logos (from art/logos), tool icons, scroll arrows and the selection
  frames. Each image is written as HTML/CSS/SVG and rendered to PNG with headless Microsoft Edge.
  Called by tools/export-assets.ps1 (with -Scale 2); you normally don't run it directly.

.EXAMPLE
  ./tools/render-icons.ps1 -Scale 2
#>
param(
    [string]$Name = 'icons',
    [string]$Accent = '#ff1f3d',      # accent red used for the selection frames and arrows
    [int]$Scale = 1,                  # pixel density: 2 renders every image at twice the size
    [string]$OutRoot,                 # where to write <Name>/ (default: build/icons)
    [string]$Edge = 'C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe'
)
$ErrorActionPreference = 'Stop'
$root     = Resolve-Path (Join-Path $PSScriptRoot '..')
if (-not $OutRoot) { $OutRoot = Join-Path $root 'build\icons' }
$out      = Join-Path $OutRoot $Name
$icons    = Join-Path $out 'icons'
$work     = Join-Path $root 'build\render'
$cachySvg = Get-Content (Join-Path $root 'art\logos\CachyOS_Logo.svg') -Raw
if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory -Force $icons, $work | Out-Null

function Render([string]$file, [int]$w, [int]$h, [string]$css, [string]$body) {
    $html = Join-Path $work ([IO.Path]::GetFileNameWithoutExtension($file) + '.html')
    @"
<!doctype html><html><head><meta charset="utf-8"><style>
html,body{margin:0;width:${w}px;height:${h}px;overflow:hidden;background:transparent}
$css
</style></head><body>$body</body></html>
"@ | Set-Content -Encoding utf8 $html
    $png = [IO.Path]::GetFullPath($file)
    if (Test-Path $png) { Remove-Item $png }
    $edgeProfile = Join-Path $work 'edge-profile'   # isolated profile so your normal Edge is untouched
    & $Edge --headless=new --disable-gpu --hide-scrollbars "--force-device-scale-factor=$Scale" `
        "--user-data-dir=$edgeProfile" --default-background-color=00000000 `
        "--window-size=$w,$h" "--screenshot=$png" ([Uri]$html).AbsoluteUri 2>$null | Out-Null
    if (-not (Test-Path $png)) { throw "Edge failed to render $file" }
}

# ------------------------------------------------------------------ selection highlights (cut-corner frame + glow)
function SelCss([int]$s, [int]$b) {
    $cut = [int]($s / 9); $edge = [Math]::Max(2, [int]($s / 90)); $glow = [int]($s / 18)
    @"
body{display:flex;align-items:center;justify-content:center}
.f{width:$($s-2*$b)px;height:$($s-2*$b)px;position:relative;filter:drop-shadow(0 0 ${glow}px $Accent)}
.f::before,.f::after{content:'';position:absolute;clip-path:polygon(0 0,calc(100% - ${cut}px) 0,100% ${cut}px,100% 100%,${cut}px 100%,0 calc(100% - ${cut}px))}
.f::before{inset:0;background:$Accent}
.f::after{inset:${edge}px;background:rgba(28,6,10,.92)}
"@
}
Render (Join-Path $out 'selection_big.png') 288 288 (SelCss 288 22) '<div class="f"></div>'
Render (Join-Path $out 'selection_small.png') 108 108 (SelCss 108 10) '<div class="f"></div>'

# ------------------------------------------------------------------ OS icons: official logos
$center = 'body{display:flex;align-items:center;justify-content:center}'
# Windows 11 logo: four equal squares, blue gradient
$win = @'
<svg style="width:184px;height:184px" viewBox="0 0 88 88"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1">
<stop offset="0" stop-color="#2fb5f5"/><stop offset="1" stop-color="#0067c0"/></linearGradient></defs>
<path fill="url(#g)" d="M0 0h42v42H0zM46 0h42v42H46zM0 46h42v42H0zM46 46h42v42H46z"/></svg>
'@
Render (Join-Path $icons 'os_win.png') 256 256 $center $win
Copy-Item (Join-Path $icons 'os_win.png') (Join-Path $icons 'os_win8.png')
$cachy = ($cachySvg -replace '<\?xml[^>]*\?>', '') -replace 'width="[^"]*" height="[^"]*"', 'width="200" height="200"'
Render (Join-Path $icons 'os_cachyos.png') 256 256 $center $cachy

# Linux distros. menu.conf's `icon <name>` picks os_<name>; each logo is also saved under common aliases
# (os-release IDs and EFI folder names), so `icon pop_os` or `icon opensuse-tumbleweed` work too.
# Key = logo file in art/logos (".mono" = single-colour mark, tinted with the brand colour).
$distros = [ordered]@{
    'bazzite'     = @{ Aliases = @() }
    'ubuntu'      = @{ Aliases = @() }
    'kubuntu'     = @{ Mono = '#0079c1'; Aliases = @() }
    'debian'      = @{ Aliases = @() }
    'arch'        = @{ Aliases = @('archlinux') }
    'linuxmint'   = @{ Aliases = @('mint') }
    'pop'         = @{ Mono = '#48b9c7'; Aliases = @('popos', 'pop_os', 'pop-os') }
    'fedora'      = @{ Aliases = @() }
    'nobara'      = @{ Mono = '#e8e8ee'; Aliases = @() }
    'opensuse'    = @{ Aliases = @('suse', 'opensuse-tumbleweed', 'opensuse-leap', 'opensuse-aeon') }
    'manjaro'     = @{ Aliases = @() }
    'endeavouros' = @{ Mono = '#7f7fff'; Aliases = @() }
    'garuda'      = @{ Aliases = @() }
    'zorin'       = @{ Mono = '#15a6f0'; Aliases = @() }
    'elementary'  = @{ Mono = '#64baff'; Aliases = @() }
    'nixos'       = @{ Aliases = @() }
    'kali'        = @{ Aliases = @() }
    'gentoo'      = @{ Aliases = @() }
    'void'        = @{ Aliases = @() }
    'steamos'     = @{ Mono = '#1a9fff'; Aliases = @() }
    'linux'       = @{ Aliases = @('unknown') }    # Tux: generic fallback for anything unrecognised
}
$logoDir = Join-Path $root 'art\logos'
foreach ($d in $distros.GetEnumerator()) {
    $mono = [bool]$d.Value.Mono
    $file = Join-Path $logoDir ("$($d.Key)" + $(if ($mono) { '.mono.svg' } else { '.svg' }))
    if (-not (Test-Path $file)) { Write-Warning "missing logo $file"; continue }
    $svg = (Get-Content $file -Raw) -replace '<\?xml[^>]*\?>', ''
    # logos without a viewBox can't scale (they get cropped): derive one from width/height
    if ($svg -notmatch 'viewBox=' -and $svg -match '<svg[^>]*?\swidth="([\d.]+)(px)?"[^>]*?\sheight="([\d.]+)(px)?"') {
        $svg = $svg -replace '<svg', "<svg viewBox=`"0 0 $($Matches[1]) $($Matches[3])`""
    }
    # black parts of a logo vanish on the dark background: lighten them
    $svg = $svg -replace 'fill="#000000"', 'fill="#e6e6ea"' -replace 'fill:#000000', 'fill:#e6e6ea'
    $fill = if ($mono) { "svg path{fill:$($d.Value.Mono)}" } else { '' }
    Render (Join-Path $icons "os_$($d.Key).png") 256 256 "$center svg{width:176px;height:176px} $fill" $svg
    foreach ($a in $d.Value.Aliases) { Copy-Item (Join-Path $icons "os_$($d.Key).png") (Join-Path $icons "os_$a.png") }
}

# scroll arrows (shown only when there are more OS entries than fit on screen)
foreach ($dir in @(@('arrow_left.png', 'M15 3 6 12l9 9'), @('arrow_right.png', 'M9 3l9 9-9 9'))) {
    Render (Join-Path $icons $dir[0]) 64 64 "$center svg{width:40px;height:40px;filter:drop-shadow(0 0 6px $Accent)}" `
        "<svg viewBox='0 0 24 24'><path d='$($dir[1])' fill='none' stroke='$Accent' stroke-width='2.6' stroke-linecap='square'/></svg>"
}

# ------------------------------------------------------------------ tool icons (Segoe Fluent Icons glyphs, same as Windows' power menu)
$tools = [ordered]@{ 'func_shutdown.png' = 'E7E8'; 'func_reset.png' = 'E777'; 'func_firmware.png' = 'E713'; 'func_menu.png' = 'E9E9' }
foreach ($t in $tools.GetEnumerator()) {
    Render (Join-Path $icons $t.Key) 96 96 "$center body{font:40px 'Segoe Fluent Icons','Segoe MDL2 Assets';color:#c9c9d2}" "<span>&#x$($t.Value);</span>"
}
Render (Join-Path $icons 'transparent.png') 16 16 '' ''

Write-Host "Icons written to $out"
