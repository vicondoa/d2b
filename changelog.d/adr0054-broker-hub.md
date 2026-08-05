### Added

- Recorded the decision that resolves the last blocker in the Bazel Rust build
  and test migration: the privileged broker's dependency hub. The broker keeps
  its own Cargo workspace and its own independently pinned and independently
  audited lock file, which is the dependency closure of the only binary the
  framework runs as root, and neither that manifest nor that lock is edited,
  generated, or rewritten by any build command. The build tool instead reads a
  generated, drift-checked stand-in workspace derived from what the broker's
  own dependency resolution actually contains, so an optional dependency the
  broker never turns on stays out of the privileged closure along with
  everything it would pull in. Four independent checks prove the generated tree
  still describes the authoritative lock and that the build-side lock was
  produced from the tree the repository currently has, and each catches drift
  the others cannot see.
- Recorded that the shared first-party libraries the broker uses are built
  twice, once for each dependency set, while their tests continue to run once
  in the main workspace exactly as they do today, because the broker's lock
  does not contain those crates' test-only dependencies and its own test run
  never built them. An explicit check refuses any build graph in which the
  privileged binary reaches a library built against the other dependency set,
  or the main build reaches one built against the broker's, so the audited
  closure cannot silently become a mixture of the two. That check first
  confirms it is looking at exactly the set of libraries the broker's own
  dependency resolution implies, and fails on a set that is empty, short or
  too long, so it cannot report success by examining nothing.
- Recorded that regenerating the broker's build-side dependency lock is one
  contributor command that brings the generated stand-in workspace up to date
  itself rather than refusing and asking for a second command. It regenerates
  and validates those inputs offline before it starts the build tool, refuses
  outright when the stand-in workspace already carries local changes it did not
  make, naming each one and a single reversible command that puts tracked,
  untracked and ignored entries alike safely aside, and afterwards proves it
  changed nothing beyond the stand-in workspace and that one build-side lock,
  naming any other changed path. The command it prints is chosen to actually
  clear the
  refusal: the most likely leftover is a build output directory that version
  control ignores, and the narrower form of the same command silently leaves
  that behind. Where a half-finished merge is the cause, it prints the bounded
  command that finishes it for those paths instead, because setting changes
  aside is not something version control will do for a file with an unresolved
  conflict. It accepts a stand-in workspace it wrote itself on an
  earlier run whose result is not yet committed, so making two dependency
  changes before committing works, and it still compares every file byte for
  byte. It never asks anyone to delete a path recursively.
- Recorded how that command replaces the generated directory: it writes the
  validated files into a staging area beside the target, on the same
  filesystem by construction, makes them durable, reads each one back through
  the same handle it wrote it through so that what is verified is what reached
  the device, and swaps them into place in
  a single step, so the directory is never absent, never half written, and
  never assembled from a path that was checked once and resolved again to be
  written. A filesystem that cannot perform that swap is refused before
  anything is touched rather than worked around, an interrupted run is
  recovered by comparing what is on disk rather than by trusting a flag it
  could not have written reliably, the previous copy is deleted only when every
  file in it is one the command itself measured, and two regeneration commands
  cannot run over each other. Anything an interrupted run left behind before it
  had any recovery record to its name is cleared at the start of the next run,
  from a fixed list of the names it could have created, leaving the command's
  own bookkeeping files alone and refusing an unfamiliar name rather than
  guessing what it is. Each of the three refusals that can come out of that
  bookkeeping area now prints a recovery command that actually clears it,
  against the area that holds the state rather than against the generated
  directory, which is the one the earlier wording pointed at and which would
  have left the contributor refused in exactly the same way on every re-run.
  That recovery is a mode of the regeneration command itself rather than a
  line of version-control shell for the contributor to paste. It moves the
  whole bookkeeping directory aside, in one step, to an ignored name beside
  it, without reading, copying, renaming or deleting anything inside it, so
  entries of a kind version control cannot store at all are preserved exactly
  at any depth and under any name, rather than surviving the recovery and
  refusing the next run in the same way. It takes the same exclusion token a
  regeneration takes and declines to act while a regeneration is actually
  running, which the version-control command it replaces was measured not to
  do. It prints where it put the state, so it stays readable in place, and
  says to run the ordinary regeneration again. There is a small fixed number
  of places it will put state aside; once they are full it stops and says so
  rather than choosing for the contributor, and it never deletes anything or
  tells anyone else to. Unrelated changes elsewhere in the working copy,
  staged or not,
  tracked or ignored, are left alone by all of it. The token that keeps them apart is held only
  while a regeneration is actually running: it is closed automatically when the
  command starts the build tool, so the build tool's long-lived background
  server cannot end up holding it and locking the contributor out of their own
  working copy until that server is stopped. Nothing here adds a privileged
  path or any state outside the contributor's own working copy.
- Recorded that the command takes an exact copy of the build-side lock before
  it starts the build tool. If the build tool fails, that copy is put back, so
  the lock is exactly what it was even if the tool had already begun rewriting
  it, while the stand-in workspace stays current; the command reports both and
  re-running it is the recovery. A run that is killed outright rather than
  failing is picked up where it stopped: the record it leaves says whether the
  build tool ever finished, which is the one thing the files on disk cannot
  say, so a run interrupted after the generated directory was swapped in but
  before the build tool succeeded goes on to run it rather than reporting
  success on a build-side lock nothing regenerated, and a run whose success
  was recorded is not made to do that work twice. The command no longer demands
  that the
  build-side lock match the last commit before it will run, which had made a
  successful run refuse the next one and had left no way to regenerate a lock
  that came out of a merge conflict. Build and gate entry points still generate
  nothing, and the drift gate still refuses a stale committed tree.
