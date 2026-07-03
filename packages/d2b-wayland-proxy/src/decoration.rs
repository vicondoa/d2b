//! Proxy-owned Wayland decoration state and drawing helpers.
//!
//! The drawing path only consumes resolved configuration, surface dimensions,
//! and the authenticated VM label. Guest wl_buffers/dma-bufs remain opaque
//! Wayland objects and are never read or sampled here.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::CString,
    fs::File,
    io::{self, Write},
    os::fd::OwnedFd,
    rc::{Rc, Weak},
    str::FromStr,
};

use nix::sys::memfd::{MemFdCreateFlag, memfd_create};
use wl_proxy::{
    fixed::Fixed,
    object::{ConcreteObject, ObjectCoreApi},
    protocols::{
        ObjectInterface,
        wayland::{
            wl_buffer::{WlBuffer, WlBufferHandler},
            wl_compositor::WlCompositor,
            wl_output::WlOutputTransform,
            wl_registry::WlRegistry,
            wl_shm::WlShmFormat,
            wl_shm_pool::WlShmPool,
            wl_subcompositor::WlSubcompositor,
            wl_subsurface::WlSubsurface,
            wl_surface::WlSurface,
        },
        xdg_shell::{
            xdg_surface::{XdgSurface, XdgSurfaceHandler},
            xdg_toplevel::{XdgToplevel, XdgToplevelHandler, XdgToplevelState},
            xdg_wm_base::{XdgWmBase, XdgWmBaseHandler},
        },
    },
};

use crate::diag::{DiagRateLimiter, bounded_error_detail};

pub const DEFAULT_BORDER_THICKNESS: u32 = 4;
pub const WRAPPER_RAIL_WIDTH: u32 = 9;
const MAX_LABEL_CHARS: usize = 64;
const BYTES_PER_PIXEL: u32 = 4;
const MAX_DECORATION_DIMENSION: u32 = 16_384;
const MAX_DECORATION_BUFFER_BYTES: usize = 64 * 1024 * 1024;
const MAX_LABEL_FALLBACK_WIDTH: u32 = 1024;
const LABEL_PAD_X: u32 = 5;
const LABEL_PAD_Y: u32 = 3;
const GLYPH_W: u32 = 5;
const GLYPH_H: u32 = 7;
const GLYPH_ADVANCE: u32 = 6;
const MIN_LABEL_BAND_HEIGHT: u32 = LABEL_PAD_Y + GLYPH_H;
const VERTICAL_LABEL_X_SCALE: u32 = 1;
const VERTICAL_LABEL_Y_SCALE: u32 = 2;
const VERTICAL_LABEL_SIDE_PAD: u32 = 1;
const VERTICAL_LABEL_END_PAD: u32 = 8;
const MAX_RETIRED_DECORATION_BUFFERS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    fn argb8888_bytes(self) -> [u8; 4] {
        [self.b, self.g, self.r, self.a]
    }

    fn readable_text_color(self) -> Self {
        let luminance = u32::from(self.r) * 299 + u32::from(self.g) * 587 + u32::from(self.b) * 114;
        if luminance > 128_000 {
            Self::BLACK
        } else {
            Self::WHITE
        }
    }
}

impl FromStr for Color {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_hex_color(s)
    }
}

pub fn parse_hex_color(input: &str) -> Result<Color, String> {
    let value = input
        .strip_prefix('#')
        .ok_or_else(|| format!("expected #rrggbb color, got `{input}`"))?;
    if value.len() != 6 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("expected #rrggbb color, got `{input}`"));
    }
    let r = u8::from_str_radix(&value[0..2], 16)
        .map_err(|_| format!("invalid red channel in `{input}`"))?;
    let g = u8::from_str_radix(&value[2..4], 16)
        .map_err(|_| format!("invalid green channel in `{input}`"))?;
    let b = u8::from_str_radix(&value[4..6], 16)
        .map_err(|_| format!("invalid blue channel in `{input}`"))?;
    Ok(Color::rgb(r, g, b))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelPosition {
    TopLeft,
    TopCenter,
}

