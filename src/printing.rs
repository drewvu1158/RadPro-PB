//! Windows printing through the system dialog. Nothing is spooled until the
//! guest confirms that dialog; cancelling it is a normal outcome.

use image::DynamicImage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrintOutcome {
    Submitted { job_id: i32, copies: u16 },
    Cancelled,
}

/// The native DEVMODE copy field is a signed 16-bit integer.
pub const MAX_COPIES: u16 = i16::MAX as u16;

fn validate_copies(copies: u16) -> Result<(), String> {
    if copies == 0 || copies > MAX_COPIES {
        return Err(format!("Choose between 1 and {MAX_COPIES} copies"));
    }
    Ok(())
}

/// Open the printer/paper dialog with the app's copy count preselected. After
/// confirmation, submit that many pages in one job with driver copies set to
/// one. This also works with drivers that cannot generate copies themselves.
/// Submission is not proof of physical completion. The call blocks while the
/// dialog is open or the printer driver is working.
pub fn print_image(
    image: &DynamicImage,
    copies: u16,
    owner_hwnd: Option<isize>,
) -> Result<PrintOutcome, String> {
    validate_copies(copies)?;
    #[cfg(windows)]
    {
        windows::print_image(image, copies, owner_hwnd)
    }
    #[cfg(not(windows))]
    {
        let _ = (image, copies, owner_hwnd);
        Err("Printing is available on Windows only".into())
    }
}

/// Return a centered destination rectangle in printer device pixels. Printer
/// dots can have different horizontal/vertical DPI; preserve physical aspect.
fn fitted_rect(
    image: (u32, u32),
    paper: (i32, i32),
    dpi: (i32, i32),
) -> Result<(i32, i32, i32, i32), String> {
    if image.0 == 0 || image.1 == 0 || paper.0 <= 0 || paper.1 <= 0 || dpi.0 <= 0 || dpi.1 <= 0 {
        return Err("The image or selected printer has invalid page dimensions".into());
    }
    let scaled_width = f64::from(image.0) * f64::from(dpi.0);
    let scaled_height = f64::from(image.1) * f64::from(dpi.1);
    let scale = (f64::from(paper.0) / scaled_width).min(f64::from(paper.1) / scaled_height);
    let width = ((scaled_width * scale).floor() as i32).clamp(1, paper.0);
    let height = ((scaled_height * scale).floor() as i32).clamp(1, paper.1);
    Ok(((paper.0 - width) / 2, (paper.1 - height) / 2, width, height))
}

#[cfg(windows)]
mod windows {
    use super::{fitted_rect, validate_copies, DynamicImage, PrintOutcome};
    use image::GenericImageView;
    use std::mem::size_of;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{GlobalFree, HWND};
    use windows_sys::Win32::Graphics::Gdi::{
        DeleteDC, GetDeviceCaps, ResetDCW, SetBrushOrgEx, SetStretchBltMode, StretchDIBits,
        BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DEVMODEW, DIB_RGB_COLORS, DM_COPIES, GDI_ERROR,
        HALFTONE, HDC, HORZRES, LOGPIXELSX, LOGPIXELSY, SRCCOPY, VERTRES,
    };
    use windows_sys::Win32::Storage::Xps::{
        AbortDoc, EndDoc, EndPage, StartDocW, StartPage, DOCINFOW,
    };
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    use windows_sys::Win32::UI::Controls::Dialogs::{
        CommDlgExtendedError, PrintDlgW, PD_DISABLEPRINTTOFILE, PD_HIDEPRINTTOFILE, PD_NOPAGENUMS,
        PD_NOSELECTION, PD_RETURNDC, PRINTDLGW,
    };

    /// Own every resource returned by PrintDlgW, including cancellation paths.
    struct DialogResources(PRINTDLGW);

    impl Drop for DialogResources {
        fn drop(&mut self) {
            // SAFETY: These handles were allocated by this invocation of
            // PrintDlgW and are never exported or freed anywhere else.
            unsafe {
                if !self.0.hDC.is_null() {
                    DeleteDC(self.0.hDC);
                }
                if !self.0.hDevMode.is_null() {
                    GlobalFree(self.0.hDevMode);
                }
                if !self.0.hDevNames.is_null() {
                    GlobalFree(self.0.hDevNames);
                }
            }
        }
    }

    struct PrintJob {
        dc: HDC,
        active: bool,
    }

