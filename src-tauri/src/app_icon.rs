use base64::{engine::general_purpose::STANDARD, Engine};
use image::{DynamicImage, ImageFormat, RgbaImage};
use std::io::Cursor;
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::HWND,
        Graphics::Gdi::{
            CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
            ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        },
        Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES,
        UI::{
            Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON},
            WindowsAndMessaging::{DestroyIcon, DrawIconEx, DI_NORMAL},
        },
    },
};

const ICON_SIZE: i32 = 64;

pub fn from_executable(path: &str) -> Result<String, String> {
    if path.is_empty() {
        return Err("Application path is empty".into());
    }
    let path_wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let mut info = SHFILEINFOW::default();
        if SHGetFileInfoW(
            PCWSTR(path_wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        ) == 0
        {
            return Err("Windows did not provide an application icon".into());
        }
        let desktop = GetDC(HWND(std::ptr::null_mut()));
        if desktop.0.is_null() {
            let _ = DestroyIcon(info.hIcon);
            return Err("Unable to access the desktop drawing surface".into());
        }
        let memory = CreateCompatibleDC(desktop);
        if memory.0.is_null() {
            let _ = ReleaseDC(HWND(std::ptr::null_mut()), desktop);
            let _ = DestroyIcon(info.hIcon);
            return Err("Unable to create an icon drawing surface".into());
        }
        let bitmap = CreateCompatibleBitmap(desktop, ICON_SIZE, ICON_SIZE);
        if bitmap.0.is_null() {
            let _ = DeleteDC(memory);
            let _ = ReleaseDC(HWND(std::ptr::null_mut()), desktop);
            let _ = DestroyIcon(info.hIcon);
            return Err("Unable to create an icon drawing surface".into());
        }
        let previous = SelectObject(memory, bitmap);
        if previous.0.is_null() {
            let _ = DeleteObject(bitmap);
            let _ = DeleteDC(memory);
            let _ = ReleaseDC(HWND(std::ptr::null_mut()), desktop);
            let _ = DestroyIcon(info.hIcon);
            return Err("Unable to select the icon drawing surface".into());
        }
        let result = DrawIconEx(
            memory, 0, 0, info.hIcon, ICON_SIZE, ICON_SIZE, 0, None, DI_NORMAL,
        );
        let mut bitmap_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: ICON_SIZE,
                biHeight: -ICON_SIZE,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = vec![0_u8; (ICON_SIZE * ICON_SIZE * 4) as usize];
        SelectObject(memory, previous);
        let rows = if result.is_ok() {
            GetDIBits(
                memory,
                bitmap,
                0,
                ICON_SIZE as u32,
                Some(pixels.as_mut_ptr().cast()),
                &mut bitmap_info,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };
        let _ = DeleteObject(bitmap);
        let _ = DeleteDC(memory);
        let _ = ReleaseDC(HWND(std::ptr::null_mut()), desktop);
        let _ = DestroyIcon(info.hIcon);
        result.map_err(|err| err.to_string())?;
        if rows == 0 {
            return Err("Windows did not return icon pixels".into());
        }
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let image = RgbaImage::from_raw(ICON_SIZE as u32, ICON_SIZE as u32, pixels)
            .ok_or("Unable to encode icon pixels")?;
        let mut png = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut png, ImageFormat::Png)
            .map_err(|err| err.to_string())?;
        Ok(format!(
            "data:image/png;base64,{}",
            STANDARD.encode(png.into_inner())
        ))
    }
}
