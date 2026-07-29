//! Renders the candidate and its controls to PNG contact sheets.
//!
//! Offline rendering exists so variant artwork iterates in seconds rather than
//! through a full compositor round-trip. The live nested-niri capture remains
//! the source of truth for how chrome behaves *inside* niri; this binary is for
//! how it looks.
//!
//! Usage: `cargo run --bin render-sheets -- <out-dir>`

use std::{env, fs, path::PathBuf};

use d2b_chrome_engine::{
    canvas::Canvas,
    color::Rgba,
    text::{blend_glyph, TextRenderer},
    variant::{render, Candidate, ChromeSpec, VisualState},
    PROTOTYPE_FONT,
};

/// Human display names lead; canonical targets are a deliberate control.
/// Accents are spread in luminance as well as hue, since colour is supportive
/// and text carries identity.
const PALETTE: &[(&str, &str)] = &[
    ("Work", "#ffa500"),
    ("Personal", "#7fc8ff"),
    ("Media", "#c792ea"),
    ("Banking", "#4ade80"),
];

/// Accents chosen to stress control C: a very light fill, a very dark fill, and
/// one sitting right at the black/white selection threshold where the naive
/// luma rule flips and the true worst case (4.58:1) lives.
const STRESS_ACCENTS: &[(&str, &str)] = &[
    ("light accent", "#f8e08e"),
    ("dark accent", "#3b2d6b"),
    ("threshold accent", "#2f72de"),
    ("naive-luma trap", "#04d800"),
];

const DARK: Rgba = Rgba::rgb(0x10, 0x10, 0x14);
const LIGHT: Rgba = Rgba::rgb(0xf4, 0xf4, 0xf8);
/// Stands in for the compositor background visible in niri's gaps.
const DESKTOP: Rgba = Rgba::rgb(0x2e, 0x2e, 0x34);
const BUDGET: u64 = 5 * 1024 * 1024;

struct Cell {
    caption: String,
    canvas: Canvas,
}

fn main() {
    let out = PathBuf::from(
        env::args()
            .nth(1)
            .unwrap_or_else(|| "../out/sheets".to_owned()),
    );
    fs::create_dir_all(&out).expect("create output dir");
    let fonts = TextRenderer::from_bytes(PROTOTYPE_FONT).expect("font");

    let sheets: Vec<(&str, Vec<Cell>)> = vec![
        ("01-candidates-and-controls", candidates(&fonts)),
        ("02-states", states(&fonts)),
        ("03-labels-and-scaling", labels(&fonts)),
        ("04-status-and-blocked", status_and_blocked(&fonts)),
        ("05-accessibility-passes", accessibility(&fonts)),
        ("06-accent-fill-across-palette", accent_fill_stress(&fonts)),
        ("07-compound-reflow", compound_reflow(&fonts)),
        ("08-status-token-states", status_token_states(&fonts)),
        ("09-tab", tab_states(&fonts)),
    ];

    let mut failed = false;
    for (name, cells) in sheets {
        let sheet = compose(&fonts, name, &cells);
        let path = out.join(format!("{name}.png"));
        sheet.write_png(&path).expect("write sheet");
        let bytes = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let over = bytes > BUDGET;
        failed |= over;
        println!(
            "{name}.png  {}x{}  {bytes} bytes{}",
            sheet.width,
            sheet.height,
            if over { "  OVER BUDGET" } else { "" }
        );
    }
    assert!(!failed, "a sheet exceeded the 5 MiB ceiling");
}

fn spec(candidate: Candidate, label: &str, accent: &str) -> ChromeSpec {
    let mut s = ChromeSpec::new(candidate, label, Rgba::parse_hex(accent).unwrap());
    s.content_width = 460;
    s.content_height = 150;
    s
}

fn candidates(f: &TextRenderer) -> Vec<Cell> {
    let mut cells = Vec::new();
    for (candidate, note) in [
        (Candidate::BandNeutral, "A candidate - painted band"),
        (Candidate::BandTransparent, "B control - bare band"),
        (Candidate::AccentFill, "C control - accent fill"),
        (Candidate::OutsideNotch, "D control - outside geometry"),
    ] {
        for (label, accent) in [PALETTE[0], PALETTE[1]] {
            let s = spec(candidate, label, accent);
            cells.push(Cell {
                caption: format!("{note} / {label}"),
                canvas: render(&s, f, DARK).canvas,
            });
        }
    }
    cells
}

