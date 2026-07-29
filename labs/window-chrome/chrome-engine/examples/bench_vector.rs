// Resident cost of the vector-only stack, with no bitmap rasterizer loaded.
// Run alongside `bench` to compare: that binary loads both paths, so its
// figures cannot separate them.
use std::time::Instant;

use d2b_chrome_engine::{
    color::Rgba,
    skia::{self, TabGeometry},
    variant::Action,
    vectext::VectorFont,
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
    println!("start                   {:>7} kB", rss_kb());

    let font = VectorFont::from_bytes(PROTOTYPE_FONT).unwrap();
    println!("face parsed             {:>7} kB", rss_kb());

    let width = 1280_u32;
    let band = 32_u32;
    let geo = TabGeometry {
        x: 8.0,
        y: 1.0,
        width: 130.0,
        height: 30.0,
        radius: 6.0,
        bar: 3.0,
        hairline: 1.0,
    };

    let frames = 1000;
    let t = Instant::now();
    let mut sink = 0_u64;
    for _ in 0..frames {
        let mut pm = tiny_skia::Pixmap::new(width, band).unwrap();
        skia::draw_tab_frame(
            &mut pm,
            &geo,
            Rgba::rgb(0x25, 0x27, 0x2b),
            Rgba::rgb(0xff, 0xb3, 0x47),
            0.45,
        );
        font.draw(&mut pm, "Work", 12.0, 0.0, 18.0, 21.0, Rgba::WHITE);
        skia::draw_chevron(&mut pm, 100.0, 16.0, 9.0, false, Rgba::WHITE);
        for i in 0..5 {
            skia::draw_action_icon(
                &mut pm,
                Action::DEFAULTS[i],
                150.0 + i as f32 * 22.0,
                7.0,
                18.0,
                Rgba::WHITE,
            );
        }
        sink += pm.data().len() as u64;
    }
    println!(
        "full tab, expanded      {:>6.3} ms/frame",
        t.elapsed().as_secs_f64() * 1000.0 / frames as f64
    );
    println!("peak                    {:>7} kB", rss_kb());
    std::hint::black_box(sink);
}
