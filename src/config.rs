//! Parser for menu.conf (see the comments at the top of that file for the format).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[derive(Clone, Default)]
pub struct Choice {
    pub title: String,
    pub loader: String,
    pub options: String,
}

#[derive(Clone, Default)]
pub struct Entry {
    pub title: String,
    pub icon: String,
    pub main: Choice,
    pub subs: Vec<Choice>,
}

pub struct Config {
    pub timeout: u32,
    pub intro: bool,
    pub resolution: String,
    pub debug: bool,
    pub default: String,
    pub entries: Vec<Entry>,
}

/// Splits a line into words, honouring "double quotes".
fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut any) = (false, false);
    for c in line.chars() {
        match c {
            '"' => { quoted = !quoted; any = true; }
            c if c.is_whitespace() && !quoted => {
                if any { out.push(core::mem::take(&mut cur)); any = false; }
            }
            c => { cur.push(c); any = true; }
        }
    }
    if any { out.push(cur); }
    out
}

pub fn parse(text: &str) -> Config {
    let mut cfg = Config { timeout: 8, intro: true, resolution: String::from("native"), debug: false, default: String::new(), entries: Vec::new() };
    let mut in_sub = false;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() { continue; }
        let w = words(line);
        let key = w[0].to_ascii_lowercase();
        let arg = w.get(1).cloned().unwrap_or_default();
        match key.as_str() {
            "timeout" => cfg.timeout = arg.parse().unwrap_or(8),
            "debug" => cfg.debug = matches!(arg.to_ascii_lowercase().as_str(), "on" | "true" | "1" | "yes"),
            "resolution" => cfg.resolution = arg.to_ascii_lowercase(),
            "intro" => cfg.intro = !matches!(arg.to_ascii_lowercase().as_str(), "off" | "false" | "0" | "no"),
            "default" => cfg.default = arg,
            "entry" => {
                cfg.entries.push(Entry { title: arg.clone(), icon: "linux".to_string(), main: Choice { title: arg, ..Default::default() }, subs: Vec::new() });
                in_sub = false;
            }
            "sub" => {
                if let Some(e) = cfg.entries.last_mut() {
                    // a sub starts as a copy of the entry's own loader/options; following lines override
                    e.subs.push(Choice { title: arg, loader: e.main.loader.clone(), options: e.main.options.clone() });
                    in_sub = true;
                }
            }
            "icon" | "loader" | "options" => {
                let Some(e) = cfg.entries.last_mut() else { continue };
                if key == "icon" { e.icon = arg; continue; }
                let target = if in_sub { e.subs.last_mut().unwrap() } else { &mut e.main };
                if key == "loader" { target.loader = arg; } else { target.options = arg; }
                // later subs inherit from the entry, but subs declared before an entry-level line keep theirs
            }
            _ => {}
        }
    }
    // subs that never set a loader inherit the entry's (handles `loader` written after `sub` lines)
    for e in cfg.entries.iter_mut() {
        for s in e.subs.iter_mut() {
            if s.loader.is_empty() { s.loader = e.main.loader.clone(); }
        }
    }
    cfg
}
