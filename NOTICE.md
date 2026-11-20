# Third-party material

Kerneloscope's source code is licensed under the GNU GPL v3 or later (see [LICENSE](LICENSE)).
The artwork bundled with it comes from the sources below and keeps its own licence.

## Operating-system logos (`art/logos/`, rendered into `assets/os_*.bin`)

All logos are trademarks of their respective owners. They're used only to identify each operating system in the boot menu; this project isn't affiliated with or endorsed by any of them.

| Logo(s) | Source | Licence |
|---|---|---|
| CachyOS (`CachyOS_Logo.svg`) | [Wikimedia Commons](https://commons.wikimedia.org/wiki/File:CachyOS_Logo.svg), by ptr1337 | GPL |
| Bazzite (`bazzite.svg`) | [Bazzite press kit](https://github.com/ublue-os/bazzite/tree/main/press_kit) | Apache-2.0 repository; see its brand guidance |
| Ubuntu, Debian, Arch, Linux Mint, Fedora, openSUSE, Manjaro, NixOS, Kali, Void, Gentoo, Garuda, Tux (`linux.svg`) | [homarr-labs/dashboard-icons](https://github.com/homarr-labs/dashboard-icons) | Apache-2.0 (Tux originally by Larry Ewing, made with The GIMP) |
| Pop!_OS, Zorin, elementary, EndeavourOS, Kubuntu, Nobara, Steam Deck (`*.mono.svg`) | [simple-icons](https://simpleicons.org) | CC0-1.0 |
| Windows | Drawn for this project as four squares in the Windows 11 colours | Windows is a trademark of Microsoft |

## Fonts and tool glyphs

The text and the tool icons (power, restart, firmware, settings) are pre-rendered into bitmap images in `assets/*.bin` from fonts installed with Windows: **Bahnschrift** (text) and **Segoe Fluent Icons** (tool glyphs), both by Microsoft. No font files are included. If you redistribute modified builds, you may prefer to re-export the assets with openly licensed fonts (change the font names in `tools/export-assets.ps1` and `tools/render-icons.ps1`).

## Libraries (compiled into `kerneloscope.efi`)

- [`uefi`](https://crates.io/crates/uefi) / `uefi-raw` / `uefi-macros`: MIT or Apache-2.0
- [`miniz_oxide`](https://crates.io/crates/miniz_oxide) and `adler2`: MIT, Zlib or Apache-2.0
