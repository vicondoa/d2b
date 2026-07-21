//! Additive workspace-registration scaffold for the future systemd-user-scoped
//! persistent-shell supervisor (`shell-supervisor`; `d2b.shell.v2`; see
//! ADR 0045).
//!
//! This crate exists only to satisfy the `workspace-crate-registration-seam`
//! W8 external dependency: it registers the crate as a workspace member with
//! a buildable, dependency-free, behavior-free scaffold so the
//! `systemd-user-shell-routing` component can rebase onto a real crate
//! directory instead of creating one itself. No PTY, session-table, or
//! attach/detach behavior is implemented here; that design work belongs to
//! the `systemd-user-shell-routing` component.

#![forbid(unsafe_code)]
