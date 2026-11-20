//! Reads the display's preferred (native) resolution from its EDID, via the firmware's
//! EFI_EDID_ACTIVE / EFI_EDID_DISCOVERED protocols on the graphics output handle.

use uefi::boot::{self, OpenProtocolAttributes, OpenProtocolParams};
use uefi::proto::console::gop::GraphicsOutput;
use uefi::proto::unsafe_protocol;
use uefi::Handle;

#[repr(C)]
struct EdidRaw {
    size: u32,
    edid: *const u8,
}
#[repr(transparent)]
#[unsafe_protocol("bd8c1056-9f36-44ec-92a8-a6337f817986")]
struct EdidActive(EdidRaw);
#[repr(transparent)]
#[unsafe_protocol("1c0c34f6-d380-41fa-a049-8ad06c1a66aa")]
struct EdidDiscovered(EdidRaw);

fn get<P: uefi::proto::ProtocolPointer + ?Sized>(h: Handle) -> Option<boot::ScopedProtocol<P>> {
    unsafe {
        boot::open_protocol::<P>(
            OpenProtocolParams { handle: h, agent: boot::image_handle(), controller: None },
            OpenProtocolAttributes::GetProtocol,
        )
        .ok()
    }
}

/// Preferred timing = first detailed timing descriptor (bytes 54..72 of the 128-byte base block).
fn parse(raw: &EdidRaw) -> Option<(usize, usize)> {
    if raw.edid.is_null() || raw.size < 128 {
        return None;
    }
    let e = unsafe { core::slice::from_raw_parts(raw.edid, 128) };
    if e[0..8] != [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00] {
        return None;
    }
    let d = &e[54..72];
    if d[0] == 0 && d[1] == 0 {
        return None; // not a timing descriptor
    }
    let w = d[2] as usize | ((d[4] as usize >> 4) << 8);
    let h = d[5] as usize | ((d[7] as usize >> 4) << 8);
    (w >= 320 && h >= 200).then_some((w, h))
}

/// Native resolution of the display driven by the graphics output, if the firmware exposes its EDID.
pub fn native_resolution() -> Option<(usize, usize)> {
    let h = boot::get_handle_for_protocol::<GraphicsOutput>().ok()?;
    if let Some(a) = get::<EdidActive>(h) {
        if let Some(r) = parse(&a.0) {
            return Some(r);
        }
    }
    get::<EdidDiscovered>(h).and_then(|d| parse(&d.0))
}
