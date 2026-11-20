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
    let text: Vec<_> = format!("{number}%").chars().collect();
    let cells = text.len() * 6 - 1;
    let mut mask = vec![false; SIZE * SIZE];
    for y in 15..81 {
        for x in 4..92 {
            let gx = (x - 4) as f64 / 88.0 * cells as f64;
            let index = gx as usize / 6;
            let col = gx as usize % 6;
            let row = (y - 15) * 7 / 66;
            if index < text.len() && col < 5 && glyph(text[index])[row] & (1 << (4 - col)) != 0 {
                mask[y * SIZE + x] = true;
            }
        }
    }
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
#[cfg(test)]
mod tests {
    use super::*;
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
