// Measure the cost of vector rendering: peak RSS and per-frame time for the
// hand-rolled path versus tiny-skia, so the choice is made on numbers.
use std::time::Instant;

use d2b_chrome_engine::{
    color::Rgba,
    skia::{self, TabGeometry},
    text::TextRenderer,
    variant::{render, Candidate, ChromeSpec},
    PROTOTYPE_FONT,
};

fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1).and_then(|v| v.parse().ok()))
        })
        .unwrap_or(0)
}

fn main() {
    let width = 1280_u32;
    let band = 32_u32;
    let frames = 500;

    println!("baseline RSS            {:>7} kB", rss_kb());

    let fonts = TextRenderer::from_bytes(PROTOTYPE_FONT).unwrap();
    println!("after font load         {:>7} kB", rss_kb());

    // Hand-rolled path, as originally written.
    let mut spec = ChromeSpec::new(Candidate::Tab, "Work", Rgba::rgb(0xff, 0xb3, 0x47));
    spec.content_width = width;
    spec.content_height = 1;
    let t0 = Instant::now();
    let mut sink = 0_u64;
    for _ in 0..frames {
        let r = render(&spec, &fonts, Rgba::TRANSPARENT);
        sink += r.canvas.pixels.len() as u64;
    }
    let manual = t0.elapsed();
    let rss_manual = rss_kb();
    println!(
        "hand-rolled  {:>6.2} ms/frame   RSS {:>7} kB",
        manual.as_secs_f64() * 1000.0 / frames as f64,
        rss_manual
    );

    // Vector path.
    let geo = TabGeometry {
        x: 8.0,
        y: 1.0,
        width: 120.0,
        height: 30.0,
        radius: 6.0,
        bar: 3.0,
        hairline: 1.0,
    };
    let t1 = Instant::now();
    for _ in 0..frames {
        let mut pm = tiny_skia::Pixmap::new(width, band).unwrap();
        skia::draw_tab_frame(
            &mut pm,
            &geo,
            Rgba::rgb(0x25, 0x27, 0x2b),
            Rgba::rgb(0xff, 0xb3, 0x47),
            0.45,
        );
        skia::draw_chevron(&mut pm, 90.0, 16.0, 9.0, false, Rgba::WHITE);
        for i in 0..5 {
            skia::draw_action_icon(
                &mut pm,
                d2b_chrome_engine::variant::Action::DEFAULTS[i],
                140.0 + i as f32 * 22.0,
                7.0,
                18.0,
                Rgba::WHITE,
            );
        }
        sink += pm.data().len() as u64;
    }
    let vector = t1.elapsed();
    println!(
        "tiny-skia    {:>6.2} ms/frame   RSS {:>7} kB",
        vector.as_secs_f64() * 1000.0 / frames as f64,
        rss_kb()
    );

    // One band buffer at 4K width, the realistic worst case for a single window.
    let big = tiny_skia::Pixmap::new(3840, 48).unwrap();
    println!(
        "4K band buffer          {:>7} kB   (RSS now {} kB)",
        big.data().len() / 1024,
        rss_kb()
    );
    // Outline text: parse the face and draw without a raster cache.
    let vf = d2b_chrome_engine::vectext::VectorFont::from_bytes(PROTOTYPE_FONT).unwrap();
    println!("after vector font load  {:>7} kB", rss_kb());
    let t2 = Instant::now();
    for _ in 0..frames {
        let mut pm = tiny_skia::Pixmap::new(width, band).unwrap();
        skia::draw_tab_frame(
            &mut pm,
            &geo,
            Rgba::rgb(0x25, 0x27, 0x2b),
            Rgba::rgb(0xff, 0xb3, 0x47),
            0.45,
        );
        vf.draw(&mut pm, "Work", 12.0, 0.0, 18.0, 21.0, Rgba::WHITE);
        skia::draw_chevron(&mut pm, 90.0, 16.0, 9.0, false, Rgba::WHITE);
        sink += pm.data().len() as u64;
    }
    println!(
        "vector text  {:>6.2} ms/frame   RSS {:>7} kB",
        t2.elapsed().as_secs_f64() * 1000.0 / frames as f64,
        rss_kb()
    );
    std::hint::black_box(sink);
}
