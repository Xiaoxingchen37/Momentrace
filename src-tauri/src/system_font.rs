//! Reads the Windows interface font and the fonts the user actually has
//! installed, so the dashboard can render in the same type as the rest of the
//! desktop instead of a font that only the bundled build provides.
use std::collections::BTreeMap;
use windows::{
    Win32::{
        Foundation::LPARAM,
        Graphics::Gdi::{
            EnumFontFamiliesExW, GetDC, ReleaseDC, DEFAULT_CHARSET, LOGFONTW, TEXTMETRICW,
        },
        UI::WindowsAndMessaging::{
            SystemParametersInfoW, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS,
        },
    },
};

/// A pathological font list must not turn into a megabyte-long settings page.
const MAX_FONTS: usize = 800;

fn wide_to_string(value: &[u16]) -> String {
    let end = value.iter().position(|&unit| unit == 0).unwrap_or(value.len());
    String::from_utf16_lossy(&value[..end]).trim().to_string()
}

/// The face Windows uses for menus and message boxes, which is what "same as my
/// machine" means to a user. Falls back to an empty string when the shell query
/// fails so the stylesheet keeps its own stack.
pub fn ui_font() -> String {
    unsafe {
        let mut metrics = NONCLIENTMETRICSW::default();
        metrics.cbSize = std::mem::size_of::<NONCLIENTMETRICSW>() as u32;
        let queried = SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            Some(&mut metrics as *mut _ as *mut core::ffi::c_void),
            Default::default(),
        );
        if queried.is_err() {
            return String::new();
        }
        let message = wide_to_string(&metrics.lfMessageFont.lfFaceName);
        if message.is_empty() {
            wide_to_string(&metrics.lfMenuFont.lfFaceName)
        } else {
            message
        }
    }
}

unsafe extern "system" fn collect(
    logfont: *const LOGFONTW,
    _metric: *const TEXTMETRICW,
    _font_type: u32,
    lparam: LPARAM,
) -> i32 {
    let fonts = &mut *(lparam.0 as *mut BTreeMap<String, String>);
    if !logfont.is_null() {
        let name = wide_to_string(&(*logfont).lfFaceName);
        // Vertical CJK faces (prefixed with '@') only make glyphs rotate.
        if !name.is_empty() && !name.starts_with('@') && fonts.len() < MAX_FONTS {
            fonts.entry(name.to_lowercase()).or_insert(name);
        }
    }
    // Non-zero keeps enumeration going; the whole point is to list every face.
    1
}

/// Every installed family, deduplicated case-insensitively. Horizontal faces
/// only, because a rotated one cannot render a normal label.
pub fn installed_fonts() -> Vec<String> {
    let mut fonts: BTreeMap<String, String> = BTreeMap::new();
    unsafe {
        let hdc = GetDC(None);
        if hdc.0.is_null() {
            return Vec::new();
        }
        let mut query = LOGFONTW::default();
        query.lfCharSet = DEFAULT_CHARSET;
        EnumFontFamiliesExW(
            hdc,
            &query,
            Some(collect),
            LPARAM(&mut fonts as *mut _ as isize),
            0,
        );
        ReleaseDC(None, hdc);
    }
    let preferred = ui_font().to_lowercase();
    let mut names: Vec<String> = fonts.into_values().collect();
    // The interface font leads the list: it is the one most users want.
    names.sort_by_key(|name| (name.to_lowercase() != preferred, name.to_lowercase()));
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_windows_interface_face_is_readable() {
        let face = ui_font();
        assert!(!face.is_empty(), "Windows did not report an interface face");
        assert!(!face.starts_with('@'));
    }

    #[test]
    fn the_installed_list_leads_with_the_interface_face() {
        let fonts = installed_fonts();
        assert!(!fonts.is_empty(), "no installed fonts were enumerated");
        assert!(fonts.iter().all(|name| !name.starts_with('@')));
        assert_eq!(
            fonts.len(),
            fonts
                .iter()
                .map(|name| name.to_lowercase())
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            "font families must be deduplicated"
        );
        let preferred = ui_font().to_lowercase();
        if !preferred.is_empty() {
            assert_eq!(fonts[0].to_lowercase(), preferred);
        }
    }
}
