//! Slow, drifting, softly twinkling particles in the spirit of a console start-up screen, plus a
//! start-up "burst": particles launch from a central flash, swirl outwards and settle into the drift.
//!
//! Smoothness: every sprite is pre-rendered at 4x4 sub-pixel offsets, so slow-moving particles glide
//! instead of stepping a whole pixel at a time.

use crate::gfx::Canvas;
use alloc::vec::Vec;

/// Length of the start-up animation in seconds. The menu fades in during the last ~1.6 s.
pub const INTRO_SECONDS: f32 = 4.6;

const SUB: usize = 4; // sub-pixel steps per axis

struct Star {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    sprite: usize,
    base: f32,  // base brightness 0..1
    phase: f32, // twinkle phase
    rate: f32,  // twinkle speed (cycles / second)
    rgb: u32,
    // start-up burst: launches from the centre after `delay`, takes `fly` seconds to reach its place,
    // swirling `spin` radians on the way
    delay: f32,
    fly: f32,
    spin: f32,
}

/// One soft round sprite, pre-shifted to SUB x SUB sub-pixel positions.
struct Sprite {
    size: usize,
    half: i32,
    masks: Vec<Vec<u8>>, // index: sy * SUB + sx
}

pub struct Starfield {
    stars: Vec<Star>,
    sprites: Vec<Sprite>,
    glow: Sprite, // big soft flash for the burst (single offset)
    core: Sprite,
    w: f32,
    h: f32,
}

const PI: f32 = 3.141_592_7;

/// sin without libm (Bhaskara I approximation, error < 0.2%), any input range.
fn sin(x: f32) -> f32 {
    let tau = 2.0 * PI;
    let mut x = x - ((x / tau) as i32) as f32 * tau;
    if x < 0.0 { x += tau; }
    let (x, sign) = if x > PI { (x - PI, -1.0) } else { (x, 1.0) };
    sign * 16.0 * x * (PI - x) / (5.0 * PI * PI - 4.0 * x * (PI - x))
}
fn cos(x: f32) -> f32 {
    sin(x + PI / 2.0)
}
fn floor(x: f32) -> f32 {
    let t = (x as i32) as f32;
    if t > x { t - 1.0 } else { t }
}
fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0) // smootherstep
}
/// ease-out quint: fast launch, long gentle landing
fn ease_out(t: f32) -> f32 {
    let u = 1.0 - t.clamp(0.0, 1.0);
    1.0 - u * u * u * u * u
}

struct Rng(u32);
impl Rng {
    fn next(&mut self) -> f32 {
        // xorshift32 -> 0..1
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / (1u32 << 24) as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.next()
    }
}

/// Soft round sprite (radius in px) with falloff `softness`; offset (ox, oy) in pixels (0..1).
fn mask(radius: f32, softness: f32, ox: f32, oy: f32, size: usize) -> Vec<u8> {
    let c = (size as f32 - 1.0) / 2.0;
    let r2 = (radius + 0.5) * (radius + 0.5);
    let mut m = Vec::with_capacity(size * size);
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 - c - ox + 0.5, y as f32 - c - oy + 0.5);
            let t = 1.0 - (dx * dx + dy * dy) / r2; // 1 at centre, 0 at edge (no sqrt needed)
            let v = if t <= 0.0 { 0.0 } else { t * t * (1.0 + softness * (t - 1.0)) };
            m.push((v.clamp(0.0, 1.0) * 255.0) as u8);
        }
    }
    m
}

fn sprite(radius: f32, softness: f32, subpixel: bool) -> Sprite {
    let size = (radius.max(0.5) as usize) * 2 + 3;
    let n = if subpixel { SUB } else { 1 };
    let mut masks = Vec::with_capacity(n * n);
    for sy in 0..n {
        for sx in 0..n {
            masks.push(mask(radius, softness, sx as f32 / SUB as f32, sy as f32 / SUB as f32, size));
        }
    }
    Sprite { size, half: (size / 2) as i32, masks }
}

impl Sprite {
    /// Additive draw centred on (x, y) with sub-pixel placement.
    fn draw(&self, cv: &mut Canvas, x: f32, y: f32, rgb: u32, alpha: u32) {
        if alpha == 0 {
            return;
        }
        let (fx, fy) = (floor(x), floor(y));
        let idx = if self.masks.len() == 1 {
            0
        } else {
            let sx = (((x - fx) * SUB as f32) as usize).min(SUB - 1);
            let sy = (((y - fy) * SUB as f32) as usize).min(SUB - 1);
            sy * SUB + sx
        };
        cv.add_mask(&self.masks[idx], self.size, fx as i32 - self.half, fy as i32 - self.half, rgb, alpha);
    }
}

