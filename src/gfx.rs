//! Software rendering: a 32-bit canvas (0x00RRGGBB, same byte order as UEFI GOP BltPixel),
//! straight-alpha images (0xAARRGGBB), bilinear scaling, cut-corner frames and bitmap text.

use alloc::vec;
use alloc::vec::Vec;

pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
}

#[derive(Clone)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
}

#[inline]
fn blend_px(dst: u32, src_rgb: u32, a: u32) -> u32 {
    // a: 0..=256
    if a == 0 {
        return dst;
    }
    if a >= 256 {
        return src_rgb & 0x00ff_ffff;
    }
    let inv = 256 - a;
    let rb = (((src_rgb & 0x00ff_00ff) * a + (dst & 0x00ff_00ff) * inv) >> 8) & 0x00ff_00ff;
    let g = (((src_rgb & 0x0000_ff00) * a + (dst & 0x0000_ff00) * inv) >> 8) & 0x0000_ff00;
    rb | g
}

#[inline]
fn add_px(dst: u32, rgb: u32, a: u32) -> u32 {
    let ch = |shift: u32| -> u32 {
        let d = (dst >> shift) & 0xff;
        let s = (((rgb >> shift) & 0xff) * a) >> 8;
        (d + s).min(255) << shift
    };
    ch(16) | ch(8) | ch(0)
}

impl Image {
    /// Decodes an "IMG1" blob produced by tools/export-assets.ps1.
    pub fn decode(blob: &[u8]) -> Image {
        assert!(&blob[0..4] == b"IMG1", "bad image blob");
        let w = u32::from_le_bytes(blob[4..8].try_into().unwrap()) as usize;
        let h = u32::from_le_bytes(blob[8..12].try_into().unwrap()) as usize;
        let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&blob[12..]).expect("inflate");
        let px = raw
            .chunks_exact(4)
            .map(|c| (c[3] as u32) << 24 | (c[2] as u32) << 16 | (c[1] as u32) << 8 | c[0] as u32)
            .collect();
        Image { w, h, px }
    }

    /// Bilinear resample (premultiplied internally, so transparent edges don't get dark halos).
    pub fn scaled(&self, s: f32) -> Image {
        if s > 0.999 && s < 1.001 {
            return self.clone();
        }
        let nw = ((self.w as f32 * s) as usize).max(1);
        let nh = ((self.h as f32 * s) as usize).max(1);
        self.resized(nw, nh)
    }

    /// 2x2 box-filter halving (premultiplied), used before bilinear for big reductions so small
    /// screens don't get jagged, aliased icons and text.
    fn halved(&self) -> Image {
        let (nw, nh) = ((self.w / 2).max(1), (self.h / 2).max(1));
        let mut px = vec![0u32; nw * nh];
        for y in 0..nh {
            for x in 0..nw {
                let mut c = [0u32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let p = self.px[(2 * y + dy).min(self.h - 1) * self.w + (2 * x + dx).min(self.w - 1)];
                    let a = p >> 24;
                    c[0] += ((p >> 16) & 0xff) * a;
                    c[1] += ((p >> 8) & 0xff) * a;
                    c[2] += (p & 0xff) * a;
                    c[3] += a;
                }
                px[y * nw + x] = if c[3] == 0 { 0 } else { (c[3] / 4) << 24 | (c[0] / c[3]) << 16 | (c[1] / c[3]) << 8 | (c[2] / c[3]) };
            }
        }
        Image { w: nw, h: nh, px }
    }

    pub fn resized(&self, nw: usize, nh: usize) -> Image {
        if nw == self.w && nh == self.h {
            return self.clone();
        }
        if nw * 2 <= self.w && nh * 2 <= self.h {
            return self.halved().resized(nw, nh);
        }
        let mut px = vec![0u32; nw * nh];
        let fx = (self.w as u32) * 256 / nw as u32; // 8.8 fixed point step
        let fy = (self.h as u32) * 256 / nh as u32;
        let get = |x: usize, y: usize| -> [u32; 4] {
            let p = self.px[y.min(self.h - 1) * self.w + x.min(self.w - 1)];
            let a = p >> 24;
            [((p >> 16) & 0xff) * a, ((p >> 8) & 0xff) * a, (p & 0xff) * a, a]
        };
        for y in 0..nh {
            let sy = (y as u32 * fy + fy / 2).saturating_sub(128);
            let (y0, ty) = ((sy >> 8) as usize, sy & 0xff);
            for x in 0..nw {
                let sx = (x as u32 * fx + fx / 2).saturating_sub(128);
                let (x0, tx) = ((sx >> 8) as usize, sx & 0xff);
                let (p00, p10, p01, p11) = (get(x0, y0), get(x0 + 1, y0), get(x0, y0 + 1), get(x0 + 1, y0 + 1));
                let mut c = [0u32; 4];
                for i in 0..4 {
                    let top = p00[i] * (256 - tx) + p10[i] * tx;
                    let bot = p01[i] * (256 - tx) + p11[i] * tx;
                    c[i] = (top / 256 * (256 - ty) + bot / 256 * ty) / 256;
                }
                let a = c[3];
                px[y * nw + x] = if a == 0 {
                    0
                } else {
                    a << 24 | (c[0] / a).min(255) << 16 | (c[1] / a).min(255) << 8 | (c[2] / a).min(255)
                };
            }
        }
        Image { w: nw, h: nh, px }
    }
}

