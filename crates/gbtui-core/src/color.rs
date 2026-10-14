//! 256-color palette math used by quantization.
//!
//! `SOURCE: crates/codegen/xai-grok-pager-render/src/render/color.rs (STRIP)`

use ratatui::style::Color;

const CUBE_VALUES: [u8; 6] = [0, 95, 135, 175, 215, 255];

pub fn indexed_to_rgb(index: u8) -> (u8, u8, u8) {
    match index {
        0 => (0, 0, 0),
        1 => (128, 0, 0),
        2 => (0, 128, 0),
        3 => (128, 128, 0),
        4 => (0, 0, 128),
        5 => (128, 0, 128),
        6 => (0, 128, 128),
        7 => (192, 192, 192),
        8 => (128, 128, 128),
        9 => (255, 0, 0),
        10 => (0, 255, 0),
        11 => (255, 255, 0),
        12 => (0, 0, 255),
        13 => (255, 0, 255),
        14 => (0, 255, 255),
        15 => (255, 255, 255),
        16..=231 => {
            let n = index - 16;
            let r = CUBE_VALUES[(n / 36) as usize];
            let g = CUBE_VALUES[((n % 36) / 6) as usize];
            let b = CUBE_VALUES[(n % 6) as usize];
            (r, g, b)
        }
        232..=255 => {
            let v = 8 + (index - 232) * 10;
            (v, v, v)
        }
    }
}

pub fn nearest_indexed(r: u8, g: u8, b: u8) -> u8 {
    let ri = nearest_cube_channel(r);
    let gi = nearest_cube_channel(g);
    let bi = nearest_cube_channel(b);
    let cube_idx = 16 + 36 * ri as u16 + 6 * gi as u16 + bi as u16;
    let cube_dist = sq_dist(
        r,
        g,
        b,
        CUBE_VALUES[ri as usize],
        CUBE_VALUES[gi as usize],
        CUBE_VALUES[bi as usize],
    );

    let lum = (r as u16 + g as u16 + b as u16) / 3;
    let gray_step = if lum <= 3 {
        0u8
    } else if lum >= 243 {
        23
    } else {
        ((lum as i16 - 8 + 5) / 10).clamp(0, 23) as u8
    };
    let gv = (8 + gray_step as u16 * 10) as u8;
    let gray_dist = sq_dist(r, g, b, gv, gv, gv);

    if gray_dist < cube_dist {
        232 + gray_step
    } else {
        cube_idx as u8
    }
}

fn nearest_cube_channel(v: u8) -> u8 {
    let mut best = 0u8;
    let mut best_d = v.abs_diff(CUBE_VALUES[0]) as u16;
    for i in 1..6u8 {
        let d = v.abs_diff(CUBE_VALUES[i as usize]) as u16;
        if d < best_d {
            best = i;
            best_d = d;
        }
    }
    best
}

fn sq_dist(r1: u8, g1: u8, b1: u8, r2: u8, g2: u8, b2: u8) -> u32 {
    let dr = r1 as i32 - r2 as i32;
    let dg = g1 as i32 - g2 as i32;
    let db = b1 as i32 - b2 as i32;
    (dr * dr + dg * dg + db * db) as u32
}

pub fn indexed_to_ansi16(n: u8) -> Color {
    match n {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        7 => Color::White,
        8 => Color::DarkGray,
        9 => Color::LightRed,
        10 => Color::LightGreen,
        11 => Color::LightYellow,
        12 => Color::LightBlue,
        13 => Color::LightMagenta,
        14 => Color::LightCyan,
        15 => Color::White,
        _ => {
            let (r, g, b) = indexed_to_rgb(n);
            rgb_to_ansi16(r, g, b)
        }
    }
}

fn rgb_to_ansi16(r: u8, g: u8, b: u8) -> Color {
    const PALETTE: [(u8, u8, u8, Color); 16] = [
        (0, 0, 0, Color::Black),
        (128, 0, 0, Color::Red),
        (0, 128, 0, Color::Green),
        (128, 128, 0, Color::Yellow),
        (0, 0, 128, Color::Blue),
        (128, 0, 128, Color::Magenta),
        (0, 128, 128, Color::Cyan),
        (192, 192, 192, Color::White),
        (128, 128, 128, Color::DarkGray),
        (255, 0, 0, Color::LightRed),
        (0, 255, 0, Color::LightGreen),
        (255, 255, 0, Color::LightYellow),
        (0, 0, 255, Color::LightBlue),
        (255, 0, 255, Color::LightMagenta),
        (0, 255, 255, Color::LightCyan),
        (255, 255, 255, Color::White),
    ];
    let mut best = Color::White;
    let mut best_dist = u32::MAX;
    for &(pr, pg, pb, color) in &PALETTE {
        let dist = sq_dist(r, g, b, pr, pg, pb);
        if dist < best_dist {
            best_dist = dist;
            best = color;
        }
    }
    best
}
