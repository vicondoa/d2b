# d2b examples

These flakes exercise the current Zone resource authoring surface against the
in-tree framework. Each Guest is declared as a Zone resource and its system
evaluator is supplied through `d2b.guestSystems`; the controller-owned child
graph is never duplicated in Nix.

| Path | Audience | Focus |
| --- | --- | --- |
| [`minimal/`](./minimal/) | First evaluation | One headless Guest in `local-root`. |
| [`graphics-workstation/`](./graphics-workstation/) | Wayland host | One Guest with host Wayland admission enabled. |
| [`multi-env/`](./multi-env/) | Isolation comparison | Same-named Guests in two child Zones. |
| [`with-observability/`](./with-observability/) | Telemetry integration | A Zone-owned observability Provider beside a Guest. |
| [`with-entra-id/`](./with-entra-id/) | External identity integration | A Guest slot for a consumer-owned Entra evaluator. |

## Lock policy

The examples that consume `d2b.url = "path:../.."` intentionally do not
commit a `flake.lock`: the path input is mutable and a checked-in lock would
describe stale source. The external identity module is an optional consumer
input in the Entra example and is wired by the host flake when needed.

When copying an example, replace the path input with a released d2b reference
and replace the placeholder artifact packages with signed Provider and Guest
system artifacts.

## See also

- [`../templates/default/`](../templates/default/) - `nix flake init` scaffold.
- [`../README.md`](../README.md) - product model and operator quick start.
- [`../docs/reference/zone-control-nix.md`](../docs/reference/zone-control-nix.md)
  - current Zone and Guest authoring.
