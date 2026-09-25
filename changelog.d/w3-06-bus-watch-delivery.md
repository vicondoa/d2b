### Fixed

- Bus watch delivery no longer allocates a fresh frame buffer per frame or
  per backpressure retry: the payload is copied once at the stream bridge
  when the frame is admitted, so bounded watch streams keep their kept-half
  credit path allocation-free on retry.
- Bus session seam tests wait on observable conditions instead of fixed-count
  yield and poll loops, so a loaded runner can no longer fail them by being
  slow.
- The bus operation-table test asserts the `RetainedOperationId` variant
  instead of pinning its full error sentence, so a wording change no longer
  breaks the cancellation-retry contract test.