    impl Drop for PrintJob {
        fn drop(&mut self) {
            if self.active {
                // SAFETY: The owning DialogResources outlives this job. An
                // incomplete document is aborted on every error return.
                unsafe {
                    AbortDoc(self.dc);
                }
            }
        }
    }

    fn failed(action: &str) -> String {
        format!("{action}: {}", std::io::Error::last_os_error())
    }

    fn new_dialog(copies: u16, owner_hwnd: Option<isize>) -> DialogResources {
        DialogResources(PRINTDLGW {
            lStructSize: size_of::<PRINTDLGW>() as u32,
            hwndOwner: owner_hwnd.map_or(null_mut(), |handle| handle as HWND),
            Flags: PD_RETURNDC
                | PD_NOPAGENUMS
                | PD_NOSELECTION
                | PD_HIDEPRINTTOFILE
                | PD_DISABLEPRINTTOFILE,
            nFromPage: 1,
            nToPage: 1,
            nMinPage: 1,
            nMaxPage: 1,
            nCopies: copies,
            ..Default::default()
        })
    }

    fn single_driver_copy(mode: &mut DEVMODEW) {
        mode.dmFields |= DM_COPIES;
        // Writing the printer member of this DEVMODE union is safe; the mode
        // comes from the printer dialog, not from a display device.
        mode.Anonymous1.Anonymous1.dmCopies = 1;
    }

    fn apply_single_driver_copy(dialog: &mut DialogResources) -> Result<(), String> {
        if dialog.0.hDevMode.is_null() {
            return Err("The printer did not return page settings. Choose another printer.".into());
        }
        // SAFETY: PrintDlgW returned this owned printer DEVMODE allocation.
        // Keep it locked, including the driver-private data, until ResetDCW
        // returns. ResetDCW returns the original DC on success.
        unsafe {
            let mode = GlobalLock(dialog.0.hDevMode).cast::<DEVMODEW>();
            if mode.is_null() {
                return Err(failed("Could not read printer settings"));
            }
            single_driver_copy(&mut *mode);
            let dc = ResetDCW(dialog.0.hDC, mode);
            let result = if dc.is_null() {
                Err(failed("Could not apply the print quantity"))
            } else {
                dialog.0.hDC = dc;
                Ok(())
            };
            GlobalUnlock(dialog.0.hDevMode);
            result
        }
    }

