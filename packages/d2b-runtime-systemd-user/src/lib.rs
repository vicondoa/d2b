//! Additive workspace-registration scaffold for the future systemd-user
//! runtime-agent socket surface (`d2b-runtime-systemd-user.socket`,
//! `d2b.runtime.systemd-user.v2`; see ADR 0045).
//!
//! This crate exists only to satisfy the `workspace-crate-registration-seam`
//! W8 external dependency: it registers the crate as a workspace member with
//! a buildable, dependency-free, behavior-free scaffold so the
//! `systemd-user-shell-routing` component can rebase onto a real crate
//! directory instead of creating one itself. No runtime behavior, wire
//! protocol, or API surface is implemented here; that design work belongs to
//! the `systemd-user-shell-routing` component.

#![forbid(unsafe_code)]
