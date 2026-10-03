### Changed

- A `RoleBinding` may now name an `EphemeralProcess` subject. Both Process
  lifetimes are the same converted resource type and already share one
  preparation path and one policy path, so the closed subject vocabulary names
  both and a run-to-completion consumer's binding leg is authorized by the same
  contract that authorizes a long-running one. Naming only `Process` left the
  binding vocabulary as a second authority that distinguished the two lifetimes
  for no security reason, which is the differential authority route the model
  forbids.

- The closed subject vocabulary is now declared in exactly one place and read
  from there. The foundation seed kept its own copy of the list, and the two
  had diverged: the seed carried a `Group` entry that the contract omitted, so
  the seed could commit a `RoleBinding` row naming a `Group` that the contract
  decoder would then refuse. `Group` is a real principal the volume principal
  projection resolves, so it is now declared in the contract and the seed
  derives its slice from it, which makes this class of divergence impossible
  rather than fixed once.

- The generated Nix `subjectTypes` projection carries both new entries, so
  configuration authored against the vocabulary sees them too.