    pub(super) fn print_image(
        image: &DynamicImage,
        copies: u16,
        owner_hwnd: Option<isize>,
    ) -> Result<PrintOutcome, String> {
        let (width, height) = image.dimensions();
        let width_i32 = i32::try_from(width).map_err(|_| "Image is too wide to print")?;
        let height_i32 = i32::try_from(height).map_err(|_| "Image is too tall to print")?;
        if width == 0 || height == 0 {
            return Err("There is no photo to print".into());
        }
        // A 32-bit uncompressed DIB has a DWORD byte length and LONG dimensions.
        // Check representation expansion before allocating the bitmap buffer.
        let byte_length = u64::from(width) * u64::from(height) * 4;
        let byte_length = u32::try_from(byte_length)
            .map_err(|_| "Image is too large for Windows bitmap printing")?;

        let mut dialog = new_dialog(copies, owner_hwnd);

        // SAFETY: dialog has the exact ABI size and initialized optional fields.
        let confirmed = unsafe { PrintDlgW(&mut dialog.0) };
        if confirmed == 0 {
            let code = unsafe { CommDlgExtendedError() };
            return if code == 0 {
                Ok(PrintOutcome::Cancelled)
            } else {
                Err(format!("Windows could not open the print dialog (code 0x{code:08X}). Check that a printer is installed."))
            };
        }
        if dialog.0.hDC.is_null() {
            return Err("Windows did not return a printer connection".into());
        }
        // With PD_USEDEVMODECOPIESANDCOLLATE unset, nCopies is the confirmed
        // quantity. Modern Windows may also put it in the DC, so explicitly
        // normalize driver copies before rendering pages to avoid N x N copies.
        let copies = dialog.0.nCopies;
        validate_copies(copies)?;
        apply_single_driver_copy(&mut dialog)?;
        let dc = dialog.0.hDC;

        // SAFETY: dc is the printer DC owned by dialog for this call's lifetime.
        let (paper, dpi) = unsafe {
            (
                (
                    GetDeviceCaps(dc, HORZRES as i32),
                    GetDeviceCaps(dc, VERTRES as i32),
                ),
                (
                    GetDeviceCaps(dc, LOGPIXELSX as i32),
                    GetDeviceCaps(dc, LOGPIXELSY as i32),
                ),
            )
        };
        let (x, y, print_width, print_height) = fitted_rect((width, height), paper, dpi)?;

        let mut bitmap = Vec::new();
        bitmap
            .try_reserve_exact(byte_length as usize)
            .map_err(|_| "Not enough memory to prepare the photo for printing")?;
        for (_, _, pixel) in image.pixels() {
            let [red, green, blue, alpha] = pixel.0;
            let white_background = |channel: u8| -> u8 {
                ((u16::from(channel) * u16::from(alpha) + 255 * u16::from(255 - alpha) + 127) / 255)
                    as u8
            };
            bitmap.extend_from_slice(&[
                white_background(blue),
                white_background(green),
                white_background(red),
                0,
            ]);
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width_i32,
                // Negative height describes the top-down rows supplied above.
                biHeight: -height_i32,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: byte_length,
                ..Default::default()
            },
            ..Default::default()
        };
        let title: Vec<u16> = "Radiate Photobooth - Photo layout\0"
            .encode_utf16()
            .collect();
        let document = DOCINFOW {
            cbSize: size_of::<DOCINFOW>() as i32,
            lpszDocName: title.as_ptr(),
            lpszOutput: null(),
            lpszDatatype: null(),
            fwType: 0,
        };
        // SAFETY: All pointers below reference live buffers with matching bitmap
        // dimensions. No printer operation precedes the guest's confirmation.
        unsafe {
            let job_id = StartDocW(dc, &document);
            if job_id <= 0 {
                return Err(failed("Could not start the print job"));
            }
            let mut job = PrintJob { dc, active: true };
            for _ in 0..copies {
                if StartPage(dc) <= 0 {
                    return Err(failed("Could not start the printed page"));
                }
                if SetStretchBltMode(dc, HALFTONE) == 0 || SetBrushOrgEx(dc, 0, 0, null_mut()) == 0
                {
                    return Err(failed("Could not configure photo printing"));
                }
                let lines = StretchDIBits(
                    dc,
                    x,
                    y,
                    print_width,
                    print_height,
                    0,
                    0,
                    width_i32,
                    height_i32,
                    bitmap.as_ptr().cast(),
                    &info,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                );
                if lines == 0 || lines == GDI_ERROR {
                    return Err(failed("The printer could not render the photo"));
                }
                if EndPage(dc) <= 0 {
                    return Err(failed("Could not finish the printed page"));
                }
            }
            if EndDoc(dc) <= 0 {
                return Err(failed("Could not submit the print job"));
            }
            job.active = false;
            Ok(PrintOutcome::Submitted { job_id, copies })
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use windows_sys::Win32::Graphics::Gdi::DM_PAPERSIZE;
        use windows_sys::Win32::UI::Controls::Dialogs::PD_USEDEVMODECOPIESANDCOLLATE;

        #[test]
        fn native_dialog_receives_app_quantity_and_application_owns_copies() {
            let dialog = new_dialog(7, None);
            assert_eq!(dialog.0.nCopies, 7);
            assert!(dialog.0.hDevMode.is_null());
            assert_eq!(dialog.0.Flags & PD_USEDEVMODECOPIESANDCOLLATE, 0);
            assert_ne!(dialog.0.Flags & PD_RETURNDC, 0);
        }

        #[test]
        fn driver_copy_reset_preserves_the_selected_paper() {
            let mut mode = DEVMODEW::default();
            mode.dmFields = DM_COPIES | DM_PAPERSIZE;
            mode.Anonymous1.Anonymous1.dmCopies = 7;
            mode.Anonymous1.Anonymous1.dmPaperSize = 256;
            single_driver_copy(&mut mode);
            // SAFETY: This test initialized the printer variant above.
            unsafe {
                assert_eq!(mode.Anonymous1.Anonymous1.dmCopies, 1);
                assert_eq!(mode.Anonymous1.Anonymous1.dmPaperSize, 256);
            }
            assert_ne!(mode.dmFields & DM_PAPERSIZE, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_quantity_is_rejected_before_opening_a_print_dialog() {
        let image = DynamicImage::new_rgb8(1, 1);
        assert!(print_image(&image, 0, None).unwrap_err().contains("copies"));
        assert!(print_image(&image, MAX_COPIES + 1, None)
            .unwrap_err()
            .contains("copies"));
        assert!(validate_copies(1).is_ok());
        assert!(validate_copies(MAX_COPIES).is_ok());
    }
}
