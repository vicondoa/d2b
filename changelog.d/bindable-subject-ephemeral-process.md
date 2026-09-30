### Changed

- A `RoleBinding` may now name an `EphemeralProcess` subject. Both Process
  lifetimes are the same converted resource type and already share one
  preparation path and one policy path, so the closed subject vocabulary names
  both and a run-to-completion consumer's binding leg is authorized by the same
  contract that authorizes a long-running one. Naming only `Process` left the
  binding vocabulary as a second authority that distinguished the two lifetimes
  for no security reason, which is the differential authority route the model
  forbids. The generated Nix `subjectTypes` projection carries the new entry,
  so configuration authored against the vocabulary sees it too.
