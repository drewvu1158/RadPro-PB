//! Unicode Windows file dialogs; cancellation is distinct from a failure.
use std::path::PathBuf;

pub fn choose(save: bool, artwork: bool) -> Result<Option<PathBuf>, String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        use windows_sys::Win32::UI::Controls::Dialogs::*;
        let filter: Vec<u16> = if artwork {
            "Images (PNG, JPEG)\0*.png;*.jpg;*.jpeg\0\0"
        } else {
            "Photobooth layout (*.sbl)\0*.sbl\0\0"
        }
        .encode_utf16()
        .collect();
        let title: Vec<u16> = if save {
            "Export layout\0"
        } else {
            "Import layout or artwork\0"
        }
        .encode_utf16()
        .collect();
        let ext: Vec<u16> = "sbl\0".encode_utf16().collect();
        let mut filename = vec![0u16; 32768];
        let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
        dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        dialog.lpstrFilter = filter.as_ptr();
        dialog.lpstrFile = filename.as_mut_ptr();
        dialog.nMaxFile = filename.len() as u32;
        dialog.lpstrTitle = title.as_ptr();
        dialog.lpstrDefExt = if artwork {
            std::ptr::null()
        } else {
            ext.as_ptr()
        };
        dialog.Flags = OFN_EXPLORER
            | OFN_NOCHANGEDIR
            | OFN_PATHMUSTEXIST
            | if save {
                OFN_OVERWRITEPROMPT
            } else {
                OFN_FILEMUSTEXIST
            };
        // All pointer buffers outlive the modal call; the dialog owns none.
        let ok = unsafe {
            if save {
                GetSaveFileNameW(&mut dialog)
            } else {
                GetOpenFileNameW(&mut dialog)
            }
        };
        if ok == 0 {
            let error = unsafe { CommDlgExtendedError() };
            return if error == 0 {
                Ok(None)
            } else {
                Err(format!("File dialog failed (0x{error:x})"))
            };
        }
        let end = filename
            .iter()
            .position(|c| *c == 0)
            .ok_or("Invalid file dialog path")?;
        Ok(Some(PathBuf::from(std::ffi::OsString::from_wide(
            &filename[..end],
        ))))
    }
    #[cfg(not(windows))]
    {
        let _ = (save, artwork);
        Err("Native file selection currently requires Windows".into())
    }
}
