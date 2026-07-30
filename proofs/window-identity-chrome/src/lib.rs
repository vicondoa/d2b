//! Proof for ADR 0047: window identity chrome.
//!
//! This crate carries the load-bearing logic of the design, isolated from the
//! Wayland plumbing so it can be reasoned about and tested directly:
//!
//! - [`geometry`] - band sizing, tab placement, and the pointer region, with
//!   the fail-closed contract that replaces "draw nothing and hope".
//! - [`parts`] - the single measured list that drawing and hit-testing share,
//!   and the configuration contract layered on top of it.
//! - [`contrast`] - WCAG relative luminance and the colour selection that the
//!   shipped proxy currently gets wrong.
//! - [`label`] - identity text sanitization and ellipsization.
//!
//! What is deliberately *not* here: pixel rendering, Wayland protocol
//! handling, and anything that needs a compositor. Those are exercised by the
//! prototype under `labs/window-chrome/`.

pub mod action;
pub mod contrast;
pub mod geometry;
pub mod label;
pub mod measure;
pub mod parts;
