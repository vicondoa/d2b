### Changed

- Use the native BuildBuddy Workflows `build / check` action for protected
  `v3` pull requests and trusted pushes, running the remote-compatible fixed
  Layer-1 selection locally in an Ubuntu 22.04 hosted runner without nesting
  the RBE profile or using a GitHub secret-bearing proxy.