fn states(f: &TextRenderer) -> Vec<Cell> {
    [
        ("focused", VisualState::focused()),
        ("unfocused", VisualState::default()),
        (
            "hover",
            VisualState {
                hover: true,
                ..VisualState::focused()
            },
        ),
        (
            "pressed",
            VisualState {
                pressed: true,
                ..VisualState::focused()
            },
        ),
        (
            "menu open",
            VisualState {
                menu_open: true,
                ..VisualState::focused()
            },
        ),
        (
            "keyboard focus",
            VisualState {
                keyboard_focus: true,
                ..VisualState::focused()
            },
        ),
    ]
    .into_iter()
    .map(|(name, state)| {
        let mut s = spec(Candidate::BandNeutral, "Work", PALETTE[0].1);
        s.state = state;
        Cell {
            caption: format!("A / {name}"),
            canvas: render(&s, f, DARK).canvas,
        }
    })
    .collect()
}

fn labels(f: &TextRenderer) -> Vec<Cell> {
    let mut cells = Vec::new();
    for (label, accent) in PALETTE {
        let s = spec(Candidate::BandNeutral, label, accent);
        cells.push(Cell {
            caption: format!("A / {label}"),
            canvas: render(&s, f, DARK).canvas,
        });
    }

    let mut long = spec(Candidate::BandNeutral, "corp-workstation.work", PALETTE[0].1);
    long.content_width = 520;
    cells.push(Cell {
        caption: "A / long canonical label".to_owned(),
        canvas: render(&long, f, DARK).canvas,
    });

    let light = spec(Candidate::BandNeutral, "Work", PALETTE[0].1);
    cells.push(Cell {
        caption: "A / light guest content".to_owned(),
        canvas: render(&light, f, LIGHT).canvas,
    });

    let mut scaled = spec(Candidate::BandNeutral, "Work", PALETTE[0].1);
    scaled.scale = 1.5;
    cells.push(Cell {
        caption: "A / scale 1.5".to_owned(),
        canvas: render(&scaled, f, DARK).canvas,
    });

    let mut big = spec(Candidate::BandNeutral, "Work", PALETTE[0].1);
    big.font_px = 28.0;
    cells.push(Cell {
        caption: "A / 200% text - band grows".to_owned(),
        canvas: render(&big, f, DARK).canvas,
    });
    cells
}

fn status_and_blocked(f: &TextRenderer) -> Vec<Cell> {
    let mut cells = Vec::new();
    for token in ["MIC", "MIC MUTED", "USB", "MIC . USB", "DEGRADED"] {
        let mut s = spec(Candidate::BandNeutral, "Work", PALETTE[0].1);
        s.status = Some(token.to_owned());
        cells.push(Cell {
            caption: format!("A / status {token}"),
            canvas: render(&s, f, DARK).canvas,
        });
    }
    let mut blocked = spec(Candidate::BandNeutral, "Work", PALETTE[0].1);
    blocked.identity_verified = false;
    cells.push(Cell {
        caption: "A / unverified - guest blocked".to_owned(),
        canvas: render(&blocked, f, DARK).canvas,
    });
    cells
}

/// The compact tab: collapsed, expanded with actions beside the name, and the
/// interaction states. This is the candidate after the wlcontrol restyle.
fn tab_states(f: &TextRenderer) -> Vec<Cell> {
    let mut cells = Vec::new();
    let tab = |label: &str, accent: &str| {
        let mut s = spec(Candidate::Tab, label, accent);
        s.content_width = 460;
        s.content_height = 120;
        s
    };

    cells.push(Cell {
        caption: "tab / collapsed".to_owned(),
        canvas: render(&tab("Work", PALETTE[0].1), f, DARK).canvas,
    });

    let mut expanded = tab("Work", PALETTE[0].1);
    expanded.expanded = true;
    cells.push(Cell {
        caption: "tab / expanded - actions beside the name".to_owned(),
        canvas: render(&expanded, f, DARK).canvas,
    });

    for (name, st) in [
        (
            "hover",
            VisualState {
                hover: true,
                ..VisualState::focused()
            },
        ),
        (
            "keyboard focus",
            VisualState {
                keyboard_focus: true,
                ..VisualState::focused()
            },
        ),
        ("unfocused", VisualState::default()),
    ] {
        let mut s = tab("Work", PALETTE[0].1);
        s.state = st;
        cells.push(Cell {
            caption: format!("tab / {name}"),
            canvas: render(&s, f, DARK).canvas,
        });
    }

    for (label, accent) in [PALETTE[1], PALETTE[3]] {
        cells.push(Cell {
            caption: format!("tab / {label}"),
            canvas: render(&tab(label, accent), f, DARK).canvas,
        });
    }

    let mut token = tab("Work", PALETTE[0].1);
    token.status = Some("MIC MUTED".to_owned());
    cells.push(Cell {
        caption: "tab / with capability token".to_owned(),
        canvas: render(&token, f, DARK).canvas,
    });

    cells.push(Cell {
        caption: "tab / grayscale".to_owned(),
        canvas: render(&tab("Work", PALETTE[0].1), f, DARK)
            .canvas
            .to_grayscale(),
    });

    cells.push(Cell {
        caption: "tab / light guest content".to_owned(),
        canvas: render(&tab("Media", PALETTE[2].1), f, LIGHT).canvas,
    });
    cells
}

