//! Private site-runtime artifact (`site.json`).
//!
//! The NixOS site module resolves the host session facts the daemon must
//! never guess - today the host Wayland socket that
//! `d2b.site.waylandUser` + `d2b.site.waylandDisplay` describe - into this
//! artifact, so the trusted bundle and the runtime directory the site
//! provisions cannot disagree. The artifact is optional in the bundle index:
//! a bundle that predates it, or a site that declares no Wayland session,
//! leaves the value absent and the consuming launch refuses by name instead
//! of naming an invented path.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::Component;

/// Private site-runtime contract emitted by the NixOS site module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SiteJson {
    /// Artifact schema version (currently `"v1"`).
    pub schema_version: String,
    /// Absolute host Wayland socket (`/run/user/<uid>/<display>`), or `null`
    /// when the site declares no Wayland session (`d2b.site.waylandUser` is
    /// unset). Optional so bundles that predate the field still parse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wayland_socket: Option<String>,
}

impl SiteJson {
    /// The projected Wayland socket, or `None` when the site declares none.
    ///
    /// The value is re-fenced here so a consumer never acts on a malformed
    /// artifact even if one reached the resolver through a test fixture.
    pub fn wayland_socket(&self) -> Option<&str> {
        self.wayland_socket
            .as_deref()
            .filter(|socket| wayland_socket_ok(socket))
    }

    /// Fail-closed artifact validation: a malformed socket is an emitter
    /// contract violation and refuses the bundle load.
    pub fn validate(&self) -> Result<(), &'static str> {
        match self.wayland_socket.as_deref() {
            None => Ok(()),
            Some(socket) if wayland_socket_ok(socket) => Ok(()),
            Some(_) => Err("invalid-wayland-socket"),
        }
    }
}

/// The one accepted shape: exactly `/run/user/<uid>/<display>` with no parent
/// components - the value the Nix emitter resolves from
/// `d2b.site.waylandUser`'s uid and `d2b.site.waylandDisplay`.
fn wayland_socket_ok(socket: &str) -> bool {
    let mut components = std::path::Path::new(socket).components();
    matches!(components.next(), Some(Component::RootDir))
        && matches!(components.next(), Some(Component::Normal(part)) if part == "run")
        && matches!(components.next(), Some(Component::Normal(part)) if part == "user")
        && matches!(
            components.next(),
            Some(Component::Normal(uid)) if uid.to_str().is_some_and(|uid| uid.parse::<u32>().is_ok())
        )
        && matches!(components.next(), Some(Component::Normal(_)))
        && components.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::SiteJson;

    fn site(value: Option<&str>) -> SiteJson {
        SiteJson {
            schema_version: "v1".to_owned(),
            wayland_socket: value.map(str::to_owned),
        }
    }

    #[test]
    fn site_json_round_trips_the_projected_socket() {
        let parsed: SiteJson = serde_json::from_str(
            r#"{"schemaVersion":"v1","waylandSocket":"/run/user/1000/wayland-1"}"#,
        )
        .expect("the emitted artifact parses");
        assert_eq!(parsed.wayland_socket(), Some("/run/user/1000/wayland-1"));
        assert_eq!(
            serde_json::to_string(&parsed).expect("serializes"),
            r#"{"schemaVersion":"v1","waylandSocket":"/run/user/1000/wayland-1"}"#
        );

        let headless: SiteJson =
            serde_json::from_str(r#"{"schemaVersion":"v1","waylandSocket":null}"#)
                .expect("a Wayland-less site still emits the artifact");
        assert_eq!(headless.wayland_socket(), None);
        assert_eq!(headless.validate(), Ok(()));

        let absent: SiteJson =
            serde_json::from_str(r#"{"schemaVersion":"v1"}"#).expect("the field is optional");
        assert_eq!(absent.wayland_socket(), None);
    }

    #[test]
    fn site_json_refuses_malformed_sockets_and_unknown_fields() {
        for socket in [
            "run/user/1000/wayland-0",
            "/run/user/zzz/wayland-0",
            "/run/user/1000/wayland-0/extra",
            "/run/user/1000/../wayland-0",
            "/tmp/wayland-0",
        ] {
            let parsed = site(Some(socket));
            assert_eq!(
                parsed.validate(),
                Err("invalid-wayland-socket"),
                "{socket} must not validate"
            );
            assert_eq!(
                parsed.wayland_socket(),
                None,
                "{socket} must not resolve from the artifact"
            );
        }
        let error = serde_json::from_str::<SiteJson>(
            r#"{"schemaVersion":"v1","waylandSocket":"/run/user/1000/wayland-0","extra":true}"#,
        )
        .expect_err("unknown fields fail closed");
        assert!(error.to_string().contains("unknown field"));
    }
}