impl FromStr for LabelPosition {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "top-left" => Ok(Self::TopLeft),
            "top-center" => Ok(Self::TopCenter),
            _ => Err(format!("expected top-left|top-center, got `{value}`")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizedLabel(String);

impl SanitizedLabel {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

pub fn sanitize_label(input: &str) -> Option<SanitizedLabel> {
    let mut out = String::new();
    let mut pending_space = false;
    for ch in input.chars() {
        if out.chars().count() >= MAX_LABEL_CHARS {
            break;
        }
        if ch.is_control() {
            pending_space = true;
            continue;
        }
        if ch.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space && !out.is_empty() {
            out.push(' ');
        }
        pending_space = false;
        if ch.is_ascii_graphic() {
            out.push(ch);
        } else {
            out.push('?');
        }
    }
    let out = out.trim().to_owned();
    if out.is_empty() {
        None
    } else {
        Some(SanitizedLabel(out))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BorderConfig {
    pub enabled: bool,
    pub active: Color,
    pub inactive: Color,
    pub urgent: Color,
    pub thickness: u32,
    pub label: Option<SanitizedLabel>,
    pub label_position: LabelPosition,
}

impl Default for BorderConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            active: Color::rgb(60, 170, 255),
            inactive: Color::rgb(95, 95, 95),
            urgent: Color::rgb(255, 86, 86),
            thickness: DEFAULT_BORDER_THICKNESS,
            label: None,
            label_position: LabelPosition::TopLeft,
        }
    }
}

impl BorderConfig {
    pub fn enabled(&self) -> bool {
        self.enabled && self.thickness > 0
    }

    fn color_for_state(&self, state: VisualState) -> Color {
        if state.urgent {
            self.urgent
        } else if state.active {
            self.active
        } else {
            self.inactive
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BorderGeometry {
    pub content: Size,
    pub outer: Size,
    pub content_origin: Point,
    pub thickness: u32,
    pub top_thickness: u32,
}

impl BorderGeometry {
    pub fn expand(content: Size, thickness: u32) -> Option<Self> {
        Self::expand_with_label_band(content, thickness, false)
    }

    pub fn expand_with_label_band(
        content: Size,
        thickness: u32,
        label_present: bool,
    ) -> Option<Self> {
        if content.is_empty() || thickness == 0 {
            return None;
        }
        let horizontal = thickness.checked_mul(2)?;
        let top_thickness = top_border_height(thickness, label_present);
        let vertical = top_thickness.checked_add(thickness)?;
        Some(Self {
            content,
            outer: Size::new(
                content.width.checked_add(horizontal)?,
                content.height.checked_add(vertical)?,
            ),
            content_origin: Point {
                x: i32::try_from(thickness).ok()?,
                y: i32::try_from(top_thickness).ok()?,
            },
            thickness,
            top_thickness,
        })
    }

    fn label_fallback_for_oversized(content: Size, thickness: u32) -> Option<Self> {
        if content.is_empty() || thickness == 0 {
            return None;
        }
        let horizontal = thickness.checked_mul(2)?;
        let top_thickness = top_border_height(thickness, true);
        let outer_width = content
            .width
            .checked_add(horizontal)?
            .min(MAX_LABEL_FALLBACK_WIDTH)
            .max(horizontal.checked_add(GLYPH_W)?);
        Some(Self {
            content,
            outer: Size::new(outer_width, top_thickness.checked_add(thickness)?),
            content_origin: Point {
                x: i32::try_from(thickness).ok()?,
                y: i32::try_from(top_thickness).ok()?,
            },
            thickness,
            top_thickness,
        })
    }
}

fn top_border_height(thickness: u32, label_present: bool) -> u32 {
    if label_present {
        thickness.max(MIN_LABEL_BAND_HEIGHT)
    } else {
        thickness
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl WindowGeometry {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigureSize {
    pub width: i32,
    pub height: i32,
}

impl ConfigureSize {
    pub const fn new(width: i32, height: i32) -> Self {
        Self { width, height }
    }
}

pub fn expand_window_geometry_for_border(
    geometry: WindowGeometry,
    thickness: u32,
) -> Option<WindowGeometry> {
    expand_window_geometry_for_decoration(geometry, thickness, false)
}

fn expand_window_geometry_for_decoration(
    geometry: WindowGeometry,
    thickness: u32,
    label_present: bool,
) -> Option<WindowGeometry> {
    if geometry.width <= 0 || geometry.height <= 0 || thickness == 0 {
        return None;
    }
    let thickness = i32::try_from(thickness).ok()?;
    let top_thickness = i32::try_from(top_border_height(
        u32::try_from(thickness).ok()?,
        label_present,
    ))
    .ok()?;
    let horizontal = thickness.checked_mul(2)?;
    let vertical = top_thickness.checked_add(thickness)?;
    Some(WindowGeometry {
        x: geometry.x.checked_sub(thickness)?,
        y: geometry.y.checked_sub(top_thickness)?,
        width: geometry.width.checked_add(horizontal)?,
        height: geometry.height.checked_add(vertical)?,
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct VisualState {
    pub active: bool,
    pub urgent: bool,
    pub fullscreen: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecorationPlan {
    pub geometry: BorderGeometry,
    pub color: Color,
    pub label: Option<SanitizedLabel>,
    pub label_position: LabelPosition,
}

pub fn decoration_plan(
    config: &BorderConfig,
    content: Size,
    state: VisualState,
) -> Option<DecorationPlan> {
    if !config.enabled() || state.fullscreen {
        return None;
    }
    let mut geometry =
        BorderGeometry::expand_with_label_band(content, config.thickness, config.label.is_some())?;
    if decoration_buffer_layout(geometry.outer).is_none() {
        if config.label.is_some() {
            geometry = BorderGeometry::label_fallback_for_oversized(content, config.thickness)?;
            decoration_buffer_layout(geometry.outer)?;
        } else {
            return None;
        }
    }
    Some(DecorationPlan {
        geometry,
        color: config.color_for_state(state),
        label: config.label.clone(),
        label_position: config.label_position,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawInput {
    pub geometry: BorderGeometry,
    pub color: Color,
    pub label: Option<SanitizedLabel>,
    pub label_position: LabelPosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DecorationBufferLayout {
    width: usize,
    height: usize,
    stride: usize,
    len: usize,
}

fn decoration_buffer_layout(size: Size) -> Option<DecorationBufferLayout> {
    if size.is_empty() {
        return None;
    }
    let width = usize::try_from(size.width).ok()?;
    let height = usize::try_from(size.height).ok()?;
    let stride = width.checked_mul(BYTES_PER_PIXEL as usize)?;
    let len = stride.checked_mul(height)?;
    if size.width > MAX_DECORATION_DIMENSION || size.height > MAX_DECORATION_DIMENSION {
        return None;
    }
    if len > MAX_DECORATION_BUFFER_BYTES {
        return None;
    }
    Some(DecorationBufferLayout {
        width,
        height,
        stride,
        len,
    })
}

pub fn draw_decoration(input: &DrawInput) -> Option<Vec<u8>> {
    let layout = decoration_buffer_layout(input.geometry.outer)?;
    let mut pixels = vec![0_u8; layout.len];
    fill_border(
        &mut pixels,
        layout.width,
        layout.height,
        input.geometry.thickness as usize,
        input.geometry.top_thickness as usize,
        input.color,
    );
    if let Some(label) = &input.label {
        draw_label(
            &mut pixels,
            layout.width,
            layout.height,
            input.geometry.top_thickness,
            input.color,
            label.as_str(),
            input.label_position,
        );
    }
    Some(pixels)
}

fn draw_wrapper_rail(
    width: u32,
    height: u32,
    color: Color,
    label: Option<&SanitizedLabel>,
) -> Option<Vec<u8>> {
    let size = Size::new(width, height);
    let layout = decoration_buffer_layout(size)?;
    let mut pixels = vec![0_u8; layout.len];
    let color_bytes = color.argb8888_bytes();
    for y in 0..layout.height {
        for x in 0..layout.width.min(WRAPPER_RAIL_WIDTH as usize) {
            set_pixel(&mut pixels, layout.width, x, y, color_bytes);
        }
    }
    if let Some(label) = label {
        draw_vertical_label(
            &mut pixels,
            layout.width,
            layout.height,
            WRAPPER_RAIL_WIDTH,
            color,
            label.as_str(),
        );
    }
    Some(pixels)
}

fn fill_border(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    thickness: usize,
    top_thickness: usize,
    color: Color,
) {
    let color = color.argb8888_bytes();
    for y in 0..height {
        for x in 0..width {
            if x < thickness
                || y < top_thickness
                || x >= width.saturating_sub(thickness)
                || y >= height.saturating_sub(thickness)
            {
                set_pixel(pixels, width, x, y, color);
            }
        }
    }
}

fn draw_vertical_label(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    rail_width: u32,
    border: Color,
    label: &str,
) {
    let scale_x = VERTICAL_LABEL_X_SCALE as usize;
    let scale_y = VERTICAL_LABEL_Y_SCALE as usize;
    let rail_width = rail_width as usize;
    let glyph_width = GLYPH_H as usize * scale_x;
    let glyph_height = GLYPH_W as usize * scale_y;
    let advance = glyph_height + scale_y;
    if rail_width < glyph_width + (VERTICAL_LABEL_SIDE_PAD as usize * 2) || height < advance {
        return;
    }
    let end_pad = VERTICAL_LABEL_END_PAD as usize;
    let available_height = height.saturating_sub(end_pad * 2);
    let max_chars = if available_height < glyph_height {
        0
    } else {
        1 + (available_height - glyph_height) / advance
    };
    if max_chars == 0 {
        return;
    }
    let text: String = label.chars().take(max_chars).collect();
    let glyph_count = text.chars().count();
    let total_height = glyph_height + glyph_count.saturating_sub(1).saturating_mul(advance);
    let x = VERTICAL_LABEL_SIDE_PAD as usize;
    let mut y = height
        .saturating_sub(total_height)
        .saturating_div(2)
        .max(end_pad);
    let foreground = border.readable_text_color().argb8888_bytes();
    for ch in text.chars() {
        draw_rotated_glyph(
            pixels,
            width,
            height,
            RotatedGlyph {
                x,
                y,
                rows: glyph(ch),
                scale_x,
                scale_y,
                color: foreground,
            },
        );
        y = y.saturating_add(advance);
    }
}

struct RotatedGlyph {
    x: usize,
    y: usize,
    rows: [u8; 7],
    scale_x: usize,
    scale_y: usize,
    color: [u8; 4],
}

fn draw_rotated_glyph(pixels: &mut [u8], width: usize, height: usize, glyph: RotatedGlyph) {
    let RotatedGlyph {
        x,
        y,
        rows,
        scale_x,
        scale_y,
        color,
    } = glyph;
    for (row, bits) in rows.iter().enumerate() {
        for col in 0..GLYPH_W as usize {
            if bits & (1 << (GLYPH_W as usize - 1 - col)) == 0 {
                continue;
            }
            let px = x + (GLYPH_H as usize - 1 - row) * scale_x;
            let py = y + col * scale_y;
            for dy in 0..scale_y {
                for dx in 0..scale_x {
                    let sx = px + dx;
                    let sy = py + dy;
                    if sx < width && sy < height {
                        set_pixel(pixels, width, sx, sy, color);
                    }
                }
            }
        }
    }
}

fn draw_label(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    top_thickness: u32,
    border: Color,
    label: &str,
    position: LabelPosition,
) {
    let max_label_width = width.saturating_sub((LABEL_PAD_X * 2) as usize);
    if max_label_width < GLYPH_W as usize || top_thickness < MIN_LABEL_BAND_HEIGHT {
        return;
    }
    let label_height = height.min(top_thickness as usize);
    let glyphs_that_fit = ((max_label_width as u32) / GLYPH_ADVANCE).max(1) as usize;
    let text: String = label.chars().take(glyphs_that_fit).collect();
    let text_width = text.chars().count() as u32 * GLYPH_ADVANCE;
    let x = match position {
        LabelPosition::TopLeft => LABEL_PAD_X,
        LabelPosition::TopCenter => input_center(width as u32, text_width),
    } as usize;
    let y = LABEL_PAD_Y as usize;
    let foreground = border.readable_text_color().argb8888_bytes();
    let shadow = Color {
        a: 160,
        ..Color::BLACK
    }
    .argb8888_bytes();
    draw_text_pixels(pixels, width, label_height, x + 1, y + 1, &text, shadow);
    draw_text_pixels(pixels, width, label_height, x, y, &text, foreground);
}

fn input_center(width: u32, text_width: u32) -> u32 {
    width.saturating_sub(text_width) / 2
}

fn draw_text_pixels(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    text: &str,
    color: [u8; 4],
) {
    let mut cursor = x;
    for ch in text.chars() {
        draw_glyph(pixels, width, height, cursor, y, glyph(ch), color);
        cursor = cursor.saturating_add(GLYPH_ADVANCE as usize);
    }
}

fn draw_glyph(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    rows: [u8; 7],
    color: [u8; 4],
) {
    for (row, bits) in rows.iter().enumerate() {
        for col in 0..GLYPH_W as usize {
            if bits & (1 << (GLYPH_W as usize - 1 - col)) != 0 {
                let px = x + col;
                let py = y + row;
                if px < width && py < height {
                    set_pixel(pixels, width, px, py, color);
                }
            }
        }
    }
}

fn set_pixel(pixels: &mut [u8], width: usize, x: usize, y: usize, color: [u8; 4]) {
    let offset = (y * width + x) * BYTES_PER_PIXEL as usize;
    if offset + 4 <= pixels.len() {
        pixels[offset..offset + 4].copy_from_slice(&color);
    }
}

fn glyph(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'Y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110,
        ],
        '6' => [
            0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110,
        ],
        '-' => [0, 0, 0, 0b11111, 0, 0, 0],
        '_' => [0, 0, 0, 0, 0, 0, 0b11111],
        '.' => [0, 0, 0, 0, 0, 0b01100, 0b01100],
        ':' => [0, 0b01100, 0b01100, 0, 0b01100, 0b01100, 0],
        '/' => [
            0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000,
        ],
        '[' => [
            0b01110, 0b01000, 0b01000, 0b01000, 0b01000, 0b01000, 0b01110,
        ],
        ']' => [
            0b01110, 0b00010, 0b00010, 0b00010, 0b00010, 0b00010, 0b01110,
        ],
        ' ' => [0, 0, 0, 0, 0, 0, 0],
        _ => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0, 0b00100],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferDimensions {
    pub width: i32,
    pub height: i32,
}

impl BufferDimensions {
    pub fn surface_size(self, scale: i32, transform: WlOutputTransform) -> Option<Size> {
        if self.width <= 0 || self.height <= 0 || scale <= 0 {
            return None;
        }
        let (width, height) = if transform.0 % 2 == 1 {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        };
        Some(Size::new(
            u32::try_from(width / scale).ok()?,
            u32::try_from(height / scale).ok()?,
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ViewportSource {
    width: Fixed,
    height: Fixed,
}

impl ViewportSource {
    fn surface_size(self) -> Option<Size> {
        Some(Size::new(
            positive_integral_fixed_to_u32(self.width)?,
            positive_integral_fixed_to_u32(self.height)?,
        ))
    }
}

#[derive(Debug, Default)]
struct ViewportState {
    pending_source: Option<Option<ViewportSource>>,
    pending_destination: Option<Option<Size>>,
    current_source: Option<ViewportSource>,
    current_destination: Option<Size>,
}

impl ViewportState {
    fn set_source(&mut self, x: Fixed, y: Fixed, width: Fixed, height: Fixed) {
        if let Some(source) = viewport_source_from_request(x, y, width, height) {
            self.pending_source = Some(source);
        }
    }

    fn set_destination(&mut self, width: i32, height: i32) {
        if let Some(destination) = viewport_destination_from_request(width, height) {
            self.pending_destination = Some(destination);
        }
    }

    fn destroy(&mut self) {
        self.pending_source = Some(None);
        self.pending_destination = Some(None);
    }

    fn apply_pending(&mut self) {
        if let Some(source) = self.pending_source.take() {
            self.current_source = source;
        }
        if let Some(destination) = self.pending_destination.take() {
            self.current_destination = destination;
        }
    }

    fn has_pending(&self) -> bool {
        self.pending_source.is_some() || self.pending_destination.is_some()
    }
}

fn positive_integral_fixed_to_u32(value: Fixed) -> Option<u32> {
    let raw = value.to_wire();
    if raw <= 0 || raw % 256 != 0 {
        return None;
    }
    u32::try_from(raw / 256).ok()
}

fn viewport_source_from_request(
    x: Fixed,
    y: Fixed,
    width: Fixed,
    height: Fixed,
) -> Option<Option<ViewportSource>> {
    let unset = Fixed::from_i32_saturating(-1);
    if x == unset && y == unset && width == unset && height == unset {
        return Some(None);
    }
    if x < Fixed::ZERO || y < Fixed::ZERO || width <= Fixed::ZERO || height <= Fixed::ZERO {
        return None;
    }
    Some(Some(ViewportSource { width, height }))
}

fn viewport_destination_from_request(width: i32, height: i32) -> Option<Option<Size>> {
    if width == -1 && height == -1 {
        return Some(None);
    }
    if width <= 0 || height <= 0 {
        return None;
    }
    Some(Some(Size::new(
        u32::try_from(width).ok()?,
        u32::try_from(height).ok()?,
    )))
}

#[derive(Debug)]
struct SurfaceState {
    pending_buffer: Option<Option<u64>>,
    current_buffer: Option<u64>,
    pending_scale: Option<i32>,
    pending_transform: Option<WlOutputTransform>,
    current_scale: i32,
    current_transform: WlOutputTransform,
    viewport: ViewportState,
    current_size: Option<Size>,
    pending_window_geometry: Option<WindowGeometry>,
    current_window_geometry: Option<WindowGeometry>,
    toplevel: bool,
    visual: VisualState,
    decoration: Option<DecorationSurface>,
    wrapper: Option<WrapperToplevel>,
}

impl Default for SurfaceState {
    fn default() -> Self {
        Self {
            pending_buffer: None,
            current_buffer: None,
            pending_scale: None,
            pending_transform: None,
            current_scale: 1,
            current_transform: WlOutputTransform::NORMAL,
            viewport: ViewportState::default(),
            current_size: None,
            pending_window_geometry: None,
            current_window_geometry: None,
            toplevel: false,
            visual: VisualState::default(),
            decoration: None,
            wrapper: None,
        }
    }
}

impl SurfaceState {
    fn committed_size(&self, buffers: &HashMap<u64, BufferDimensions>) -> Option<Size> {
        self.committed_size_with_fallback(buffers, None)
    }

    fn committed_size_with_fallback(
        &self,
        buffers: &HashMap<u64, BufferDimensions>,
        fallback: Option<Size>,
    ) -> Option<Size> {
        let buffer_id = self.current_buffer?;
        if let Some(destination) = self.viewport.current_destination {
            return Some(destination);
        }
        if let Some(source_size) = self
            .viewport
            .current_source
            .and_then(ViewportSource::surface_size)
        {
            return Some(source_size);
        }
        buffers
            .get(&buffer_id)
            .and_then(|dims| dims.surface_size(self.current_scale, self.current_transform))
            .or(fallback)
    }

    fn apply_commit_state(&mut self, buffers: &HashMap<u64, BufferDimensions>) {
        let previous_size = self.current_size;
        let pending_buffer = self.pending_buffer.take();
        let pending_viewport = self.viewport.has_pending();
        if let Some(scale) = self.pending_scale.take() {
            self.current_scale = scale;
        }
        if let Some(transform) = self.pending_transform.take() {
            self.current_transform = transform;
        }
        if let Some(pending) = pending_buffer {
            self.current_buffer = pending;
        }
        self.viewport.apply_pending();
        self.current_size = if matches!(pending_buffer, Some(None)) {
            None
        } else if pending_buffer.is_some() || pending_viewport {
            self.committed_size(buffers)
        } else {
            self.committed_size_with_fallback(buffers, previous_size)
        };
        if let Some(geometry) = self.pending_window_geometry.take() {
            self.current_window_geometry = Some(geometry);
        }
    }

    fn effective_window_geometry(&self) -> Option<WindowGeometry> {
        if let Some(geometry) = self.current_window_geometry
            && geometry.width > 0
            && geometry.height > 0
        {
            return Some(geometry);
        }
        let size = self.current_size?;
        Some(WindowGeometry::new(
            0,
            0,
            i32::try_from(size.width).ok()?,
            i32::try_from(size.height).ok()?,
        ))
    }
}

#[derive(Debug)]
struct DecorationSurface {
    surface: Rc<WlSurface>,
    subsurface: Rc<WlSubsurface>,
    buffer: Option<ProxyDecorationBuffer>,
    // The compositor may still scan out an attached decoration buffer until it
    // sends wl_buffer.release. Keep released-aware retired buffers instead of
    // destroying every replacement immediately, but cap the queue so a guest
    // cannot force unbounded proxy memfd retention with rapid geometry changes.
    retired_buffers: Vec<ProxyDecorationBuffer>,
    key: Option<FrameKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WrapperGeometry {
    rail_width: u32,
    content: Size,
    outer: Size,
    guest_offset: Point,
}

impl WrapperGeometry {
    fn from_window_geometry(geometry: WindowGeometry) -> Option<Self> {
        if geometry.width <= 0 || geometry.height <= 0 {
            return None;
        }
        let content = Size::new(
            u32::try_from(geometry.width).ok()?,
            u32::try_from(geometry.height).ok()?,
        );
        let outer = Size::new(
            content.width.checked_add(WRAPPER_RAIL_WIDTH)?,
            content.height,
        );
        Some(Self {
            rail_width: WRAPPER_RAIL_WIDTH,
            content,
            outer,
            guest_offset: Point {
                x: i32::try_from(WRAPPER_RAIL_WIDTH)
                    .ok()?
                    .checked_sub(geometry.x)?,
                y: geometry.y.checked_neg()?,
            },
        })
    }
}

#[derive(Debug)]
struct WrapperToplevel {
    wrapper_surface: Rc<WlSurface>,
    wrapper_xdg_surface: Rc<XdgSurface>,
    wrapper_toplevel: Rc<XdgToplevel>,
    guest_surface: Rc<WlSurface>,
    guest_xdg_surface: Rc<XdgSurface>,
    guest_toplevel: Rc<XdgToplevel>,
    guest_subsurface: Rc<WlSubsurface>,
    buffer: Option<ProxyDecorationBuffer>,
    retired_buffers: Vec<ProxyDecorationBuffer>,
    key: Option<FrameKey>,
    applied_geometry: Option<WrapperGeometry>,
    pending_window_geometry: Option<WindowGeometry>,
    current_window_geometry: Option<WindowGeometry>,
    current_configure: ConfigureSize,
}

impl WrapperToplevel {
    fn retire_buffer(&mut self, buffer: ProxyDecorationBuffer) {
        buffer.retire();
        if !buffer.destroyed() {
            self.retired_buffers.push(buffer);
        }
        self.prune_destroyed_buffers();
        while self.retired_buffers.len() > MAX_RETIRED_DECORATION_BUFFERS {
            let buffer = self.retired_buffers.remove(0);
            buffer.force_destroy();
        }
    }

    fn force_destroy_buffers(&mut self) {
        if let Some(buffer) = self.buffer.take() {
            buffer.force_destroy();
        }
        for buffer in self.retired_buffers.drain(..) {
            buffer.force_destroy();
        }
    }

    fn prune_destroyed_buffers(&mut self) {
        self.retired_buffers.retain(|buffer| !buffer.destroyed());
    }
}

impl DecorationSurface {
    fn retire_buffer(&mut self, buffer: ProxyDecorationBuffer) {
        buffer.retire();
        if !buffer.destroyed() {
            self.retired_buffers.push(buffer);
        }
        self.prune_destroyed_buffers();
        while self.retired_buffers.len() > MAX_RETIRED_DECORATION_BUFFERS {
            let buffer = self.retired_buffers.remove(0);
            buffer.force_destroy();
        }
    }

    fn force_destroy_buffers(&mut self) {
        if let Some(buffer) = self.buffer.take() {
            buffer.force_destroy();
        }
        for buffer in self.retired_buffers.drain(..) {
            buffer.force_destroy();
        }
    }

    fn prune_destroyed_buffers(&mut self) {
        self.retired_buffers.retain(|buffer| !buffer.destroyed());
    }
}

#[derive(Debug)]
struct ProxyDecorationBuffer {
    buffer: Rc<WlBuffer>,
    lifecycle: Rc<DecorationBufferLifecycle>,
}

impl ProxyDecorationBuffer {
    fn new(buffer: Rc<WlBuffer>) -> Self {
        let lifecycle = Rc::new(DecorationBufferLifecycle::default());
        buffer.set_handler(DecorationBufferHandler {
            lifecycle: Rc::downgrade(&lifecycle),
        });
        Self { buffer, lifecycle }
    }

    fn wl_buffer(&self) -> &Rc<WlBuffer> {
        &self.buffer
    }

    fn retire(&self) {
        retire_proxy_decoration_buffer(&self.buffer, &self.lifecycle);
    }

    fn force_destroy(&self) {
        force_destroy_proxy_decoration_buffer(&self.buffer, &self.lifecycle);
    }

    fn destroyed(&self) -> bool {
        self.lifecycle.destroyed.get()
    }
}

#[derive(Debug, Default)]
struct DecorationBufferLifecycle {
    // Wayland clients should not destroy an attached wl_buffer until the
    // compositor sends release. The proxy owns these buffers, so it tracks
    // release/retire/destroy separately and destroys only once the buffer is
    // both retired and released, unless the bounded retired-buffer queue must
    // force cleanup for resource-exhaustion protection.
    released: Cell<bool>,
    retired: Cell<bool>,
    destroyed: Cell<bool>,
}

struct DecorationBufferHandler {
    lifecycle: Weak<DecorationBufferLifecycle>,
}

impl WlBufferHandler for DecorationBufferHandler {
    fn handle_release(&mut self, slf: &Rc<WlBuffer>) {
        if let Some(lifecycle) = self.lifecycle.upgrade() {
            release_proxy_decoration_buffer(slf, &lifecycle);
        }
    }
}

trait ProxyDecorationBufferDestroy {
    fn send_destroy_request(&self);
    fn delete_proxy_id(&self);
}

impl ProxyDecorationBufferDestroy for Rc<WlBuffer> {
    fn send_destroy_request(&self) {
        self.send_destroy();
    }

    fn delete_proxy_id(&self) {
        self.delete_id();
    }
}

fn retire_proxy_decoration_buffer(
    buffer: &impl ProxyDecorationBufferDestroy,
    lifecycle: &DecorationBufferLifecycle,
) {
    lifecycle.retired.set(true);
    if lifecycle.released.get() {
        destroy_proxy_decoration_buffer(buffer, lifecycle);
    }
}

fn release_proxy_decoration_buffer(
    buffer: &impl ProxyDecorationBufferDestroy,
    lifecycle: &DecorationBufferLifecycle,
) {
    lifecycle.released.set(true);
    if lifecycle.retired.get() {
        destroy_proxy_decoration_buffer(buffer, lifecycle);
    }
}

fn force_destroy_proxy_decoration_buffer(
    buffer: &impl ProxyDecorationBufferDestroy,
    lifecycle: &DecorationBufferLifecycle,
) {
    lifecycle.retired.set(true);
    destroy_proxy_decoration_buffer(buffer, lifecycle);
}

fn destroy_proxy_decoration_buffer(
    buffer: &impl ProxyDecorationBufferDestroy,
    lifecycle: &DecorationBufferLifecycle,
) {
    if lifecycle.destroyed.replace(true) {
        return;
    }
    buffer.send_destroy_request();
    buffer.delete_proxy_id();
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FrameKey {
    outer: Size,
    color: Color,
    label: Option<SanitizedLabel>,
    label_position: LabelPosition,
}

#[derive(Debug)]
pub struct DecorationManager {
    config: BorderConfig,
    diag: Rc<RefCell<DiagRateLimiter>>,
    compositor: Option<Rc<WlCompositor>>,
    subcompositor: Option<Rc<WlSubcompositor>>,
    shm: Option<Rc<wl_proxy::protocols::wayland::wl_shm::WlShm>>,
    wm_base: Option<Rc<XdgWmBase>>,
    buffers: HashMap<u64, BufferDimensions>,
    surfaces: HashMap<u64, SurfaceState>,
    subsurfaces_by_parent: HashMap<u64, Vec<Weak<WlSurface>>>,
}

impl DecorationManager {
    pub fn new(config: BorderConfig, diag: Rc<RefCell<DiagRateLimiter>>) -> Self {
        Self {
            config,
            diag,
            compositor: None,
            subcompositor: None,
            shm: None,
            wm_base: None,
            buffers: HashMap::new(),
            surfaces: HashMap::new(),
            subsurfaces_by_parent: HashMap::new(),
        }
    }

    pub fn enabled(&self) -> bool {
        self.config.enabled()
    }

    pub fn translate_window_geometry(
        &self,
        surface: &Rc<WlSurface>,
        geometry: WindowGeometry,
    ) -> WindowGeometry {
        self.translate_window_geometry_for_surface_id(surface.unique_id(), geometry)
    }

    fn translate_window_geometry_for_surface_id(
        &self,
        surface_id: u64,
        geometry: WindowGeometry,
    ) -> WindowGeometry {
        if !self.config.enabled() {
            return geometry;
        }
        let Some(state) = self.surfaces.get(&surface_id) else {
            return geometry;
        };
        if !state.toplevel || state.visual.fullscreen {
            return geometry;
        }
        expand_window_geometry_for_decoration(
            geometry,
            self.config.thickness,
            self.config.label.is_some(),
        )
        .unwrap_or(geometry)
    }

    pub fn observe_global(
        &mut self,
        registry: &Rc<WlRegistry>,
        name: u32,
        interface: ObjectInterface,
        version: u32,
    ) {
        if !self.enabled() {
            return;
        }
        match interface {
            ObjectInterface::WlCompositor if self.compositor.is_none() => {
                let compositor = registry
                    .state()
                    .create_object::<WlCompositor>(version.min(WlCompositor::XML_VERSION));
                registry.send_bind(name, compositor.clone());
                self.compositor = Some(compositor);
            }
            ObjectInterface::WlSubcompositor if self.subcompositor.is_none() => {
                let subcompositor = registry
                    .state()
                    .create_object::<WlSubcompositor>(version.min(WlSubcompositor::XML_VERSION));
                registry.send_bind(name, subcompositor.clone());
                self.subcompositor = Some(subcompositor);
            }
            ObjectInterface::WlShm if self.shm.is_none() => {
                let shm = registry
                    .state()
                    .create_object::<wl_proxy::protocols::wayland::wl_shm::WlShm>(
                        version.min(wl_proxy::protocols::wayland::wl_shm::WlShm::XML_VERSION),
                    );
                registry.send_bind(name, shm.clone());
                self.shm = Some(shm);
            }
            ObjectInterface::XdgWmBase if self.wm_base.is_none() => {
                let wm_base = registry
                    .state()
                    .create_object::<XdgWmBase>(version.min(XdgWmBase::XML_VERSION));
                registry.send_bind(name, wm_base.clone());
                wm_base.set_handler(ProxyWmBaseHandler);
                self.wm_base = Some(wm_base);
            }
            _ => {}
        }
    }

    pub fn register_surface(&mut self, surface: &Rc<WlSurface>) {
        if self.enabled() {
            self.surfaces.entry(surface.unique_id()).or_default();
        }
    }

    pub fn mark_toplevel(&mut self, surface: &Rc<WlSurface>) {
        if let Some(state) = self.surfaces.get_mut(&surface.unique_id()) {
            state.toplevel = true;
        }
    }

    pub fn create_wrapper_toplevel(
        &mut self,
        surface: &Rc<WlSurface>,
        guest_xdg_surface: &Rc<XdgSurface>,
        guest_toplevel: &Rc<XdgToplevel>,
        manager: Weak<RefCell<DecorationManager>>,
    ) -> bool {
        if !self.enabled() {
            return false;
        }
        let Some(compositor) = self.compositor.as_ref() else {
            return false;
        };
        let Some(subcompositor) = self.subcompositor.as_ref() else {
            return false;
        };
        let Some(wm_base) = self.wm_base.as_ref() else {
            return false;
        };

        let wrapper_surface = compositor.new_send_create_surface();
        wrapper_surface.set_forward_to_client(false);
        let wrapper_xdg_surface = wm_base.new_send_get_xdg_surface(&wrapper_surface);
        wrapper_xdg_surface.set_forward_to_client(false);
        let wrapper_toplevel = wrapper_xdg_surface.new_send_get_toplevel();
        wrapper_toplevel.set_forward_to_client(false);
        let guest_subsurface = subcompositor.new_send_get_subsurface(surface, &wrapper_surface);
        guest_subsurface.set_forward_to_client(false);
        guest_subsurface.send_set_position(i32::try_from(WRAPPER_RAIL_WIDTH).unwrap_or(0), 0);
        guest_subsurface.send_place_below(&wrapper_surface);
        guest_subsurface.send_set_desync();
        wrapper_surface.send_commit();

        guest_xdg_surface.set_forward_to_server(false);
        guest_toplevel.set_forward_to_server(false);

        let surface_id = surface.unique_id();
        wrapper_xdg_surface.set_handler(WrapperXdgSurfaceHandler {
            manager: manager.clone(),
            surface_id,
        });
        wrapper_toplevel.set_handler(WrapperToplevelHandler {
            manager,
            surface_id,
        });

        let state = self.surfaces.entry(surface_id).or_default();
        state.toplevel = true;
        state.wrapper = Some(WrapperToplevel {
            wrapper_surface,
            wrapper_xdg_surface,
            wrapper_toplevel,
            guest_surface: surface.clone(),
            guest_xdg_surface: guest_xdg_surface.clone(),
            guest_toplevel: guest_toplevel.clone(),
            guest_subsurface,
            buffer: None,
            retired_buffers: Vec::new(),
            key: None,
            applied_geometry: None,
            pending_window_geometry: None,
            current_window_geometry: None,
            current_configure: ConfigureSize::new(0, 0),
        });
        true
    }

    pub fn has_wrapper(&self, surface_id: u64) -> bool {
        self.surfaces
            .get(&surface_id)
            .and_then(|state| state.wrapper.as_ref())
            .is_some()
    }

    pub fn wrapper_set_title(&mut self, surface_id: u64, title: &str) -> bool {
        let Some(wrapper) = self
            .surfaces
            .get_mut(&surface_id)
            .and_then(|state| state.wrapper.as_mut())
        else {
            return false;
        };
        wrapper.wrapper_toplevel.send_set_title(title);
        true
    }

    pub fn wrapper_set_app_id(&mut self, surface_id: u64, app_id: &str) -> bool {
        let Some(wrapper) = self
            .surfaces
            .get_mut(&surface_id)
            .and_then(|state| state.wrapper.as_mut())
        else {
            return false;
        };
        wrapper.wrapper_toplevel.send_set_app_id(app_id);
        true
    }

    pub fn wrapper_set_window_geometry(
        &mut self,
        surface_id: u64,
        geometry: WindowGeometry,
    ) -> bool {
        let Some(state) = self.surfaces.get_mut(&surface_id) else {
            return false;
        };
        if state.wrapper.is_none() {
            return false;
        }
        state.pending_window_geometry = Some(geometry);
        if let Some(wrapper) = state.wrapper.as_mut() {
            wrapper.pending_window_geometry = Some(geometry);
        }
        true
    }

    pub fn wrapper_ack_configure(&mut self, surface_id: u64, serial: u32) -> bool {
        let Some(wrapper) = self
            .surfaces
            .get_mut(&surface_id)
            .and_then(|state| state.wrapper.as_mut())
        else {
            return false;
        };
        wrapper.wrapper_xdg_surface.send_ack_configure(serial);
        true
    }

    pub fn wrapper_set_fullscreen(
        &mut self,
        surface_id: u64,
        output: Option<&Rc<wl_proxy::protocols::wayland::wl_output::WlOutput>>,
    ) -> bool {
        let Some(wrapper) = self
            .surfaces
            .get_mut(&surface_id)
            .and_then(|state| state.wrapper.as_mut())
        else {
            return false;
        };
        wrapper.wrapper_toplevel.send_set_fullscreen(output);
        true
    }

    pub fn wrapper_unset_fullscreen(&mut self, surface_id: u64) -> bool {
        let Some(wrapper) = self
            .surfaces
            .get_mut(&surface_id)
            .and_then(|state| state.wrapper.as_mut())
        else {
            return false;
        };
        wrapper.wrapper_toplevel.send_unset_fullscreen();
        true
    }

    pub fn wrapper_set_maximized(&mut self, surface_id: u64, maximized: bool) -> bool {
        let Some(wrapper) = self
            .surfaces
            .get_mut(&surface_id)
            .and_then(|state| state.wrapper.as_mut())
        else {
            return false;
        };
        if maximized {
            wrapper.wrapper_toplevel.send_set_maximized();
        } else {
            wrapper.wrapper_toplevel.send_unset_maximized();
        }
        true
    }

    pub fn wrapper_set_minimized(&mut self, surface_id: u64) -> bool {
        let Some(wrapper) = self
            .surfaces
            .get_mut(&surface_id)
            .and_then(|state| state.wrapper.as_mut())
        else {
            return false;
        };
        wrapper.wrapper_toplevel.send_set_minimized();
        true
    }

    pub fn wrapper_set_min_size(&mut self, surface_id: u64, width: i32, height: i32) -> bool {
        let Some(wrapper) = self
            .surfaces
            .get_mut(&surface_id)
            .and_then(|state| state.wrapper.as_mut())
        else {
            return false;
        };
        let width = if width > 0 {
            width.saturating_add(i32::try_from(WRAPPER_RAIL_WIDTH).unwrap_or(0))
        } else {
            width
        };
        wrapper.wrapper_toplevel.send_set_min_size(width, height);
        true
    }

    pub fn wrapper_set_max_size(&mut self, surface_id: u64, width: i32, height: i32) -> bool {
        let Some(wrapper) = self
            .surfaces
            .get_mut(&surface_id)
            .and_then(|state| state.wrapper.as_mut())
        else {
            return false;
        };
        let width = if width > 0 {
            width.saturating_add(i32::try_from(WRAPPER_RAIL_WIDTH).unwrap_or(0))
        } else {
            width
        };
        wrapper.wrapper_toplevel.send_set_max_size(width, height);
        true
    }

    pub fn wrapper_input_target(&self, surface: &Rc<WlSurface>) -> Option<Rc<WlSurface>> {
        let wrapper_id = surface.unique_id();
        self.surfaces.values().find_map(|state| {
            state
                .wrapper
                .as_ref()
                .filter(|wrapper| wrapper.wrapper_surface.unique_id() == wrapper_id)
                .map(|wrapper| wrapper.guest_surface.clone())
        })
    }

    pub fn wrapper_xdg_surface_for_guest(
        &self,
        xdg_surface: &Rc<XdgSurface>,
    ) -> Option<Rc<XdgSurface>> {
        let guest_id = xdg_surface.unique_id();
        self.surfaces.values().find_map(|state| {
            state
                .wrapper
                .as_ref()
                .filter(|wrapper| wrapper.guest_xdg_surface.unique_id() == guest_id)
                .map(|wrapper| wrapper.wrapper_xdg_surface.clone())
        })
    }

    fn wrapper_handle_toplevel_configure(
        &mut self,
        surface_id: u64,
        width: i32,
        height: i32,
        states: &[u8],
    ) {
        let Some(state) = self.surfaces.get_mut(&surface_id) else {
            return;
        };
        state.visual.fullscreen = xdg_state_contains(states, XdgToplevelState::FULLSCREEN.0);
        state.visual.active = xdg_state_contains(states, XdgToplevelState::ACTIVATED.0);
        let rail = if state.visual.fullscreen {
            0
        } else {
            i32::try_from(WRAPPER_RAIL_WIDTH).unwrap_or(0)
        };
        let content_width = if width > 0 {
            width.saturating_sub(rail).max(1)
        } else {
            0
        };
        if let Some(wrapper) = state.wrapper.as_mut() {
            wrapper.current_configure = ConfigureSize::new(width, height);
            wrapper
                .guest_toplevel
                .send_configure(content_width, height, states);
        }
    }

    fn wrapper_handle_surface_configure(&mut self, surface_id: u64, serial: u32) {
        let Some(wrapper) = self
            .surfaces
            .get_mut(&surface_id)
            .and_then(|state| state.wrapper.as_mut())
        else {
            return;
        };
        wrapper.guest_xdg_surface.send_configure(serial);
    }

    fn wrapper_handle_close(&mut self, surface_id: u64) {
        let Some(wrapper) = self
            .surfaces
            .get_mut(&surface_id)
            .and_then(|state| state.wrapper.as_mut())
        else {
            return;
        };
        wrapper.guest_toplevel.send_close();
    }

    pub fn record_buffer(&mut self, buffer: &Rc<WlBuffer>, width: i32, height: i32) {
        if self.enabled() && width > 0 && height > 0 {
            self.buffers
                .insert(buffer.unique_id(), BufferDimensions { width, height });
        }
    }

    pub fn remove_buffer(&mut self, buffer: &Rc<WlBuffer>) {
        self.buffers.remove(&buffer.unique_id());
    }

    pub fn surface_attach(&mut self, surface: &Rc<WlSurface>, buffer: Option<&Rc<WlBuffer>>) {
        if let Some(state) = self.surfaces.get_mut(&surface.unique_id()) {
            state.pending_buffer = Some(buffer.map(|buffer| buffer.unique_id()));
        }
    }

    pub fn surface_set_buffer_scale(&mut self, surface: &Rc<WlSurface>, scale: i32) {
        if let Some(state) = self.surfaces.get_mut(&surface.unique_id()) {
            state.pending_scale = Some(scale);
        }
    }

    pub fn surface_set_buffer_transform(
        &mut self,
        surface: &Rc<WlSurface>,
        transform: WlOutputTransform,
    ) {
        if let Some(state) = self.surfaces.get_mut(&surface.unique_id()) {
            state.pending_transform = Some(transform);
        }
    }

    pub fn surface_get_viewport(&mut self, surface: &Rc<WlSurface>) {
        if self.enabled() {
            self.surfaces.entry(surface.unique_id()).or_default();
        }
    }

    pub fn surface_viewport_destroyed(&mut self, surface: &Rc<WlSurface>) {
        if let Some(state) = self.surfaces.get_mut(&surface.unique_id()) {
            state.viewport.destroy();
        }
    }

    pub fn surface_set_viewport_source(
        &mut self,
        surface: &Rc<WlSurface>,
        x: Fixed,
        y: Fixed,
        width: Fixed,
        height: Fixed,
    ) {
        if let Some(state) = self.surfaces.get_mut(&surface.unique_id()) {
            state.viewport.set_source(x, y, width, height);
        }
    }

    pub fn surface_set_viewport_destination(
        &mut self,
        surface: &Rc<WlSurface>,
        width: i32,
        height: i32,
    ) {
        if let Some(state) = self.surfaces.get_mut(&surface.unique_id()) {
            state.viewport.set_destination(width, height);
        }
    }

    pub fn surface_commit(&mut self, surface: &Rc<WlSurface>) {
        let surface_id = surface.unique_id();
        let Some(state) = self.surfaces.get_mut(&surface_id) else {
            return;
        };
        state.apply_commit_state(&self.buffers);
        if !state.toplevel {
            return;
        }
        if state.wrapper.is_some() {
            self.apply_wrapper(surface_id);
            return;
        }
        let plan = state
            .current_size
            .and_then(|size| decoration_plan(&self.config, size, state.visual));
        match plan {
            Some(plan) => self.apply_decoration(surface, surface_id, plan),
            None => self.remove_decoration(surface_id),
        }
    }

    fn apply_wrapper(&mut self, surface_id: u64) {
        let (wrapper_geometry, key, needs_buffer) = {
            let Some(state) = self.surfaces.get_mut(&surface_id) else {
                return;
            };
            let base_geometry = state.effective_window_geometry();
            let visual = state.visual;
            let Some(wrapper) = state.wrapper.as_mut() else {
                return;
            };
            if let Some(pending) = wrapper.pending_window_geometry.take() {
                wrapper.current_window_geometry = Some(pending);
            }
            let geometry = wrapper
                .current_window_geometry
                .or(base_geometry)
                .filter(|geometry| geometry.width > 0 && geometry.height > 0);
            let Some(geometry) = geometry else {
                return;
            };
            let wrapper_geometry = if visual.fullscreen {
                WrapperGeometry {
                    rail_width: 0,
                    content: Size::new(
                        u32::try_from(geometry.width).unwrap_or(1),
                        u32::try_from(geometry.height).unwrap_or(1),
                    ),
                    outer: Size::new(
                        u32::try_from(geometry.width).unwrap_or(1),
                        u32::try_from(geometry.height).unwrap_or(1),
                    ),
                    guest_offset: Point {
                        x: geometry.x.saturating_neg(),
                        y: geometry.y.saturating_neg(),
                    },
                }
            } else {
                match WrapperGeometry::from_window_geometry(geometry) {
                    Some(geometry) => geometry,
                    None => return,
                }
            };
            let key = FrameKey {
                outer: Size::new(wrapper_geometry.rail_width, wrapper_geometry.outer.height),
                color: self.config.color_for_state(visual),
                label: if visual.fullscreen {
                    None
                } else {
                    self.config.label.clone()
                },
                label_position: self.config.label_position,
            };
            let needs_buffer = wrapper.key.as_ref() != Some(&key);
            if !needs_buffer && wrapper.applied_geometry == Some(wrapper_geometry) {
                return;
            }
            (wrapper_geometry, key, needs_buffer)
        };
        let new_buffer = if needs_buffer && wrapper_geometry.rail_width > 0 {
            match self.create_wrapper_rail_buffer(
                wrapper_geometry.rail_width,
                wrapper_geometry.outer.height,
                key.color,
                key.label.as_ref(),
            ) {
                Ok(buffer) => Some(buffer),
                Err(error) => {
                    let error = bounded_error_detail(error.to_string());
                    self.diag
                        .borrow_mut()
                        .warn("wrapper-rail", "draw-failed", || {
                            format!(
                                "[d2b-wlproxy] event=wrapper-rail reason=draw-failed error={error}"
                            )
                        });
                    None
                }
            }
        } else {
            None
        };
        let compositor = self.compositor.clone();
        let Some(wrapper) = self
            .surfaces
            .get_mut(&surface_id)
            .and_then(|state| state.wrapper.as_mut())
        else {
            return;
        };
        wrapper.guest_subsurface.send_set_position(
            wrapper_geometry.guest_offset.x,
            wrapper_geometry.guest_offset.y,
        );
        if let Some(compositor) = &compositor {
            let region = compositor.new_send_create_region();
            if wrapper_geometry.rail_width > 0 {
                region.send_add(
                    0,
                    0,
                    i32::try_from(wrapper_geometry.rail_width).unwrap_or(i32::MAX),
                    i32::try_from(wrapper_geometry.outer.height).unwrap_or(i32::MAX),
                );
            }
            wrapper.wrapper_surface.send_set_input_region(Some(&region));
            region.send_destroy();
        }
        wrapper.wrapper_xdg_surface.send_set_window_geometry(
            0,
            0,
            i32::try_from(wrapper_geometry.outer.width).unwrap_or(i32::MAX),
            i32::try_from(wrapper_geometry.outer.height).unwrap_or(i32::MAX),
        );
        if wrapper_geometry.rail_width == 0 {
            wrapper.wrapper_surface.send_attach(None, 0, 0);
            wrapper.wrapper_surface.send_commit();
            if let Some(old_buffer) = wrapper.buffer.take() {
                wrapper.retire_buffer(old_buffer);
            }
            wrapper.key = Some(key);
            wrapper.applied_geometry = Some(wrapper_geometry);
            return;
        }
        if let Some(buffer) = new_buffer {
            let old_buffer = wrapper.buffer.take();
            wrapper
                .wrapper_surface
                .send_attach(Some(buffer.wl_buffer()), 0, 0);
            wrapper.wrapper_surface.send_damage_buffer(
                0,
                0,
                i32::try_from(wrapper_geometry.rail_width).unwrap_or(i32::MAX),
                i32::try_from(wrapper_geometry.outer.height).unwrap_or(i32::MAX),
            );
            wrapper.wrapper_surface.send_commit();
            if let Some(old_buffer) = old_buffer {
                wrapper.retire_buffer(old_buffer);
            } else {
                wrapper.prune_destroyed_buffers();
            }
            wrapper.buffer = Some(buffer);
            wrapper.key = Some(key);
            wrapper.applied_geometry = Some(wrapper_geometry);
        } else {
            wrapper.wrapper_surface.send_commit();
            wrapper.applied_geometry = Some(wrapper_geometry);
        }
    }

    pub fn surface_destroyed(&mut self, surface: &Rc<WlSurface>) {
        let id = surface.unique_id();
        self.remove_decoration(id);
        self.surfaces.remove(&id);
        self.subsurfaces_by_parent.remove(&id);
        for siblings in self.subsurfaces_by_parent.values_mut() {
            siblings.retain(|sibling| {
                sibling
                    .upgrade()
                    .is_some_and(|sibling| sibling.unique_id() != id)
            });
        }
    }

    pub fn toplevel_destroyed(&mut self, surface_id: u64) {
        self.remove_decoration(surface_id);
        if let Some(state) = self.surfaces.get_mut(&surface_id) {
            state.toplevel = false;
            state.pending_window_geometry = None;
            state.current_window_geometry = None;
        }
    }

    pub fn register_guest_subsurface(&mut self, surface: &Rc<WlSurface>, parent: &Rc<WlSurface>) {
        let parent_id = parent.unique_id();
        let siblings = self.subsurfaces_by_parent.entry(parent_id).or_default();
        if !siblings.iter().any(|sibling| {
            sibling
                .upgrade()
                .is_some_and(|sibling| sibling.unique_id() == surface.unique_id())
        }) {
            siblings.push(Rc::downgrade(surface));
        }
        self.raise_decoration_above_guest_subsurfaces(parent);
    }

    pub fn raise_decoration_above_guest_subsurfaces(&mut self, parent: &Rc<WlSurface>) {
        let parent_id = parent.unique_id();
        let Some(state) = self.surfaces.get_mut(&parent_id) else {
            return;
        };
        let Some(decoration) = state.decoration.as_mut() else {
            return;
        };
        decoration.subsurface.send_place_above(parent);
        if let Some(siblings) = self.subsurfaces_by_parent.get_mut(&parent_id) {
            siblings.retain(|sibling| sibling.strong_count() > 0);
            for sibling in siblings.iter().filter_map(Weak::upgrade) {
                decoration.subsurface.send_place_above(&sibling);
            }
        }
    }

    pub fn toplevel_configure(&mut self, surface_id: u64, states: &[u8]) {
        if let Some(state) = self.surfaces.get_mut(&surface_id) {
            state.visual.fullscreen = xdg_state_contains(states, XdgToplevelState::FULLSCREEN.0);
            state.visual.active = xdg_state_contains(states, XdgToplevelState::ACTIVATED.0);
        }
    }

    pub fn toplevel_fullscreen_request(&mut self, surface_id: u64, fullscreen: bool) {
        if let Some(state) = self.surfaces.get_mut(&surface_id) {
            state.visual.fullscreen = fullscreen;
        }
    }

    #[cfg(test)]
    pub fn set_urgent_for_tests(&mut self, surface_id: u64, urgent: bool) {
        if let Some(state) = self.surfaces.get_mut(&surface_id) {
            state.visual.urgent = urgent;
        }
    }

    fn apply_decoration(&mut self, parent: &Rc<WlSurface>, surface_id: u64, plan: DecorationPlan) {
        let key = FrameKey {
            outer: plan.geometry.outer,
            color: plan.color,
            label: plan.label.clone(),
            label_position: plan.label_position,
        };
        let needs_surface = self
            .surfaces
            .get(&surface_id)
            .and_then(|state| state.decoration.as_ref())
            .is_none();
        let new_decoration = if needs_surface {
            self.create_decoration_surface(parent)
        } else {
            None
        };
        let needs_buffer = self
            .surfaces
            .get(&surface_id)
            .and_then(|state| state.decoration.as_ref())
            .is_none_or(|decoration| decoration.key.as_ref() != Some(&key));
        let new_buffer = if needs_buffer {
            match self.create_buffer_for_plan(&plan) {
                Ok(buffer) => Some(buffer),
                Err(error) => {
                    let error = bounded_error_detail(error.to_string());
                    self.diag
                        .borrow_mut()
                        .warn("border-decoration", "draw-failed", || {
                            format!(
                                "[d2b-wlproxy] event=border-decoration reason=draw-failed error={error}"
                            )
                        });
                    None
                }
            }
        } else {
            None
        };
        let Some(state) = self.surfaces.get_mut(&surface_id) else {
            return;
        };
        if state.decoration.is_none() {
            state.decoration = new_decoration;
        }
        if let Some(decoration) = state.decoration.as_mut() {
            decoration.subsurface.send_set_position(
                -i32::try_from(plan.geometry.thickness).unwrap_or(i32::MAX),
                -i32::try_from(plan.geometry.top_thickness).unwrap_or(i32::MAX),
            );
        } else {
            return;
        }
        self.raise_decoration_above_guest_subsurfaces(parent);
        if let Some(buffer) = new_buffer {
            let Some(state) = self.surfaces.get_mut(&surface_id) else {
                return;
            };
            let Some(decoration) = state.decoration.as_mut() else {
                return;
            };
            let old_buffer = decoration.buffer.take();
            decoration
                .surface
                .send_attach(Some(buffer.wl_buffer()), 0, 0);
            decoration.surface.send_damage_buffer(
                0,
                0,
                plan.geometry.outer.width as i32,
                plan.geometry.outer.height as i32,
            );
            decoration.surface.send_commit();
            if let Some(old_buffer) = old_buffer {
                decoration.retire_buffer(old_buffer);
            } else {
                decoration.prune_destroyed_buffers();
            }
            decoration.buffer = Some(buffer);
            decoration.key = Some(key);
        }
    }

    fn create_decoration_surface(&self, parent: &Rc<WlSurface>) -> Option<DecorationSurface> {
        let compositor = self.compositor.as_ref()?;
        let subcompositor = self.subcompositor.as_ref()?;
        let surface = compositor.new_send_create_surface();
        let empty = compositor.new_send_create_region();
        surface.send_set_input_region(Some(&empty));
        empty.send_destroy();
        let subsurface = subcompositor.new_send_get_subsurface(&surface, parent);
        subsurface.send_set_desync();
        Some(DecorationSurface {
            surface,
            subsurface,
            buffer: None,
            retired_buffers: Vec::new(),
            key: None,
        })
    }

    fn create_buffer_for_plan(&self, plan: &DecorationPlan) -> io::Result<ProxyDecorationBuffer> {
        let shm = self
            .shm
            .as_ref()
            .ok_or_else(|| io::Error::other("wl_shm is not available"))?;
        let layout = decoration_buffer_layout(plan.geometry.outer)
            .ok_or_else(|| io::Error::other("border buffer exceeds decoration limits"))?;
        let input = DrawInput {
            geometry: plan.geometry,
            color: plan.color,
            label: plan.label.clone(),
            label_position: plan.label_position,
        };
        let pixels = draw_decoration(&input)
            .ok_or_else(|| io::Error::other("border buffer exceeds decoration limits"))?;
        let fd = create_memfd_with_contents(
            &pixels,
            u64::try_from(layout.len).map_err(|_| io::Error::other("border buffer too large"))?,
        )?;
        let fd = Rc::new(fd);
        let pool = shm.new_send_create_pool(
            &fd,
            i32::try_from(layout.len).map_err(|_| io::Error::other("border buffer too large"))?,
        );
        let buffer = pool.new_send_create_buffer(
            0,
            i32::try_from(layout.width).map_err(|_| io::Error::other("border width too large"))?,
            i32::try_from(layout.height)
                .map_err(|_| io::Error::other("border height too large"))?,
            i32::try_from(layout.stride)
                .map_err(|_| io::Error::other("border stride too large"))?,
            WlShmFormat::ARGB8888,
        );
        pool.send_destroy();
        Ok(ProxyDecorationBuffer::new(buffer))
    }

    fn create_wrapper_rail_buffer(
        &self,
        width: u32,
        height: u32,
        color: Color,
        label: Option<&SanitizedLabel>,
    ) -> io::Result<ProxyDecorationBuffer> {
        let shm = self
            .shm
            .as_ref()
            .ok_or_else(|| io::Error::other("wl_shm is not available"))?;
        let size = Size::new(width, height);
        let layout = decoration_buffer_layout(size)
            .ok_or_else(|| io::Error::other("wrapper rail buffer exceeds decoration limits"))?;
        let pixels = draw_wrapper_rail(width, height, color, label)
            .ok_or_else(|| io::Error::other("wrapper rail buffer exceeds decoration limits"))?;
        let fd = create_memfd_with_contents(
            &pixels,
            u64::try_from(layout.len)
                .map_err(|_| io::Error::other("wrapper rail buffer too large"))?,
        )?;
        let fd = Rc::new(fd);
        let pool = shm.new_send_create_pool(
            &fd,
            i32::try_from(layout.len)
                .map_err(|_| io::Error::other("wrapper rail buffer too large"))?,
        );
        let buffer = pool.new_send_create_buffer(
            0,
            i32::try_from(layout.width)
                .map_err(|_| io::Error::other("wrapper rail width too large"))?,
            i32::try_from(layout.height)
                .map_err(|_| io::Error::other("wrapper rail height too large"))?,
            i32::try_from(layout.stride)
                .map_err(|_| io::Error::other("wrapper rail stride too large"))?,
            WlShmFormat::ARGB8888,
        );
        pool.send_destroy();
        Ok(ProxyDecorationBuffer::new(buffer))
    }

    fn remove_decoration(&mut self, surface_id: u64) {
        let Some(state) = self.surfaces.get_mut(&surface_id) else {
            return;
        };
        if let Some(mut wrapper) = state.wrapper.take() {
            wrapper.wrapper_surface.send_attach(None, 0, 0);
            wrapper.wrapper_surface.send_commit();
            wrapper.force_destroy_buffers();
            wrapper.guest_subsurface.send_destroy();
            wrapper.wrapper_toplevel.send_destroy();
            wrapper.wrapper_xdg_surface.send_destroy();
            wrapper.wrapper_surface.send_destroy();
        }
        if let Some(mut decoration) = state.decoration.take() {
            decoration.surface.send_attach(None, 0, 0);
            decoration.surface.send_commit();
            decoration.force_destroy_buffers();
            decoration.subsurface.send_destroy();
            decoration.surface.send_destroy();
        }
    }
}

struct WrapperXdgSurfaceHandler {
    manager: Weak<RefCell<DecorationManager>>,
    surface_id: u64,
}

struct ProxyWmBaseHandler;

impl XdgWmBaseHandler for ProxyWmBaseHandler {
    fn handle_ping(&mut self, slf: &Rc<XdgWmBase>, serial: u32) {
        slf.send_pong(serial);
    }
}

impl XdgSurfaceHandler for WrapperXdgSurfaceHandler {
    fn handle_configure(&mut self, _slf: &Rc<XdgSurface>, serial: u32) {
        if let Some(manager) = self.manager.upgrade() {
            manager
                .borrow_mut()
                .wrapper_handle_surface_configure(self.surface_id, serial);
        }
    }
}

struct WrapperToplevelHandler {
    manager: Weak<RefCell<DecorationManager>>,
    surface_id: u64,
}

impl XdgToplevelHandler for WrapperToplevelHandler {
    fn handle_configure(&mut self, _slf: &Rc<XdgToplevel>, width: i32, height: i32, states: &[u8]) {
        if let Some(manager) = self.manager.upgrade() {
            manager.borrow_mut().wrapper_handle_toplevel_configure(
                self.surface_id,
                width,
                height,
                states,
            );
        }
    }

    fn handle_close(&mut self, _slf: &Rc<XdgToplevel>) {
        if let Some(manager) = self.manager.upgrade() {
            manager.borrow_mut().wrapper_handle_close(self.surface_id);
        }
    }
}

fn create_memfd_with_contents(contents: &[u8], size: u64) -> io::Result<OwnedFd> {
    let name = CString::new("d2b-wayland-border").expect("static memfd name has no nul");
    let fd = memfd_create(name.as_c_str(), MemFdCreateFlag::MFD_CLOEXEC)
        .map_err(|errno| io::Error::from_raw_os_error(errno as i32))?;
    let writer_fd = rustix::io::fcntl_dupfd_cloexec(&fd, 0).map_err(io::Error::from)?;
    let mut file = File::from(writer_fd);
    file.set_len(size)?;
    file.write_all(contents)?;
    Ok(fd)
}

fn xdg_state_contains(states: &[u8], needle: u32) -> bool {
    states
        .chunks_exact(4)
        .any(|chunk| u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) == needle)
}

pub type SharedDecorationManager = Rc<RefCell<DecorationManager>>;

pub fn tracking_buffer_handler(manager: &SharedDecorationManager) -> TrackingBufferHandler {
    TrackingBufferHandler {
        manager: Rc::downgrade(manager),
    }
}

pub struct TrackingBufferHandler {
    manager: Weak<RefCell<DecorationManager>>,
}

impl WlBufferHandler for TrackingBufferHandler {
    fn handle_destroy(&mut self, slf: &Rc<WlBuffer>) {
        if let Some(manager) = self.manager.upgrade() {
            manager.borrow_mut().remove_buffer(slf);
        }
        // Ordinary forwarded wl_buffer destruction uses wl-proxy's normal
        // client/server object lifetime. The explicit delete_id workaround below
        // is scoped to wl_shm_pool because wl-cross-domain-proxy can reuse pool
        // IDs immediately after destroy.
        slf.send_destroy();
    }
}

pub fn tracking_shm_pool_handler(manager: &SharedDecorationManager) -> TrackingShmPoolHandler {
    TrackingShmPoolHandler {
        manager: Rc::downgrade(manager),
    }
}

pub struct TrackingShmPoolHandler {
    manager: Weak<RefCell<DecorationManager>>,
}

trait TrackedShmPoolDestroy {
    fn send_destroy_request(&self);
    fn delete_proxy_id(&self);
}

impl TrackedShmPoolDestroy for Rc<WlShmPool> {
    fn send_destroy_request(&self) {
        self.send_destroy();
    }

    fn delete_proxy_id(&self) {
        self.delete_id();
    }
}

fn destroy_tracked_shm_pool(pool: &impl TrackedShmPoolDestroy) {
    // wl-cross-domain-proxy can reuse wl_shm_pool object IDs immediately after
    // destroy. Delete the proxy-side ID after forwarding destroy so the reused
    // ID is accepted instead of colliding with stale tracking state.
    pool.send_destroy_request();
    pool.delete_proxy_id();
}

impl wl_proxy::protocols::wayland::wl_shm_pool::WlShmPoolHandler for TrackingShmPoolHandler {
    fn handle_create_buffer(
        &mut self,
        slf: &Rc<WlShmPool>,
        id: &Rc<WlBuffer>,
        offset: i32,
        width: i32,
        height: i32,
        stride: i32,
        format: WlShmFormat,
    ) {
        if let Some(manager) = self.manager.upgrade() {
            id.set_handler(tracking_buffer_handler(&manager));
            manager.borrow_mut().record_buffer(id, width, height);
        }
        slf.send_create_buffer(id, offset, width, height, stride, format);
    }

    fn handle_destroy(&mut self, slf: &Rc<WlShmPool>) {
        destroy_tracked_shm_pool(slf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_diag() -> Rc<RefCell<DiagRateLimiter>> {
        Rc::new(RefCell::new(DiagRateLimiter::new("work".to_owned())))
    }

    #[test]
    fn parses_strict_hex_colors() {
        assert_eq!(
            "#1a2B3c".parse::<Color>().unwrap(),
            Color::rgb(0x1a, 0x2b, 0x3c)
        );
        assert!("1a2b3c".parse::<Color>().is_err());
        assert!("#12345".parse::<Color>().is_err());
        assert!("#12345x".parse::<Color>().is_err());
    }

    #[test]
    fn tracked_shm_pool_destroy_forwards_before_deleting_id() {
        #[derive(Default)]
        struct FakePool {
            calls: RefCell<Vec<&'static str>>,
        }

        impl TrackedShmPoolDestroy for FakePool {
            fn send_destroy_request(&self) {
                self.calls.borrow_mut().push("destroy");
            }

            fn delete_proxy_id(&self) {
                self.calls.borrow_mut().push("delete-id");
            }
        }

        let pool = FakePool::default();
        destroy_tracked_shm_pool(&pool);

        assert_eq!(*pool.calls.borrow(), ["destroy", "delete-id"]);
    }

    #[test]
    fn retired_decoration_buffer_waits_for_release_before_destroy() {
        #[derive(Default)]
        struct FakeBuffer {
            calls: RefCell<Vec<&'static str>>,
        }

        impl ProxyDecorationBufferDestroy for FakeBuffer {
            fn send_destroy_request(&self) {
                self.calls.borrow_mut().push("destroy");
            }

            fn delete_proxy_id(&self) {
                self.calls.borrow_mut().push("delete-id");
            }
        }

        let buffer = FakeBuffer::default();
        let lifecycle = DecorationBufferLifecycle::default();

        retire_proxy_decoration_buffer(&buffer, &lifecycle);
        assert!(buffer.calls.borrow().is_empty());

        release_proxy_decoration_buffer(&buffer, &lifecycle);
        assert_eq!(*buffer.calls.borrow(), ["destroy", "delete-id"]);

        release_proxy_decoration_buffer(&buffer, &lifecycle);
        force_destroy_proxy_decoration_buffer(&buffer, &lifecycle);
        assert_eq!(*buffer.calls.borrow(), ["destroy", "delete-id"]);
    }

    #[test]
    fn released_decoration_buffer_destroys_when_retired() {
        #[derive(Default)]
        struct FakeBuffer {
            calls: RefCell<Vec<&'static str>>,
        }

        impl ProxyDecorationBufferDestroy for FakeBuffer {
            fn send_destroy_request(&self) {
                self.calls.borrow_mut().push("destroy");
            }

            fn delete_proxy_id(&self) {
                self.calls.borrow_mut().push("delete-id");
            }
        }

        let buffer = FakeBuffer::default();
        let lifecycle = DecorationBufferLifecycle::default();

        release_proxy_decoration_buffer(&buffer, &lifecycle);
        assert!(buffer.calls.borrow().is_empty());

        retire_proxy_decoration_buffer(&buffer, &lifecycle);
        assert_eq!(*buffer.calls.borrow(), ["destroy", "delete-id"]);
    }

    #[test]
    fn force_destroy_decoration_buffer_uses_protocol_order_once() {
        #[derive(Default)]
        struct FakeBuffer {
            calls: RefCell<Vec<&'static str>>,
        }

        impl ProxyDecorationBufferDestroy for FakeBuffer {
            fn send_destroy_request(&self) {
                self.calls.borrow_mut().push("destroy");
            }

            fn delete_proxy_id(&self) {
                self.calls.borrow_mut().push("delete-id");
            }
        }

        let buffer = FakeBuffer::default();
        let lifecycle = DecorationBufferLifecycle::default();

        force_destroy_proxy_decoration_buffer(&buffer, &lifecycle);
        force_destroy_proxy_decoration_buffer(&buffer, &lifecycle);

        assert_eq!(*buffer.calls.borrow(), ["destroy", "delete-id"]);
    }

    #[test]
    fn geometry_expand_and_contract_offsets_content() {
        let expanded = BorderGeometry::expand(Size::new(640, 480), 4).unwrap();
        assert_eq!(expanded.outer, Size::new(648, 488));
        assert_eq!(expanded.content_origin, Point { x: 4, y: 4 });
        assert_eq!(expanded.top_thickness, 4);
    }

    #[test]
    fn wrapper_geometry_reserves_left_rail_without_moving_guest_content_vertically() {
        let geometry = WrapperGeometry::from_window_geometry(WindowGeometry::new(0, 0, 800, 600))
            .expect("valid wrapper geometry");

        assert_eq!(geometry.rail_width, WRAPPER_RAIL_WIDTH);
        assert_eq!(geometry.content, Size::new(800, 600));
        assert_eq!(geometry.outer, Size::new(809, 600));
        assert_eq!(geometry.guest_offset, Point { x: 9, y: 0 });
    }

    #[test]
    fn wrapper_rail_draws_only_proxy_owned_rail_pixels() {
        let label = sanitize_label("personal-dev");
        let pixels = draw_wrapper_rail(
            WRAPPER_RAIL_WIDTH,
            120,
            Color::rgb(0, 255, 0),
            label.as_ref(),
        )
        .expect("valid wrapper rail");
        let row_len = WRAPPER_RAIL_WIDTH as usize * BYTES_PER_PIXEL as usize;
        let green = Color::rgb(0, 255, 0).argb8888_bytes();

        assert_eq!(pixels.len(), row_len * 120);
        assert!(
            pixels
                .chunks_exact(row_len)
                .all(|row| { row.chunks_exact(4).any(|px| px == green) })
        );
    }

    #[test]
    fn label_band_keeps_default_side_border_but_makes_label_visible() {
        let config = BorderConfig {
            enabled: true,
            label: sanitize_label("work"),
            ..BorderConfig::default()
        };
        let plan = decoration_plan(&config, Size::new(640, 480), VisualState::default()).unwrap();

        assert_eq!(plan.geometry.thickness, DEFAULT_BORDER_THICKNESS);
        assert_eq!(plan.geometry.top_thickness, MIN_LABEL_BAND_HEIGHT);
        assert_eq!(
            plan.geometry.content_origin,
            Point {
                x: DEFAULT_BORDER_THICKNESS as i32,
                y: MIN_LABEL_BAND_HEIGHT as i32,
            }
        );
        assert_eq!(plan.geometry.outer, Size::new(648, 494));

        let pixels = draw_decoration(&DrawInput {
            geometry: plan.geometry,
            color: Color::BLACK,
            label: plan.label,
            label_position: plan.label_position,
        })
        .unwrap();

        assert!(
            pixels
                .chunks_exact(4)
                .any(|px| px == Color::WHITE.argb8888_bytes())
        );
    }

    #[test]
    fn oversized_labeled_surface_keeps_bounded_label_decoration() {
        let config = BorderConfig {
            enabled: true,
            label: sanitize_label("work"),
            ..BorderConfig::default()
        };
        let plan = decoration_plan(
            &config,
            Size::new(MAX_DECORATION_DIMENSION, MAX_DECORATION_DIMENSION),
            VisualState::default(),
        )
        .unwrap();

        assert!(plan.geometry.outer.width <= MAX_LABEL_FALLBACK_WIDTH);
        assert_eq!(
            plan.geometry.outer.height,
            MIN_LABEL_BAND_HEIGHT + DEFAULT_BORDER_THICKNESS
        );
        assert!(
            draw_decoration(&DrawInput {
                geometry: plan.geometry,
                color: Color::BLACK,
                label: plan.label,
                label_position: plan.label_position,
            })
            .is_some()
        );
    }

    #[test]
    fn window_geometry_expansion_covers_outer_border() {
        assert_eq!(
            expand_window_geometry_for_border(WindowGeometry::new(8, 9, 100, 50), 4),
            Some(WindowGeometry::new(4, 5, 108, 58))
        );
        assert_eq!(
            expand_window_geometry_for_decoration(WindowGeometry::new(8, 20, 100, 50), 4, true),
            Some(WindowGeometry::new(4, 10, 108, 64))
        );
    }

    #[test]
    fn window_geometry_expansion_rejects_invalid_or_overflowing_values() {
        assert_eq!(
            expand_window_geometry_for_border(WindowGeometry::new(0, 0, 0, 50), 4),
            None
        );
        assert_eq!(
            expand_window_geometry_for_border(WindowGeometry::new(i32::MIN, 0, 10, 10), 4),
            None
        );
        assert_eq!(
            expand_window_geometry_for_border(WindowGeometry::new(0, 0, i32::MAX, 10), 4),
            None
        );
    }

    #[test]
    fn manager_translates_window_geometry_for_decorated_toplevels_only() {
        let mut manager = DecorationManager::new(
            BorderConfig {
                enabled: true,
                thickness: 6,
                ..BorderConfig::default()
            },
            test_diag(),
        );
        let surface_id = 42;
        let geometry = WindowGeometry::new(10, 20, 300, 200);

        assert_eq!(
            manager.translate_window_geometry_for_surface_id(surface_id, geometry),
            geometry
        );

        manager.surfaces.insert(surface_id, SurfaceState::default());
        assert_eq!(
            manager.translate_window_geometry_for_surface_id(surface_id, geometry),
            geometry
        );

        manager.surfaces.get_mut(&surface_id).unwrap().toplevel = true;
        assert_eq!(
            manager.translate_window_geometry_for_surface_id(surface_id, geometry),
            WindowGeometry::new(4, 14, 312, 212)
        );

        manager
            .surfaces
            .get_mut(&surface_id)
            .unwrap()
            .visual
            .fullscreen = true;
        assert_eq!(
            manager.translate_window_geometry_for_surface_id(surface_id, geometry),
            geometry
        );
    }

    #[test]
    fn manager_translates_label_band_for_default_labeled_borders() {
        let mut manager = DecorationManager::new(
            BorderConfig {
                enabled: true,
                label: sanitize_label("work"),
                ..BorderConfig::default()
            },
            test_diag(),
        );
        let surface_id = 42;
        let geometry = WindowGeometry::new(10, 20, 300, 200);

        manager.surfaces.insert(surface_id, SurfaceState::default());
        manager.surfaces.get_mut(&surface_id).unwrap().toplevel = true;

        assert_eq!(
            manager.translate_window_geometry_for_surface_id(surface_id, geometry),
            WindowGeometry::new(6, 10, 308, 214)
        );
    }

    #[test]
    fn fullscreen_disables_decoration_plan() {
        let config = BorderConfig {
            enabled: true,
            thickness: 3,
            ..BorderConfig::default()
        };
        assert!(decoration_plan(&config, Size::new(100, 100), VisualState::default()).is_some());
        assert!(
            decoration_plan(
                &config,
                Size::new(100, 100),
                VisualState {
                    fullscreen: true,
                    ..VisualState::default()
                }
            )
            .is_none()
        );
    }

    #[test]
    fn decoration_plan_rejects_oversized_outer_dimensions() {
        let config = BorderConfig {
            enabled: true,
            thickness: 1,
            ..BorderConfig::default()
        };

        assert!(
            decoration_plan(
                &config,
                Size::new(MAX_DECORATION_DIMENSION, 100),
                VisualState::default()
            )
            .is_none()
        );
        assert!(
            decoration_plan(
                &config,
                Size::new(100, MAX_DECORATION_DIMENSION),
                VisualState::default()
            )
            .is_none()
        );
    }

    #[test]
    fn decoration_buffer_layout_enforces_allocation_bounds() {
        assert_eq!(
            decoration_buffer_layout(Size::new(4096, 4096)).map(|layout| layout.len),
            Some(MAX_DECORATION_BUFFER_BYTES)
        );
        assert!(decoration_buffer_layout(Size::new(4096, 4097)).is_none());
        assert!(decoration_buffer_layout(Size::new(u32::MAX, u32::MAX)).is_none());
    }

    #[test]
    fn draw_decoration_rejects_oversized_geometry_before_allocation() {
        let input = DrawInput {
            geometry: BorderGeometry {
                content: Size::new(u32::MAX, u32::MAX),
                outer: Size::new(u32::MAX, u32::MAX),
                content_origin: Point { x: 1, y: 1 },
                thickness: 1,
                top_thickness: 1,
            },
            color: Color::rgb(1, 2, 3),
            label: sanitize_label("work"),
            label_position: LabelPosition::TopLeft,
        };

        assert!(draw_decoration(&input).is_none());
    }

    #[test]
    fn label_sanitization_bounds_and_strips_controls() {
        let raw = format!("work\nvm\t{}", "x".repeat(100));
        let label = sanitize_label(&raw).unwrap();
        assert!(!label.as_str().contains('\n'));
        assert!(!label.as_str().contains('\t'));
        assert!(label.as_str().chars().count() <= MAX_LABEL_CHARS);
        assert_eq!(sanitize_label("\n\t"), None);
    }

    #[test]
    fn draw_input_contains_only_proxy_owned_render_data() {
        let input = DrawInput {
            geometry: BorderGeometry::expand(Size::new(10, 10), 2).unwrap(),
            color: Color::rgb(1, 2, 3),
            label: sanitize_label("work"),
            label_position: LabelPosition::TopLeft,
        };
        let pixels = draw_decoration(&input).unwrap();
        assert_eq!(pixels.len(), 14 * 14 * 4);
        assert!(
            pixels
                .chunks_exact(4)
                .any(|px| px == Color::rgb(1, 2, 3).argb8888_bytes())
        );
    }

    #[test]
    fn draw_path_takes_no_guest_buffer_or_fd_inputs() {
        fn accepts_only_render_metadata(_: DrawInput) {}

        accepts_only_render_metadata(DrawInput {
            geometry: BorderGeometry::expand(Size::new(32, 24), 4).unwrap(),
            color: Color::rgb(10, 20, 30),
            label: sanitize_label("work"),
            label_position: LabelPosition::TopCenter,
        });
    }

    #[test]
    fn buffer_dimensions_apply_scale_and_transform_without_reading_buffer() {
        let dims = BufferDimensions {
            width: 200,
            height: 100,
        };
        assert_eq!(
            dims.surface_size(2, WlOutputTransform::NORMAL),
            Some(Size::new(100, 50))
        );
        assert_eq!(
            dims.surface_size(2, WlOutputTransform::_90),
            Some(Size::new(50, 100))
        );
    }

    #[test]
    fn viewport_destination_then_source_then_buffer_size_priority() {
        let mut buffers = HashMap::new();
        buffers.insert(
            1,
            BufferDimensions {
                width: 400,
                height: 200,
            },
        );
        let mut state = SurfaceState {
            current_buffer: Some(1),
            current_scale: 2,
            viewport: ViewportState {
                current_source: Some(ViewportSource {
                    width: Fixed::from_i32_saturating(120),
                    height: Fixed::from_i32_saturating(80),
                }),
                current_destination: Some(Size::new(50, 40)),
                ..ViewportState::default()
            },
            ..SurfaceState::default()
        };

        assert_eq!(state.committed_size(&buffers), Some(Size::new(50, 40)));

        state.viewport.current_destination = None;
        assert_eq!(state.committed_size(&buffers), Some(Size::new(120, 80)));

        state.viewport.current_source = None;
        assert_eq!(state.committed_size(&buffers), Some(Size::new(200, 100)));
    }

    #[test]
    fn viewport_destroy_queues_reset_to_buffer_size() {
        let mut buffers = HashMap::new();
        buffers.insert(
            1,
            BufferDimensions {
                width: 300,
                height: 150,
            },
        );
        let mut state = SurfaceState {
            current_buffer: Some(1),
            viewport: ViewportState {
                current_source: Some(ViewportSource {
                    width: Fixed::from_i32_saturating(90),
                    height: Fixed::from_i32_saturating(60),
                }),
                current_destination: Some(Size::new(45, 30)),
                ..ViewportState::default()
            },
            ..SurfaceState::default()
        };

        assert_eq!(state.committed_size(&buffers), Some(Size::new(45, 30)));

        state.viewport.destroy();
        state.viewport.apply_pending();

        assert_eq!(state.viewport.current_source, None);
        assert_eq!(state.viewport.current_destination, None);
        assert_eq!(state.committed_size(&buffers), Some(Size::new(300, 150)));
    }

    #[test]
    fn destroyed_current_buffer_dimensions_preserve_last_committed_size() {
        let mut buffers = HashMap::new();
        buffers.insert(
            1,
            BufferDimensions {
                width: 400,
                height: 200,
            },
        );
        let mut state = SurfaceState {
            pending_buffer: Some(Some(1)),
            ..SurfaceState::default()
        };

        state.apply_commit_state(&buffers);
        assert_eq!(state.current_size, Some(Size::new(400, 200)));

        buffers.remove(&1);
        state.apply_commit_state(&buffers);

        assert_eq!(state.current_buffer, Some(1));
        assert_eq!(state.current_size, Some(Size::new(400, 200)));
    }

    #[test]
    fn committed_attach_none_clears_last_committed_size() {
        let mut buffers = HashMap::new();
        buffers.insert(
            1,
            BufferDimensions {
                width: 400,
                height: 200,
            },
        );
        let mut state = SurfaceState {
            current_buffer: Some(1),
            current_size: Some(Size::new(400, 200)),
            pending_buffer: Some(None),
            ..SurfaceState::default()
        };

        state.apply_commit_state(&buffers);

        assert_eq!(state.current_buffer, None);
        assert_eq!(state.current_size, None);
    }

    #[test]
    fn viewport_unset_requests_clear_pending_state() {
        let mut viewport = ViewportState {
            current_source: Some(ViewportSource {
                width: Fixed::from_i32_saturating(90),
                height: Fixed::from_i32_saturating(60),
            }),
            current_destination: Some(Size::new(45, 30)),
            ..ViewportState::default()
        };

        viewport.set_source(
            Fixed::from_i32_saturating(-1),
            Fixed::from_i32_saturating(-1),
            Fixed::from_i32_saturating(-1),
            Fixed::from_i32_saturating(-1),
        );
        viewport.set_destination(-1, -1);
        viewport.apply_pending();

        assert_eq!(viewport.current_source, None);
        assert_eq!(viewport.current_destination, None);
    }

    #[test]
    fn xdg_state_parser_detects_active_fullscreen() {
        let mut states = Vec::new();
        states.extend_from_slice(&XdgToplevelState::ACTIVATED.0.to_ne_bytes());
        states.extend_from_slice(&XdgToplevelState::FULLSCREEN.0.to_ne_bytes());
        assert!(xdg_state_contains(&states, XdgToplevelState::ACTIVATED.0));
        assert!(xdg_state_contains(&states, XdgToplevelState::FULLSCREEN.0));
        assert!(!xdg_state_contains(&states, 999));
    }
}
