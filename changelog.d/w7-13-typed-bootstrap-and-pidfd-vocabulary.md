### Changed

- The provider-controller bootstrap take leg sends and reads the
  committed `take-controller-bootstrap` row as a typed request/response
  pair instead of a hand-built payload and a loose field read, and a
  reply that does not carry the committed response is a hard failure
  naming the failed broker call. Such a reply previously read as "no
  escrow held" and replaced a controller whose bootstrap endpoint the
  daemon had not claimed.

- The pidfd failure kinds a broker pidfd dispatch reports are declared
  once, in the broker wire contract, and the live handler maps its pidfd
  variants through that vocabulary. The open-pidfd refusal detail now
  names the kind instead of leaving it implicit in the display text, and
  the daemon side no longer reads the broker's source to learn the
  names.

