### Changed

- Routed CLI EphemeralProcess attachment through the typed Zone
  ComponentSession client, preserving bounded refusal and close behavior
  instead of sending the retired OpenTerminal request directly.
