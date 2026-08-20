{ lib, ... }:

{
  options.d2bVolumeVirtiofs = {
    threadPoolSize = lib.mkOption {
      type = lib.types.ints.between 1 256;
      default = 1;
      description = "Bounded virtiofsd worker thread count.";
    };
    cache = lib.mkOption {
      type = lib.types.enum [ "auto" "always" "never" ];
      default = "auto";
      description = "virtiofsd cache mode.";
    };
  };
}
