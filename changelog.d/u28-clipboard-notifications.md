# Convert clipboard and notifications onto declared services and endpoint grants

The clipboard and notification Providers each decided two things beside the
graph. A hand-written table of service-package strings decided which role an
authenticated route served, and the host or desktop channel a selection or a
notification was delivered over was reached by falling back to whichever other
route happened to be available. Both are now declared, and both go through one
typed admission.

Clipboard gains a declared service surface: `CLIPBOARD_SERVICES` lists the
management, bridge, and picker services with their declared methods, and
`AuthenticatedClipboardSession::from_authenticated_route` resolves a route's
role through that table instead of re-matching package strings. The three
delivery channels - Guest transfer, host selection read, and host selection
supply - are declared as typed `EndpointBinding` requests over the exact
`Endpoint` rows the composing host committed, each in its own stable consumer
slot for its own bounded purpose. `admit_clipboard_endpoint` is the one gate
that admits one, and the runtime refuses a channel that carries no admitted
relationship, or one the graph has revoked, is draining, or has re-fenced -
before it constructs a host route at all, so a withdrawn endpoint stops the
selection instead of redirecting it onto another host channel. The host's
`ReasonCode` vocabulary gains `endpoint_absent`, `endpoint_withdrawn`, and
`endpoint_refused` so the refusal stays legible in the paste-failure
notification.

Notifications gain the same shape. `NOTIFICATION_SERVICE` declares the
notification methods and both named streams, and the signed descriptor's
`streams()` now reads it rather than keeping a second list. `admission.rs`
derives the Provider's two endpoint grants - the Guest-source stream and the
desktop presentation channel - and `admit_notification_endpoint` is the one
gate; `NotificationSink::deliver_over_endpoint` runs it before the desktop
presentation port is touched, and `NotificationLifecycleSupervisor::apply_over_endpoint`
runs it before any host effect, so a plan that starts or stops a host sink
cannot be applied without an admitted presentation relationship. The Guest
source and host-sink identities now derive their relationships from committed
rows through the declared stream vocabulary instead of naming an endpoint with
a free-form label.

Content filtering, directionality, and every user-visible outcome are
unchanged. No content value is an input to any derivation in either Provider:
a MIME token, a payload byte, a summary, a body, or an action label cannot
become a slot, a purpose, a consumer, or a source endpoint, and the issued
action capability is still minted per observer session rather than from the
action's own text.
