### Fixed

- Wired the network Nix-unit surface to the retained `net-vm-network`
  cases and evaluated `net.nix` as a guest module so the net VM
  `10-eth-dhcp` `mkForce` sentinel-MAC neutralizer is actually asserted.
- Fail closed when a Nix-unit surface evaluates zero cases instead of
  reporting success.
