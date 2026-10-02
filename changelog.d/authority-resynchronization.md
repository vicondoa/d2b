fix(authority): reconcile a restarted broker's Zone instead of wedging it

A broker restart moves every known Zone to reconciling and refuses every
ordinary publication message until the manager shows it the projection it
already accepted. Nothing did, so a daemon that restarted against a broker that
restarted was fenced for the rest of its life: every mutation refused with
`publication-reconciliation-required`, which is correct - the broker had not
been shown anything - with no path back.

The resynchronization now exists end to end, and it is PROVED rather than
asserted:

- The broker checks a resynchronized document against the projection it durably
  holds. The cursor must be exactly the accepted one, the deployment root must
  be the one it bootstrapped, and every authority row the document carries must
  equal the row it already accepted - same revision, same committed bytes, same
  resolved relationship identity. A document that moves the cursor forward, or
  that carries an authority row this broker never accepted, is refused by the
  new `publication-projection-unproven` code and leaves the Zone fenced. The
  install merges over the accepted rows rather than replacing them, so no
  reconciliation can silently drop a grant the broker already accepted.
- The store keeps a durable per-Zone projection: the accepted cursor, every
  committed desired row at the revision and digest it committed at, and the
  committed source and consumer identity a binding row's relationship key folds
  in. A binding key is over committed identity, so that identity cannot be
  derived from the row's own bytes; it is resolved once, against the Zone's
  committed rows, and it travels through the journal, the outbox, the fence,
  and the commit so a replay and a reconciliation publish the same one.
- `AuthorityPublisher::resynchronize` is the third seam call, beside `prepare`
  and `commit`. The manager drives it immediately after adoption and before any
  row is loaded or any actor spawned, and the plane drives it for the seed's
  Zone through the same path.

A daemon that cannot prove its projection is still refused, and a store that
acknowledged a publication the broker never accepted is refused by name before
anything is sent: there is no document that proves a cursor this broker holds
no fact about.
