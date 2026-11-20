use std::{mem::size_of, ptr::null_mut, slice};

use windows_sys::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC,
    CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    FW_SEMIBOLD, GdiFlush, SelectObject, SetBkMode, SetTextColor, TRANSPARENT, TextOutW,
};

const OVERSAMPLE: usize = 4;

pub fn render(values: &[String; 2], light: bool, width: usize, height: usize) -> Option<Vec<u8>> {
    if width < 8 || height < 6 {
        return None;
    }
    let label_width = width * 2 / 5;
    let value_width = width - label_width;
    let mut surface = Vec::with_capacity(width * height * 4);
    for (row, (label, value)) in ["5h -", "7d -"].into_iter().zip(values).enumerate() {
        let row_height = height / 2 + row * (height % 2);
        let font_height = row_height * OVERSAMPLE * 5 / 8;
        let label = render_text(label, light, label_width, row_height, font_height * 11 / 12, true)?;
        let value = render_text(value, light, value_width, row_height, font_height, false)?;
        for (label, value) in label.chunks_exact(label_width * 4).zip(value.chunks_exact(value_width * 4)) {
            surface.extend_from_slice(label);
            surface.extend_from_slice(value);
        }
    }
    // Transparent areas of a layered window pass mouse events through. One
    // alpha level keeps the spaces and margins clickable without a visible fill.
    for pixel in surface.chunks_exact_mut(4) {
        pixel[3] = pixel[3].max(1);
    }
    Some(surface)
}

fn render_text(
    title: &str, light: bool, width: usize, height: usize, font_height: usize, pad_left: bool,
) -> Option<Vec<u8>> {
    let text: Vec<_> = title.encode_utf16().collect();
    let (source_width, source_height) = (font_height * (text.len() + 1), font_height * 2);
    let bitmap_info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: source_width as i32,
            biHeight: -(source_height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..Default::default()
        },
        ..Default::default()
    };

    // The DC and its selected objects are private to this call. Flush the GDI
    // batch before reading the DIB, and release the DC before deleting its objects.
    unsafe {
        let dc = CreateCompatibleDC(null_mut());
        if dc.is_null() {
            return None;
        }
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(dc, &bitmap_info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        let font = CreateFontW(
            -(font_height as i32), 0, 0, 0, FW_SEMIBOLD as i32, 0, 0, 0,
            u32::from(DEFAULT_CHARSET), 0, 0, u32::from(ANTIALIASED_QUALITY), 0,
            windows_sys::w!("Segoe UI"),
        );
        let icon = if bitmap.is_null() || bits.is_null() || font.is_null() {
            None
        } else {
            let bytes = source_width * source_height * 4;
            std::ptr::write_bytes(bits.cast::<u8>(), 0, bytes);
            SelectObject(dc, bitmap);
            SelectObject(dc, font);
            SetBkMode(dc, TRANSPARENT as i32);
            SetTextColor(dc, 0x00ff_ffff);
            let drawn = TextOutW(dc, 4, 0, text.as_ptr(), text.len() as i32) != 0;
            GdiFlush();
            if drawn {
                let bitmap = slice::from_raw_parts(bits.cast::<u8>(), bytes);
                fit_ink(bitmap, (source_width, source_height), (width, height), light, pad_left)
            } else {
                None
            }
        };
        DeleteDC(dc);
        if !font.is_null() {
            DeleteObject(font);
        }
        if !bitmap.is_null() {
            DeleteObject(bitmap);
        }
        icon
    }
}

