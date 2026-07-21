//! Additive workspace-registration scaffold for the future per-user
//! systemd-user runtime agent process (see ADR 0045's renamed systemd-user
//! runtime agent and shell supervisor).
//!
//! This crate exists only to satisfy the `workspace-crate-registration-seam`
//! W8 external dependency: it registers the crate as a workspace member with
//! a buildable, dependency-free, behavior-free scaffold so the
//! `systemd-user-shell-routing` component can rebase onto a real crate
//! directory instead of creating one itself. No agent dispatch, socket, or
//! protocol behavior is implemented here; that design work belongs to the
//! `systemd-user-shell-routing` component.

#![forbid(unsafe_code)]