/// The status token opens the same menu as identity, so it must show the same
/// states with the same delineation and focus guarantees.
fn status_token_states(f: &TextRenderer) -> Vec<Cell> {
    let states = [
        ("resting", VisualState::default()),
        (
            "hover",
            VisualState {
                hover: true,
                ..VisualState::focused()
            },
        ),
        (
            "pressed",
            VisualState {
                pressed: true,
                ..VisualState::focused()
            },
        ),
        (
            "menu open",
            VisualState {
                menu_open: true,
                ..VisualState::focused()
            },
        ),
        (
            "keyboard focus",
            VisualState {
                keyboard_focus: true,
                ..VisualState::focused()
            },
        ),
    ];
    let mut cells = Vec::new();
    for (name, st) in states {
        let mut s = spec(Candidate::BandNeutral, "Work", PALETTE[0].1);
        s.status = Some("MIC".to_owned());
        s.status_state = st;
        cells.push(Cell {
            caption: format!("token / {name}"),
            canvas: render(&s, f, DARK).canvas,
        });
    }
    // The same states without hue, since delineation must not depend on colour.
    for (name, st) in [states[1], states[4]] {
        let mut s = spec(Candidate::BandNeutral, "Work", PALETTE[0].1);
        s.status = Some("MIC".to_owned());
        s.status_state = st;
        cells.push(Cell {
            caption: format!("token / {name} in grayscale"),
            canvas: render(&s, f, DARK).canvas.to_grayscale(),
        });
    }
    cells
}

fn accessibility(f: &TextRenderer) -> Vec<Cell> {
    let mut cells = Vec::new();

    let base = spec(Candidate::BandNeutral, "Work", PALETTE[0].1);
    cells.push(Cell {
        caption: "A / grayscale display".to_owned(),
        canvas: render(&base, f, DARK).canvas.to_grayscale(),
    });

    cells.push(Cell {
        caption: "C / grayscale - accent fill".to_owned(),
        canvas: render(&spec(Candidate::AccentFill, "Work", PALETTE[0].1), f, DARK)
            .canvas
            .to_grayscale(),
    });

    let mut spaced = spec(Candidate::BandNeutral, "corp-workstation.work", PALETTE[0].1);
    spaced.content_width = 560;
    spaced.tracking_em = 0.12;
    cells.push(Cell {
        caption: "A / +0.12em letter spacing".to_owned(),
        canvas: render(&spaced, f, DARK).canvas,
    });

    // Two identities side by side in grayscale: the monochrome distinguishing
    // test that colour alone cannot pass.
    for (label, accent) in [PALETTE[0], PALETTE[3]] {
        cells.push(Cell {
            caption: format!("A / {label} in grayscale"),
            canvas: render(&spec(Candidate::BandNeutral, label, accent), f, DARK)
                .canvas
                .to_grayscale(),
        });
    }
    cells
}

/// Control C across accents that actually stress it, including the threshold
/// where auto-contrast text flips. Showing C only on two favourable accents
/// made it look better than it is.
fn accent_fill_stress(f: &TextRenderer) -> Vec<Cell> {
    let mut cells = Vec::new();
    for (name, accent) in STRESS_ACCENTS {
        let c = render(&spec(Candidate::AccentFill, "Work", accent), f, DARK);
        cells.push(Cell {
            caption: format!("C / {name} {accent} - text contrast {:.2}:1", c.label_contrast),
            canvas: c.canvas,
        });
        let a = render(&spec(Candidate::BandNeutral, "Work", accent), f, DARK);
        cells.push(Cell {
            caption: format!("A / {name} {accent} - text contrast {:.2}:1", a.label_contrast),
            canvas: a.canvas,
        });
    }
    // The same comparison without hue, where a fill has nothing left to offer.
    for (name, accent) in [STRESS_ACCENTS[0], STRESS_ACCENTS[1]] {
        cells.push(Cell {
            caption: format!("C / {name} in grayscale"),
            canvas: render(&spec(Candidate::AccentFill, "Work", accent), f, DARK)
                .canvas
                .to_grayscale(),
        });
        cells.push(Cell {
            caption: format!("A / {name} in grayscale"),
            canvas: render(&spec(Candidate::BandNeutral, "Work", accent), f, DARK)
                .canvas
                .to_grayscale(),
        });
    }
    cells
}

