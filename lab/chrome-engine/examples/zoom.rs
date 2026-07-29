// Magnify the tab corner so the thick-to-thin transition can be judged.
use d2b_chrome_engine::{canvas::Canvas, color::Rgba, text::TextRenderer,
    variant::{render, Candidate, ChromeSpec}, PROTOTYPE_FONT};
fn main(){
    let f=TextRenderer::from_bytes(PROTOTYPE_FONT).unwrap();
    let mut s=ChromeSpec::new(Candidate::Tab,"Work",Rgba::parse_hex("#ffb347").unwrap());
    s.content_width=200; s.content_height=40;
    let r=render(&s,&f,Rgba::rgb(0x10,0x10,0x14));
    let src=&r.canvas;
    const Z:usize=8;
    let (cw,chh)=(60usize,40usize);
    let mut out=Canvas::new(cw*Z,chh*Z,Rgba::rgb(0x2e,0x2e,0x34));
    for y in 0..chh { for x in 0..cw {
        let p=src.get(x.min(src.width-1),y.min(src.height-1));
        for dy in 0..Z { for dx in 0..Z {
            out.blend((x*Z+dx) as i32,(y*Z+dy) as i32,p);
        }}
    }}
    out.write_png("../out/zoom-corner.png").unwrap();
    println!("wrote zoom-corner.png {}x{}", out.width, out.height);
}
