// Generates a colourful, textured wallpaper for judging chrome against a busy
// background rather than a flat fill.
use d2b_chrome_engine::{canvas::Canvas, color::Rgba};

fn main() {
    let (w, h) = (1920usize, 1200usize);
    let mut c = Canvas::new(w, h, Rgba::rgb(0, 0, 0));

    for y in 0..h {
        for x in 0..w {
            let fx = x as f64 / w as f64;
            let fy = y as f64 / h as f64;

            // Layered diagonal waves give broad colour movement.
            let a = ((fx * 6.0 + fy * 3.0) * std::f64::consts::TAU).sin();
            let b = ((fx * 2.5 - fy * 4.5) * std::f64::consts::TAU).sin();
            let d = ((fx * 11.0 + fy * 9.0) * std::f64::consts::TAU).sin();

            // Warm-to-cool gradient with saturated highlights.
            let r = 0.42 + 0.34 * a + 0.16 * b + 0.06 * d;
            let g = 0.24 + 0.30 * b + 0.18 * ((fy * 3.0) * std::f64::consts::TAU).cos();
            let bl = 0.55 + 0.32 * ((fx * 4.0 - fy * 2.0) * std::f64::consts::TAU).cos()
                + 0.10 * d;

            // Fine grain so the surface is textured, not just gradients.
            let n = (((x * 7919 + y * 104_729) % 251) as f64 / 251.0 - 0.5) * 0.10;

            let q = |v: f64| ((v + n).clamp(0.0, 1.0) * 255.0).round() as u8;
            c.blend(x as i32, y as i32, Rgba::rgb(q(r), q(g), q(bl)));
        }
    }

    // A few translucent bands to add structure the chrome has to survive.
    for i in 0..7 {
        let bx = (i as f64 / 7.0 * w as f64) as i32;
        c.fill_rect(bx, 0, 70, h as u32, Rgba::rgba(255, 255, 255, 16));
        c.fill_rect(0, bx % h as i32, w as u32, 40, Rgba::rgba(0, 0, 0, 20));
    }

    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "../out/wallpaper.png".to_owned());
    c.write_png(&path).unwrap();
    println!("wrote {path} {}x{}", c.width, c.height);
}