impl Starfield {
    pub fn new(w: usize, h: usize, scale: f32, seed: u32) -> Starfield {
        let mut rng = Rng(seed | 1);
        let r = |v: f32| (v * scale).max(0.6);
        // sprite table: 0..2 small sharp, 3..4 medium, 5..6 large blurred bokeh
        let sprites = alloc::vec![
            sprite(r(1.0), 0.0, true), sprite(r(1.5), 0.0, true), sprite(r(2.2), 0.2, true),
            sprite(r(3.5), 0.4, true), sprite(r(5.0), 0.6, true),
            sprite(r(14.0), 0.95, false), sprite(r(24.0), 1.0, false),
        ];
        let area = (w * h) as f32 / (2560.0 * 1600.0);
        let mut stars = Vec::new();
        let mut add = |n: usize, s0: usize, s1: usize, speed: f32, b0: f32, b1: f32, rng: &mut Rng| {
            for _ in 0..((n as f32 * area) as usize).max(4) {
                let sp = speed * scale * rng.range(0.6, 1.4);
                let tint = rng.next();
                stars.push(Star {
                    x: rng.range(0.0, w as f32),
                    y: rng.range(0.0, h as f32),
                    // gentle diagonal drift up and to the right, slightly varied per star
                    vx: sp * rng.range(0.55, 0.85),
                    vy: -sp * rng.range(0.35, 0.6),
                    sprite: s0 + (rng.next() * (s1 - s0 + 1) as f32) as usize % (s1 - s0 + 1),
                    base: rng.range(b0, b1),
                    phase: rng.next(),
                    rate: rng.range(0.08, 0.35),
                    rgb: if tint < 0.12 { 0x00ff_6b7d } else if tint < 0.22 { 0x0090_b8ff } else { 0x00ff_ffff },
                    delay: rng.range(0.15, 1.1),
                    fly: rng.range(2.0, 3.1),
                    spin: rng.range(0.9, 1.7) * if rng.next() < 0.5 { -1.0 } else { 1.0 },
                });
            }
        };
        add(170, 0, 2, 6.0, 0.25, 0.75, &mut rng); // far: tiny, slow
        add(55, 2, 4, 14.0, 0.25, 0.6, &mut rng); // mid
        add(12, 5, 6, 24.0, 0.05, 0.14, &mut rng); // near: big, faint, blurred
        let glow = sprite(r(340.0), 1.0, false);
        let core = sprite(r(80.0), 1.0, false);
        Starfield { stars, sprites, glow, core, w: w as f32, h: h as f32 }
    }

    pub fn update(&mut self, dt: f32) {
        let m = 40.0; // wrap margin so big motes don't pop at the edges
        for s in self.stars.iter_mut() {
            s.x += s.vx * dt;
            s.y += s.vy * dt;
            s.phase += s.rate * dt;
            if s.x > self.w + m { s.x -= self.w + 2.0 * m; }
            if s.y < -m { s.y += self.h + 2.0 * m; }
        }
    }

    /// `intro`: seconds since start while the start-up burst is playing, None afterwards.
    pub fn draw(&self, cv: &mut Canvas, intro: Option<f32>) {
        let (cx, cy) = (self.w / 2.0, self.h / 2.0);
        if let Some(t) = intro {
            // central flash: soft swell, long smooth fade
            let f = if t < 0.6 { smooth(t / 0.6) } else { 1.0 - smooth((t - 0.6) / 1.9) };
            if f > 0.0 {
                self.glow.draw(cv, cx, cy, 0x00ff_2a48, (f * 140.0) as u32);
                self.core.draw(cv, cx, cy, 0x00ff_e8ec, (f * f * 220.0) as u32);
            }
        }
        for s in &self.stars {
            let sp = &self.sprites[s.sprite];
            let tw = 0.55 + 0.45 * smooth(wave(s.phase));
            let base = s.base * tw;
            let Some(t) = intro else {
                sp.draw(cv, s.x, s.y, s.rgb, (base * 256.0) as u32);
                continue;
            };
            let p = (t - s.delay) / s.fly;
            if p <= 0.0 {
                continue;
            }
            let p = p.min(1.0);
            let appear = smooth(p / 0.12); // fade in at launch instead of popping
            // head plus a continuous streak of fading samples while moving fast
            let samples = if p >= 0.98 { 1 } else { 7 };
            for k in 0..samples {
                let pk = p - k as f32 * 0.012;
                if pk <= 0.0 {
                    break;
                }
                let e = ease_out(pk);
                let ang = (1.0 - e) * (1.0 - e) * s.spin;
                let (dx, dy) = (s.x - cx, s.y - cy);
                let (c, sn) = (cos(ang), sin(ang));
                let x = cx + (dx * c - dy * sn) * e;
                let y = cy + (dx * sn + dy * c) * e;
                let boost = 1.0 + 1.6 * (1.0 - e) * (1.0 - e); // brighter while flying fast
                let fade = 1.0 - k as f32 / samples as f32;
                let a = ((base * boost * fade * fade * appear).min(1.0) * 256.0) as u32;
                sp.draw(cv, x, y, s.rgb, a);
            }
        }
    }
}

/// 0..1 periodic triangle wave.
fn wave(t: f32) -> f32 {
    let f = t - floor(t);
    if f < 0.5 { f * 2.0 } else { 2.0 - f * 2.0 }
}
