---
type: fixed
area: daemon,broker
---

### Fixed

- **The daemon's origination link could never complete a round trip.**
  `AuthorityPublication`'s coordinator called `oneshot::Receiver::blocking_recv()`
  from an async context, which panics on any runtime worker. Every publication the
  daemon attempted from a worker died before reaching the broker, which is why no
  test had ever driven the link. The round trip is awaited now.

- **`d2b host reset` could never succeed on a real host.** The broker's reset
  path read `deployment-graph.json` under schema `d2b-deployment-graph/1`, a
  document no production path authors, while the deployment installs
  `deployment-bootstrap.json` under `d2b-deployment-bootstrap/1` — a different
  name and a different contract. Reset now reads the one document the daemon and
  the broker both publish and verify.

- **The host reset's admission could not fail.** The check presented
  `Operator` as both the request subject and the graph's root, so the bootstrap
  class admitted it by construction — every production graph roots at
  `Bootstrap`, which is why the reset had to fabricate its own root to be
  admitted at all. `Operator` is no longer lumped with `Bootstrap`, the root
  comes from the document, and the request subject comes from the document's own
  accepted `RoleBinding` rows. A caller can no longer build a graph that matches
  its own subject.

- **The Nix bundle subject was refused as display text.** Bundle ingestion
  presents `nix:<bundle-identity>`, which does not parse as a `ResourceRef`, and
  the identity arm matched only `bootstrap` and `operator`. The plane's
  admission now classifies that subject — requiring both the prefix and
  `ResourceProvenance::Nix`, which an API caller cannot supply, because a
  caller's principal must parse as a `ResourceRef` — so a bundle-ingested row is
  admitted by the graph arm rather than refused by name.

### Changed

- **The broker's own answer no longer claims more than it checked.** An
  endpoint grant reported traversability from a check scoped to the broker's own
  runtime root, while an observation reported it from the whole path to `/`; the
  two could disagree, and a grant could report "traversable" from a check that
  never looked above the root. Every verb now answers from one full-path read.
  The grant still installs traversal only inside the root the broker owns.

- **A Guest receives its own Zone's verified deployment graph.** It previously
  read a deployment root nothing ever wrote, so no Guest could start; and the
  document the host installs names the system Zone, which the Guest refuses. The
  per-Zone graph rides the image's own `/etc` closure and the Guest verifies its
  self-hash before reading a row out of it, checking the digest before the Zone.