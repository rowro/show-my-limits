//! Transparent, compact tray artwork. Text includes '%' on every platform.
const SIZE: usize = 96;
fn glyph(c: char) -> [u8; 7] {
    match c {
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        '%' => [25, 26, 2, 4, 8, 11, 19],
        _ => [0; 7],
    }
}
pub fn render(title: &str, light: bool) -> tauri::image::Image<'static> {
    tauri::image::Image::new_owned(pixels(title, light), SIZE as u32, SIZE as u32)
}
fn pixels(title: &str, light: bool) -> Vec<u8> {
    let mut data = vec![0; SIZE * SIZE * 4];
    let number = title
        .strip_suffix('%')
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|n| *n <= 100);
    let Some(number) = number else {
        // A recognizable application mark instead of a transparent, empty slot.
        for y in 0..SIZE {
            for x in 0..SIZE {
                let dx = x as f64 + 0.5 - 48.0;
                let dy = y as f64 + 0.5 - 48.0;
                let radius = dx.hypot(dy);
                let angle = dy.atan2(dx);
                if (29.0..39.0).contains(&radius) && !((2.5..3.05).contains(&angle)) {
                    let i = (y * SIZE + x) * 4;
                    data[i..i + 4].copy_from_slice(&[76, 151, 121, 255]);
                }
            }
        }
        return data;
    };
    let digits: Vec<_> = number.to_string().chars().collect();
    let cells = digits.len() * 6 - 1;
    // Preserve the 5:7 glyph proportions. Keep the suffix smaller rather than
    // stretching numerals vertically to fill the square notification icon.
    let scale = (70.0 / cells as f64).min(58.0 / 7.0);
    let suffix_scale = (scale * 0.6).min(3.5);
    let width = cells as f64 * scale + 3.0 + 5.0 * suffix_scale;
    let left = (SIZE as f64 - width) / 2.0;
    let top = (SIZE as f64 - 7.0 * scale) / 2.0;
    let mut mask = vec![false; SIZE * SIZE];
    for (index, digit) in digits.iter().enumerate() {
        paint_glyph(
            &mut mask,
            *digit,
            left + index as f64 * 6.0 * scale,
            top,
            scale,
        );
    }
    paint_glyph(
        &mut mask,
        '%',
        left + cells as f64 * scale + 3.0,
        top + 7.0 * (scale - suffix_scale),
        suffix_scale,
    );
    let foreground = if light {
        [25, 30, 28, 255]
    } else {
        [246, 249, 247, 255]
    };
    let outline = if light {
        [246, 249, 247, 210]
    } else {
        [25, 30, 28, 210]
    };
    for y in 1..SIZE - 1 {
        for x in 1..SIZE - 1 {
            let i = y * SIZE + x;
            let color = if mask[i] {
                Some(foreground)
            } else if (-1isize..=1).any(|dy| {
                (-1isize..=1).any(|dx| {
                    mask[((y as isize + dy) as usize) * SIZE + (x as isize + dx) as usize]
                })
            }) {
                Some(outline)
            } else {
                None
            };
            if let Some(color) = color {
                data[i * 4..i * 4 + 4].copy_from_slice(&color);
            }
        }
    }
    data
}
fn paint_glyph(mask: &mut [bool], character: char, left: f64, top: f64, scale: f64) {
    let rows = glyph(character);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let column = (x as f64 + 0.5 - left) / scale;
            let row = (y as f64 + 0.5 - top) / scale;
            if column >= 0.0
                && column < 5.0
                && row >= 0.0
                && row < 7.0
                && rows[row as usize] & (1 << (4 - column as usize)) != 0
            {
                mask[y * SIZE + x] = true;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numerals_keep_their_original_five_by_seven_proportions() {
        let mut mask = vec![false; SIZE * SIZE];
        paint_glyph(&mut mask, '0', 10.0, 10.0, 6.0);
        let points: Vec<_> = mask
            .iter()
            .enumerate()
            .filter(|(_, on)| **on)
            .map(|(i, _)| (i % SIZE, i / SIZE))
            .collect();
        let width = points.iter().map(|p| p.0).max().unwrap()
            - points.iter().map(|p| p.0).min().unwrap()
            + 1;
        let height = points.iter().map(|p| p.1).max().unwrap()
            - points.iter().map(|p| p.1).min().unwrap()
            + 1;
        assert_eq!((width, height), (30, 42));
    }
    #[test]
    fn empty_data_has_visible_logo_and_numbers_include_percent_without_background() {
        for title in ["—", "0%", "9%", "65%", "100%"] {
            let data = pixels(title, false);
            assert_eq!(data.len(), SIZE * SIZE * 4);
            assert_eq!(&data[..4], &[0, 0, 0, 0]);
            assert!(
                data.chunks_exact(4).filter(|p| p[3] > 0).count() > 500,
                "{title}"
            );
        }
        assert_eq!(render("65%", false).width(), SIZE as u32);
        assert_ne!(glyph('%'), [0; 7]);
        assert_ne!(pixels("65%", true), pixels("65%", false));
        assert_eq!(pixels("garbage", false), pixels("—", false));
    }
}
