//! Generates the app icon (`assets/icon.png`): a plant growing up a lattice —
//! the trellis motif. Run once; `main.rs` embeds the result via `include_bytes!`.

use image::{ImageBuffer, Rgba};

type Img = ImageBuffer<Rgba<u8>, Vec<u8>>;

const N: i32 = 256;
const R: f32 = 52.0; // corner radius of the rounded-square background

fn main() {
    let mut img: Img = ImageBuffer::from_pixel(N as u32, N as u32, Rgba([0, 0, 0, 0]));

    // Background: rounded dark-navy square.
    for y in 0..N {
        for x in 0..N {
            if inside(x as f32 + 0.5, y as f32 + 0.5) {
                img.put_pixel(x as u32, y as u32, Rgba([30, 37, 48, 255]));
            }
        }
    }

    // Lattice: a subtle blue diamond grid.
    let blue = [59.0, 130.0, 246.0];
    let spacing = 40.0;
    let mut c = -(N as f32);
    while c < N as f32 {
        line(&mut img, 0.0, c, N as f32, c + N as f32, blue, 0.20, 1.1); // ↘
        line(&mut img, 0.0, c + N as f32, N as f32, c, blue, 0.20, 1.1); // ↗
        c += spacing;
    }

    // Vine: a green stem with one side branch, nodes at each joint.
    let green = [34.0, 197.0, 94.0];
    let leaf = [74.0, 222.0, 128.0];
    let ring = [16.0, 26.0, 20.0];

    let stem = [(74.0, 200.0), (118.0, 150.0), (98.0, 100.0), (74.0, 58.0)];
    let branch = [(118.0, 150.0), (168.0, 118.0), (202.0, 74.0)];

    for seg in stem.windows(2) {
        line(&mut img, seg[0].0, seg[0].1, seg[1].0, seg[1].1, green, 1.0, 4.0);
    }
    for seg in branch.windows(2) {
        line(&mut img, seg[0].0, seg[0].1, seg[1].0, seg[1].1, green, 1.0, 3.5);
    }

    for &(x, y) in stem.iter().chain(branch.iter().skip(1)) {
        disc(&mut img, x, y, 11.0, ring, 1.0);
        disc(&mut img, x, y, 8.5, leaf, 1.0);
    }

    std::fs::create_dir_all("assets").expect("create assets dir");
    img.save("assets/icon.png").expect("save icon");
    eprintln!("Wrote assets/icon.png");
}

/// Is the point inside the rounded-square background?
fn inside(x: f32, y: f32) -> bool {
    let (w, h) = (N as f32, N as f32);
    let cx = x.clamp(R, w - R);
    let cy = y.clamp(R, h - R);
    let (dx, dy) = (x - cx, y - cy);
    dx * dx + dy * dy <= R * R + 0.5
}

/// Alpha-blend a color onto a pixel, clipped to the background shape.
fn set(img: &mut Img, x: i32, y: i32, col: [f32; 3], a: f32) {
    if x < 0 || y < 0 || x >= N || y >= N || a <= 0.0 {
        return;
    }
    if !inside(x as f32 + 0.5, y as f32 + 0.5) {
        return;
    }
    let p = img.get_pixel_mut(x as u32, y as u32);
    for i in 0..3 {
        p[i] = (col[i] * a + p[i] as f32 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
    }
    p[3] = 255;
}

/// A filled, edge-antialiased disc.
fn disc(img: &mut Img, cx: f32, cy: f32, rad: f32, col: [f32; 3], a: f32) {
    let r0 = rad.ceil() as i32 + 1;
    let (icx, icy) = (cx.round() as i32, cy.round() as i32);
    for dy in -r0..=r0 {
        for dx in -r0..=r0 {
            let d = ((dx * dx + dy * dy) as f32).sqrt();
            let edge = (rad - d + 0.5).clamp(0.0, 1.0);
            set(img, icx + dx, icy + dy, col, a * edge);
        }
    }
}

/// A thick line, drawn as a run of discs.
fn line(img: &mut Img, x0: f32, y0: f32, x1: f32, y1: f32, col: [f32; 3], a: f32, w: f32) {
    let len = (x1 - x0).hypot(y1 - y0);
    let steps = (len * 2.0).ceil().max(1.0) as i32;
    for s in 0..=steps {
        let t = s as f32 / steps as f32;
        disc(img, x0 + (x1 - x0) * t, y0 + (y1 - y0) * t, w * 0.5, col, a);
    }
}