impl Canvas {
    pub fn new(w: usize, h: usize) -> Canvas {
        Canvas { w, h, px: vec![0; w * h] }
    }

    /// Copies an opaque image of the same size (its alpha byte must already be 0, see `Image::opaque`).
    pub fn copy_from(&mut self, img: &Image) {
        let n = self.px.len();
        self.px.copy_from_slice(&img.px[..n]);
    }

    /// Draws `img` (or a sub-rectangle of it) at (x, y). `alpha` 0..=256 scales its opacity,
    /// `tint` (0x00RRGGBB) replaces its colour when set (used for text).
    pub fn draw_sub(&mut self, img: &Image, sx: usize, sy: usize, sw: usize, sh: usize, x: i32, y: i32, alpha: u32, tint: Option<u32>) {
        for row in 0..sh as i32 {
            let dy = y + row;
            if dy < 0 || dy >= self.h as i32 {
                continue;
            }
            let src_row = (sy + row as usize) * img.w + sx;
            let dst_row = dy as usize * self.w;
            for col in 0..sw as i32 {
                let dx = x + col;
                if dx < 0 || dx >= self.w as i32 {
                    continue;
                }
                let s = img.px[src_row + col as usize];
                let a = ((s >> 24) * alpha) >> 8;
                if a == 0 {
                    continue;
                }
                let d = &mut self.px[dst_row + dx as usize];
                *d = blend_px(*d, tint.unwrap_or(s), a + (a >> 7));
            }
        }
    }

    pub fn draw(&mut self, img: &Image, x: i32, y: i32, alpha: u32) {
        self.draw_sub(img, 0, 0, img.w, img.h, x, y, alpha, None);
    }

    pub fn draw_centered(&mut self, img: &Image, cx: i32, cy: i32, alpha: u32) {
        self.draw(img, cx - img.w as i32 / 2, cy - img.h as i32 / 2, alpha);
    }

    /// Additive blend of an alpha mask (used for soft star sprites).
    pub fn add_mask(&mut self, mask: &[u8], mw: usize, x: i32, y: i32, rgb: u32, alpha: u32) {
        let mh = mask.len() / mw;
        for row in 0..mh as i32 {
            let dy = y + row;
            if dy < 0 || dy >= self.h as i32 {
                continue;
            }
            for col in 0..mw as i32 {
                let dx = x + col;
                if dx < 0 || dx >= self.w as i32 {
                    continue;
                }
                let m = mask[row as usize * mw + col as usize] as u32;
                if m == 0 {
                    continue;
                }
                let i = dy as usize * self.w + dx as usize;
                self.px[i] = add_px(self.px[i], rgb, (m * alpha) >> 8);
            }
        }
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgb: u32, alpha: u32) {
        let (x0, y0) = (x.max(0), y.max(0));
        let (x1, y1) = ((x + w).min(self.w as i32), (y + h).min(self.h as i32));
        for yy in y0..y1 {
            let row = yy as usize * self.w;
            for xx in x0..x1 {
                let d = &mut self.px[row + xx as usize];
                *d = blend_px(*d, rgb, alpha);
            }
        }
    }

    /// Rectangle with the top-right and bottom-left corners cut at 45 degrees (the angled frame used
    /// by the icon selection). Paints a `border`-pixel outline and an optional translucent fill.
    pub fn cut_frame(&mut self, x: i32, y: i32, w: i32, h: i32, cut: i32, border: i32, line: u32, line_a: u32, fill: u32, fill_a: u32) {
        let span = |r: i32, inset: i32| -> Option<(i32, i32)> {
            // horizontal extent of the cut-corner shape shrunk by `inset`, at row r (relative to y)
            let (ry, hh, c) = (r - inset, h - 2 * inset, (cut - inset / 2).max(0));
            if ry < 0 || ry >= hh {
                return None;
            }
            let mut l = x + inset;
            let mut rr = x + w - inset;
            if ry < c {
                rr -= c - ry;
            }
            if ry >= hh - c {
                l += ry - (hh - c);
            }
            Some((l, rr))
        };
        for r in 0..h {
            let yy = y + r;
            if yy < 0 || yy >= self.h as i32 {
                continue;
            }
            let Some((l0, r0)) = span(r, 0) else { continue };
            let row = yy as usize * self.w;
            let paint = |a: i32, b: i32, c: u32, al: u32, px: &mut Vec<u32>| {
                for xx in a.max(0)..b.min(self.w as i32) {
                    let d = &mut px[row + xx as usize];
                    *d = blend_px(*d, c, al);
                }
            };
            match span(r, border) {
                Some((l1, r1)) => {
                    // diagonal edges need a thicker horizontal run to look as wide as the straight ones
                    paint(l0, l1.max(l0 + border), line, line_a, &mut self.px);
                    paint(r1.min(r0 - border), r0, line, line_a, &mut self.px);
                    if fill_a > 0 {
                        paint(l1.max(l0 + border), r1.min(r0 - border), fill, fill_a, &mut self.px);
                    }
                }
                None => paint(l0, r0, line, line_a, &mut self.px),
            }
        }
    }