fn fit_ink(
    bitmap: &[u8], (source_width, source_height): (usize, usize), (width, height): (usize, usize),
    light: bool, pad_left: bool,
) -> Option<Vec<u8>> {
    let coverage = |x: usize, y: usize| bitmap[(y * source_width + x) * 4];
    let (mut left, mut top, mut right, mut bottom) = (source_width, source_height, 0, 0);
    for y in 0..source_height {
        for x in 0..source_width {
            if coverage(x, y) != 0 {
                left = left.min(x);
                top = top.min(y);
                right = right.max(x + 1);
                bottom = bottom.max(y + 1);
            }
        }
    }
    if left >= right || top >= bottom || right == source_width {
        return None;
    }

    let (ink_width, ink_height) = (right - left, bottom - top);
    let aspect = ink_width as f64 / ink_height as f64;
    let inset = (height / 4).clamp(1, (width - 1) / 2);
    let offset_x = if pad_left { inset } else { 0 };
    let available_width = width - offset_x - inset;
    let target_height = (ink_height as f64 / OVERSAMPLE as f64)
        .min(available_width as f64 / aspect)
        .round()
        .clamp(1.0, (height - 2).min(ink_height) as f64) as usize;
    let target_width = (target_height as f64 * aspect).round() as usize;
    let target_width = target_width.clamp(1, available_width.min(ink_width));
    let offset_y = (height - target_height) / 2;
    let color = if light { 24 } else { 250 };
    let mut pixels = vec![0; width * height * 4];
    for y in 0..target_height {
        let y0 = top + y * ink_height / target_height;
        let y1 = top + (y + 1) * ink_height / target_height;
        for x in 0..target_width {
            let x0 = left + x * ink_width / target_width;
            let x1 = left + (x + 1) * ink_width / target_width;
            let mut total = 0;
            for row in y0..y1 {
                for col in x0..x1 {
                    total += usize::from(coverage(col, row));
                }
            }
            let samples = (y1 - y0) * (x1 - x0);
            let alpha = (total + samples / 2) / samples;
            // UpdateLayeredWindow blends premultiplied BGRA pixels.
            let channel = ((color * alpha + 127) / 255) as u8;
            let pixel = ((offset_y + y) * width + offset_x + x) * 4;
            pixels[pixel..pixel + 4].copy_from_slice(&[channel, channel, channel, alpha as u8]);
        }
    }
    Some(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_rows_fit_with_antialiased_edges_and_clickable_padding() {
        for (width, height) in [(76, 30), (95, 50), (114, 60), (133, 70)] {
            for (short, long) in [("65%", "90%"), ("100%", "100%"), ("—", "—")] {
                let pixels = render(&[short.to_owned(), long.to_owned()], false, width, height).unwrap();
                let alpha = |x: usize, y: usize| pixels[(y * width + x) * 4 + 3];
                assert!((0..height).all(|y| alpha(0, y) == 1 && alpha(width - 1, y) == 1));
                assert!(pixels.chunks_exact(4).all(|pixel| pixel[3] != 0));
                assert!(pixels.chunks_exact(4).any(|pixel| (2..255).contains(&pixel[3])));
                for band in [0..height / 2, height / 2..height] {
                    assert!(band.into_iter().any(|y| (0..width).any(|x| alpha(x, y) >= 128)));
                }
            }
        }
    }

    #[test]
    fn three_digit_values_leave_room_for_the_percent_sign() {
        for (width, height) in [(76, 30), (76, 40), (133, 70)] {
            let column = width * 2 / 5;
            let alpha_at = |pixels: &[u8], x, y| pixels[(y * width + x) * 4 + 3];
            let measure = |text: &str| {
                let pixels = render(&[text.to_owned(), text.to_owned()], false, width, height).unwrap();
                let columns: Vec<_> = (column..width)
                    .filter(|x| (0..height / 2).any(|y| alpha_at(&pixels, *x, y) > 1))
                    .collect();
                (*columns.first().unwrap(), *columns.last().unwrap())
            };
            let (left, right) = measure("100%");
            let (digits_left, digits_right) = measure("100");
            assert_eq!(left, digits_left, "values should start at the same column");
            assert_eq!(left, measure("65%").0);
            assert!(right > digits_right, "the percent sign must add visible width");
            assert!(right < width - 1, "the last glyph needs right padding");
        }
    }
}
