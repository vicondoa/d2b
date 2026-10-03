### Fixed

- An authority publication over the broker socket is now bounded by its
  budget for the whole round trip and not only for the connect. The connect
  was already budgeted, but the exchange that followed it ran on a socket with
  no deadline of its own, and a broker that accepted the connection and then
  stopped reading parked the single publication worker for good: nothing
  behind it drained, and once the queue filled the link refused by name. The
  budget now rides on the connected socket as that socket's read and write
  deadline, and a deadline that cannot be installed refuses the round trip
  rather than running it unbounded. There is exactly one publication worker,
  so an unbounded exchange there is the one condition that wedges every
  publication the daemon ever makes.