    /// Soft outer glow around a cut-corner frame.
    pub fn cut_glow(&mut self, x: i32, y: i32, w: i32, h: i32, cut: i32, rgb: u32, strength: u32, radius: i32) {
        for i in 1..=radius {
            let a = strength * (radius - i + 1) as u32 / (radius as u32 * 3);
            self.cut_frame(x - i, y - i, w + 2 * i, h + 2 * i, cut + i / 2, 1, rgb, a, 0, 0);
        }
    }
}

/// Left padding (in atlas pixels) before each glyph's pen origin: 4 px x export density 2.
const FONT_PAD: f32 = 8.0;

/// Proportional bitmap font: a strip of 95 equal cells (ASCII 32..126) with per-glyph advances.
pub struct Font {
    pad: f32, // pen origin inside each cell (FONT_PAD x scale), see tools/export-assets.ps1
    pub cell_w: usize,
    pub cell_h: usize,
    adv: [f32; 95], // fractional advances, so scaled text keeps even letter spacing
    atlas: Image,
}

impl Font {
    pub fn decode(blob: &[u8], scale: f32) -> Font {
        assert!(&blob[0..4] == b"FNT1", "bad font blob");
        let rd = |o: usize| u32::from_le_bytes(blob[o..o + 4].try_into().unwrap()) as usize;
        let (cw, ch, n) = (rd(4), rd(8), rd(12));
        let mut adv = [0f32; 95];
        for i in 0..n.min(95) {
            adv[i] = blob[16 + i] as f32;
        }
        let alpha = miniz_oxide::inflate::decompress_to_vec_zlib(&blob[16 + n..]).expect("inflate font");
        let atlas = Image { w: cw * n, h: ch, px: alpha.iter().map(|a| (*a as u32) << 24 | 0x00ff_ffff).collect() };
        let mut f = Font { cell_w: cw, cell_h: ch, adv, atlas, pad: FONT_PAD };
        if scale < 0.99 || scale > 1.01 {
            let ncw = ((cw as f32 * scale) as usize).max(1);
            let nch = ((ch as f32 * scale) as usize).max(1);
            f.atlas = f.atlas.resized(ncw * n, nch);
            f.cell_w = ncw;
            f.cell_h = nch;
            for a in f.adv.iter_mut() {
                *a *= scale;
            }
            f.pad *= scale;
        }
        f
    }

    fn idx(c: char) -> usize {
        let c = c as u32;
        if (32..127).contains(&c) { (c - 32) as usize } else { ('?' as u32 - 32) as usize }
    }

    /// Width in pixels; `spacing` adds extra letter-spacing.
    /// Horizontal step after a character: its advance plus letter-spacing. With letter-spacing on, spaces
    /// get extra width too, or spaced-out words run together ("SETTINGS(BIOS)").
    fn step(&self, c: char, spacing: i32) -> f32 {
        let extra = if c == ' ' { spacing as f32 * 2.0 } else { 0.0 };
        self.adv[Self::idx(c)] + spacing as f32 + extra
    }

    pub fn width(&self, text: &str, spacing: i32) -> i32 {
        (text.chars().map(|c| self.step(c, spacing)).sum::<f32>() + 0.5) as i32 - spacing.max(0)
    }

    pub fn draw(&self, cv: &mut Canvas, text: &str, x: i32, y: i32, rgb: u32, alpha: u32, spacing: i32) {
        let mut cx = x as f32;
        for c in text.chars() {
            let i = Self::idx(c);
            cv.draw_sub(&self.atlas, i * self.cell_w, 0, self.cell_w, self.cell_h, (cx - self.pad + 0.5) as i32, y, alpha, Some(rgb));
            cx += self.step(c, spacing);
        }
    }

    pub fn draw_centered(&self, cv: &mut Canvas, text: &str, cx: i32, y: i32, rgb: u32, alpha: u32, spacing: i32) {
        self.draw(cv, text, cx - self.width(text, spacing) / 2, y, rgb, alpha, spacing);
    }
}
