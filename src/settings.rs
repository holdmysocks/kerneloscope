//! Settings changed from the in-menu settings page, stored in firmware NVRAM (UEFI variables under
//! Kerneloscope's own vendor GUID). They override menu.conf, so the file never has to be rewritten by
//! the boot menu itself.
//!
//!   KerneloscopeSettings  "default=<entry title | *last*>\ntimeout=<s>\nintro=<0|1>\n"
//!   KerneloscopeLastBoot  title of the entry booted last (used when default = *last*)

use alloc::string::{String, ToString};
use uefi::runtime::{self, VariableAttributes, VariableVendor};
use uefi::{cstr16, guid};

const VENDOR: VariableVendor = VariableVendor(guid!("6d1f5c0a-8e3b-4b7e-9c2a-4f1e7d3b2a90"));
/// `default` value meaning "whatever was booted last"
pub const LAST: &str = "*last*";

#[derive(Clone, Default)]
pub struct Saved {
    pub default: Option<String>,
    pub timeout: Option<u32>,
    pub intro: Option<bool>,
}

fn attrs() -> VariableAttributes {
    // runtime access too, so the variables can be inspected/removed from Linux (efivarfs) if ever needed
    VariableAttributes::NON_VOLATILE | VariableAttributes::BOOTSERVICE_ACCESS | VariableAttributes::RUNTIME_ACCESS
}

fn read(name: &uefi::CStr16) -> Option<String> {
    let (data, _) = runtime::get_variable_boxed(name, &VENDOR).ok()?;
    String::from_utf8(data.to_vec()).ok()
}

pub fn load() -> Saved {
    let mut s = Saved::default();
    let Some(text) = read(cstr16!("KerneloscopeSettings")) else { return s };
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        match k {
            "default" if !v.is_empty() => s.default = Some(v.to_string()),
            "timeout" => s.timeout = v.parse().ok(),
            "intro" => s.intro = Some(v == "1"),
            _ => {}
        }
    }
    s
}

pub fn save(s: &Saved) -> bool {
    let mut text = String::new();
    if let Some(d) = &s.default {
        text += &alloc::format!("default={}\n", d);
    }
    if let Some(t) = s.timeout {
        text += &alloc::format!("timeout={}\n", t);
    }
    if let Some(i) = s.intro {
        text += if i { "intro=1\n" } else { "intro=0\n" };
    }
    runtime::set_variable(cstr16!("KerneloscopeSettings"), &VENDOR, attrs(), text.as_bytes()).is_ok()
}

pub fn last() -> Option<String> {
    read(cstr16!("KerneloscopeLastBoot"))
}

/// Remembers the booted entry; writes only when it changed (NVRAM is written as rarely as possible).
pub fn set_last(title: &str) {
    if last().as_deref() != Some(title) {
        let _ = runtime::set_variable(cstr16!("KerneloscopeLastBoot"), &VENDOR, attrs(), title.as_bytes());
    }
}
