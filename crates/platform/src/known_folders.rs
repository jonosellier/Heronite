//! Known Windows folders used by system-wide scanners.

use std::io;
use std::path::PathBuf;

use windows::Win32::System::Com::{CoTaskMemFree, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
use windows::Win32::UI::Shell::{FOLDERID_CommonPrograms, FOLDERID_Programs, SHGetKnownFolderPath, KNOWN_FOLDER_FLAG};

use crate::com::Com;

/// The current user's and all users' Start Menu Programs folders.
pub fn programs() -> io::Result<Vec<PathBuf>> {
    let _com = Com::init(COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    [&FOLDERID_CommonPrograms, &FOLDERID_Programs]
        .into_iter()
        .map(|folder| {
            // SAFETY: the returned shell-allocated string is freed after conversion.
            let path = unsafe { SHGetKnownFolderPath(folder, KNOWN_FOLDER_FLAG(0), None) }.map_err(io::Error::other)?;
            let value = unsafe { path.to_string() }.map_err(io::Error::other);
            unsafe { CoTaskMemFree(Some(path.0.cast())) };
            value.map(PathBuf::from)
        })
        .collect()
}
