### Fixed

- Silence, absence, and failure are three different facts in the Process
  launch binding gate, and are now read as three. A relationship row the
  manager positively holds no row for is a proven loss: it maps to the same
  `Pending` a withdrawn delivery produces, so a helper that would keep
  running over access whose delivery provably does not exist stops. A plane
  that cannot answer, and a row that is held but has published nothing for
  its current generation - including one caught mid-pass with its projection
  blank - are silence and still defer, which is what keeps one unlucky read
  from un-realizing the endpoint behind a live helper and waking the row
  again.

- A terminally `Failed` `Endpoint` row is a statement rather than silence.
  It publishes a status for its current generation and no projection beside
  it, because refusing a lookalike shape is a designed terminal state; the
  gate now reads that as the row granting nothing for this consumer, instead
  of as an unfinished pass. One refused endpoint committed anywhere in a Zone
  no longer defers every root `Process` row in that Zone forever.

- A Process row keeps the name of an internal-watch registration whose
  release the manager refused. The entry stays in the live set rather than
  being dropped on a discarded error, which had leaked a registration the
  actor could no longer release or re-arm; the next pass releases it.