/// The combination the panel called failure-prone: a narrow window, a long
/// label, enlarged text, and a status token all at once. Captions are derived
/// from what actually happened, not from what was intended.
fn compound_reflow(f: &TextRenderer) -> Vec<Cell> {
    let mut cells = Vec::new();

    let describe = |r: &d2b_chrome_engine::variant::Rendered, prefix: &str| -> String {
        if r.blocked {
            return format!("{prefix} - fails closed, guest blocked");
        }
        match r.layout {
            Some(l) => {
                let mut notes = Vec::new();
                if l.reflow.grew_band {
                    notes.push(format!("band grew to {}px", l.band.height));
                }
                if l.reflow.status_second_row {
                    notes.push("token moved to second row".to_owned());
                } else if l.status.is_some() {
                    notes.push("token inline".to_owned());
                }
                if notes.is_empty() {
                    prefix.to_owned()
                } else {
                    format!("{prefix} - {}", notes.join(", "))
                }
            }
            None => prefix.to_owned(),
        }
    };

    let mut narrow = spec(Candidate::BandNeutral, "Work", PALETTE[0].1);
    narrow.content_width = 240;
    narrow.status = Some("MIC MUTED".to_owned());
    let r = render(&narrow, f, DARK);
    cells.push(Cell {
        caption: describe(&r, "A / narrow window"),
        canvas: r.canvas,
    });

    let mut squeezed = spec(Candidate::BandNeutral, "Work", PALETTE[0].1);
    squeezed.content_width = 150;
    squeezed.status = Some("MIC MUTED".to_owned());
    let r = render(&squeezed, f, DARK);
    cells.push(Cell {
        caption: describe(&r, "A / very narrow window"),
        canvas: r.canvas,
    });

    let mut long_big = spec(Candidate::BandNeutral, "corp-workstation.work", PALETTE[0].1);
    long_big.content_width = 620;
    long_big.font_px = 28.0;
    let r = render(&long_big, f, DARK);
    cells.push(Cell {
        caption: describe(&r, "A / long label at 200% text"),
        canvas: r.canvas,
    });

    let mut everything = spec(Candidate::BandNeutral, "corp-workstation.work", PALETTE[0].1);
    everything.content_width = 620;
    everything.font_px = 28.0;
    everything.status = Some("MIC . USB".to_owned());
    everything.tracking_em = 0.12;
    let r = render(&everything, f, DARK);
    cells.push(Cell {
        caption: describe(&r, "A / long + 200% + spacing + token"),
        canvas: r.canvas,
    });

    let mut tiny = spec(Candidate::BandNeutral, "Work", PALETTE[0].1);
    tiny.content_width = 150;
    tiny.font_px = 28.0;
    tiny.status = Some("USB".to_owned());
    let r = render(&tiny, f, DARK);
    cells.push(Cell {
        caption: describe(&r, "A / very narrow at 200% text"),
        canvas: r.canvas,
    });

    // Narrow enough that even the compact name cannot fit: the terminal case.
    let mut impossible = spec(Candidate::BandNeutral, "corp-workstation.work", PALETTE[0].1);
    impossible.content_width = 90;
    impossible.font_px = 28.0;
    let r = render(&impossible, f, DARK);
    cells.push(Cell {
        caption: describe(&r, "A / identity cannot fit"),
        canvas: r.canvas,
    });
    cells
}

/// Tile cells into one labelled sheet on a desktop-like background.
fn compose(f: &TextRenderer, title: &str, cells: &[Cell]) -> Canvas {
    const PAD: usize = 20;
    const CAPTION_H: usize = 22;
    const TITLE_H: usize = 34;

    let cols = 2.min(cells.len().max(1));
    let cell_w = cells.iter().map(|c| c.canvas.width).max().unwrap_or(1);
    let cell_h = cells.iter().map(|c| c.canvas.height).max().unwrap_or(1);
    let rows = cells.len().div_ceil(cols);

    let w = PAD + cols * (cell_w + PAD);
    let h = TITLE_H + PAD + rows * (cell_h + CAPTION_H + PAD);
    let mut sheet = Canvas::new(w, h, DESKTOP);

    let fg = Rgba::rgb(0xe8, 0xe8, 0xef);
    let dim = Rgba::rgb(0xb4, 0xb4, 0xc2);

    for g in f.layout(title, 17.0, 0.0, PAD as i32, 24) {
        blend_glyph(&mut sheet.pixels, w, h, &g, fg);
    }

    for (i, cell) in cells.iter().enumerate() {
        let cx = PAD + (i % cols) * (cell_w + PAD);
        let cy = TITLE_H + PAD + (i / cols) * (cell_h + CAPTION_H + PAD);
        sheet.draw(&cell.canvas, cx as i32, cy as i32);
        let by = (cy + cell.canvas.height + 15) as i32;
        for g in f.layout(&cell.caption, 12.0, 0.0, cx as i32, by) {
            blend_glyph(&mut sheet.pixels, w, h, &g, dim);
        }
    }
    sheet
}
