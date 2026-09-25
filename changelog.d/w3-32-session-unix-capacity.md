### Changed

- Unix session send and receive bursts pre-size their packet buffers to the
  fairness budget, eliminating incremental reallocations on the hot path.
- Sending a packet with attachments pre-sizes the descriptor and identity
  collections, avoiding reallocations while each attachment is processed.