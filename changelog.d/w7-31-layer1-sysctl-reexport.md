### Fixed

- The broker's sysctl destroy-value re-export is compiled only where its
  user is: the layer-1 bootstrap build no longer fails on the unused
  import while the normal build keeps `destroy_sysctl_value` working
  unchanged.
