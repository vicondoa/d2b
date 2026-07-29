//! Prototype rendering engine for the d2b window identity tab.
//!
//! This crate is ADR 0047 exploration code. It deliberately lives outside the
//! `packages/` workspace so prototype dependencies never reach the shipping
//! Rust, deny, or audit gates.
//!
//! The parts worth promoting into the real proxy are the ones with tests:
//! WCAG-correct colour selection, horizontal scale-aware text, and the layout
//! that keeps the pointer input region off the window edges.

pub mod canvas;
pub mod skia;
pub mod tab;
pub mod vectext;
pub mod color;
pub mod geom;
pub mod parts;
pub mod text;
pub mod variant;

/// Font bundled for prototyping only; the ADR specifies fontconfig resolution.
pub const PROTOTYPE_FONT: &[u8] = include_bytes!("../assets/font.ttf");
