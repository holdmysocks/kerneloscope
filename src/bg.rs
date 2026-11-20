//! Background generated at the screen's exact size and shape (no stretching, no cropping):
//!   - dark vignette whose ellipse follows the screen's aspect ratio (16:10, 16:9, 21:9, 32:9, 5:4 ...)
//!   - faint red glow behind the OS row, sized with the layout scale
//!   - corner decorations pinned to the top-left / bottom-right corners
//! Integer maths + lookup tables only (UEFI code runs without the FPU), with ordered dithering so the
//! long, dark gradients don't band on big screens.

use crate::gfx::{Canvas, Image};
use alloc::vec;
use alloc::vec::Vec;

const LUT: usize = 4096; // entries over t^2 in [0, 4)
const Q: u32 = 16; // fixed-point bits of the t^2 terms

fn sqrt(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut g = if x > 1.0 { x / 2.0 } else { 1.0 };
    for _ in 0..12 {
        g = 0.5 * (g + x / g);
    }
    g
}

fn rgb(c: u32) -> [f32; 3] {
    [((c >> 16) & 0xff) as f32, ((c >> 8) & 0xff) as f32, (c & 0xff) as f32]
}

/// Squared normalised distance terms, Q16 fixed point: ((i - centre) / radius)^2
fn terms(n: usize, centre: f32, radius: f32) -> Vec<u32> {
    (0..n)
        .map(|i| {
            let d = (i as f32 + 0.5 - centre) / radius;
            ((d * d).min(3.99) * (1 << Q) as f32) as u32
        })
        .collect()
}

/// `oy`: top of the 2560x1600 layout band, `s`: layout scale (same values the menu uses).
pub fn generate(w: usize, h: usize, s: f32, oy: i32, corner_tl: &Image, corner_br: &Image) -> Image {
    // colour ramps in 8.8 fixed point, indexed by t^2
    let stops = [(0.0, rgb(0x121216)), (0.6, rgb(0x08080a)), (1.0, rgb(0x040405))];
    let red = rgb(0xff1f3d);
    let mut base = vec![[0u32; 3]; LUT];
    let mut glow = vec![0u32; LUT]; // red glow alpha, 0..=65536
    for i in 0..LUT {
        let t = sqrt(i as f32 * 4.0 / LUT as f32);
        let c = if t >= 1.0 {
            stops[2].1
        } else {
            let (a, b) = if t < 0.6 { (stops[0], stops[1]) } else { (stops[1], stops[2]) };
            let k = (t - a.0) / (b.0 - a.0);
            [a.1[0] + (b.1[0] - a.1[0]) * k, a.1[1] + (b.1[1] - a.1[1]) * k, a.1[2] + (b.1[2] - a.1[2]) * k]
        };
        base[i] = [(c[0] * 256.0) as u32, (c[1] * 256.0) as u32, (c[2] * 256.0) as u32];
        glow[i] = ((0.10 * (1.0 - t / 0.7)).max(0.0) * 65536.0) as u32;
    }
    let red88 = [(red[0] * 256.0) as u32, (red[1] * 256.0) as u32, (red[2] * 256.0) as u32];

    // vignette follows the screen shape; glow is a fixed-size ellipse behind the OS row
    let (bx, by) = (terms(w, w as f32 / 2.0, 2200.0 / 2560.0 * w as f32), terms(h, h as f32 / 2.0, 1400.0 / 1600.0 * h as f32));
    let glow_cy = oy as f32 + 864.0 * s;
    let (gx, gy) = (terms(w, w as f32 / 2.0, 1400.0 * s), terms(h, glow_cy, 700.0 * s));
    const BAYER: [[u32; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    let shift = Q - 10; // Q16 t^2 (0..4) -> LUT index (0..4096)

    let mut cv = Canvas::new(w, h);
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            let bi = (((bx[x] + by[y]) >> shift) as usize).min(LUT - 1);
            let gi = (((gx[x] + gy[y]) >> shift) as usize).min(LUT - 1);
            let (c, a) = (base[bi], glow[gi]);
            let d = BAYER[y & 3][x & 3] * 16; // ordered dither, +0..1 LSB
            let ch = |i: usize| -> u32 {
                let v = ((c[i] as u64 * (65536 - a) as u64 + red88[i] as u64 * a as u64) >> 16) as u32;
                ((v + d) >> 8).min(255)
            };
            cv.px[row + x] = ch(0) << 16 | ch(1) << 8 | ch(2);
        }
    }
    // corner decorations stay in the corners whatever the aspect ratio
    cv.draw(corner_tl, 0, 0, 256);
    cv.draw(corner_br, w as i32 - corner_br.w as i32, h as i32 - corner_br.h as i32, 256);
    Image { w, h, px: cv.px }
}
