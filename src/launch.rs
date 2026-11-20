//! Finding loaders on any EFI partition and chain-loading them.

use alloc::string::String;
use alloc::vec::Vec;
use uefi::boot::{self, LoadImageSource, OpenProtocolAttributes, OpenProtocolParams, ScopedProtocol, SearchType};
use uefi::fs::{FileSystem, Path};
use uefi::proto::device_path::{build, DevicePath};
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::proto::BootPolicy;
use uefi::{CString16, Handle, Identify};

/// Opens a protocol without claiming it exclusively (so firmware drivers and the next OS can still use it).
fn get_proto<P: uefi::proto::ProtocolPointer + ?Sized>(h: Handle) -> uefi::Result<ScopedProtocol<P>> {
    unsafe {
        boot::open_protocol::<P>(
            OpenProtocolParams { handle: h, agent: boot::image_handle(), controller: None },
            OpenProtocolAttributes::GetProtocol,
        )
    }
}

fn to_c16(s: &str) -> Option<CString16> {
    CString16::try_from(s).ok()
}

/// Reads a file from the partition this program was started from.
pub fn read_own_file(path: &str) -> Option<Vec<u8>> {
    let li = get_proto::<LoadedImage>(boot::image_handle()).ok()?;
    let dev = li.device()?;
    drop(li);
    let mut fs = FileSystem::new(get_proto::<SimpleFileSystem>(dev).ok()?);
    let p = to_c16(path)?;
    fs.read(Path::new(&p)).ok()
}

/// Returns the first filesystem handle that contains `path`. The partition we booted from is checked first.
pub fn find(path: &str) -> Option<Handle> {
    let p = to_c16(path)?;
    let mut handles: Vec<Handle> = boot::locate_handle_buffer(SearchType::ByProtocol(&SimpleFileSystem::GUID)).ok()?.to_vec();
    if let Some(own) = get_proto::<LoadedImage>(boot::image_handle()).ok().and_then(|li| li.device()) {
        if let Some(i) = handles.iter().position(|h| *h == own) {
            handles.swap(0, i);
        }
    }
    handles.into_iter().find(|&h| {
        get_proto::<SimpleFileSystem>(h)
            .map(|sfs| FileSystem::new(sfs).try_exists(Path::new(&p)).unwrap_or(false))
            .unwrap_or(false)
    })
}

/// Loads and starts `path` on volume `h` with an optional command line. Returns only if the loader
/// fails to start or exits back to us (e.g. the user quits rEFInd).
pub fn start(h: Handle, path: &str, options: &str) -> Result<(), String> {
    let file = to_c16(path).ok_or("bad path")?;
    let mut buf = Vec::new();
    let full: &DevicePath = {
        let dp = get_proto::<DevicePath>(h).map_err(|e| alloc::format!("{:?}", e.status()))?;
        let mut b = build::DevicePathBuilder::with_vec(&mut buf);
        for node in dp.node_iter() {
            b = b.push(&node).map_err(|_| "device path")?;
        }
        b = b.push(&build::media::FilePath { path_name: &file }).map_err(|_| "device path")?;
        b.finalize().map_err(|_| "device path")?
    };
    let child = boot::load_image(boot::image_handle(), LoadImageSource::FromDevicePath { device_path: full, boot_policy: BootPolicy::ExactMatch })
        .map_err(|e| alloc::format!("load failed ({:?})", e.status()))?;
    // the command line must stay alive until the child has started
    let cmd = to_c16(options).ok_or("bad options")?;
    if !options.is_empty() {
        let mut li = boot::open_protocol_exclusive::<LoadedImage>(child).map_err(|e| alloc::format!("{:?}", e.status()))?;
        let bytes = (cmd.as_slice_with_nul().len() * 2) as u32;
        unsafe { li.set_load_options(cmd.as_ptr().cast(), bytes) };
    }
    boot::start_image(child).map_err(|e| alloc::format!("exited ({:?})", e.status()))?;
    Ok(())
}
