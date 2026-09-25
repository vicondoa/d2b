### Fixed

- The `d2b-broker-fixture-syscall-surface` fixture's x86_64 `asm!` syscall
  surface now declares the registers the `syscall` instruction clobbers
  (`rcx` and `r11`), so the compiler's register assumptions hold if the
  fixture function is ever executed.