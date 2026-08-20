{ lib, ... }:

{
  options.d2bActivationNixos = {
    retainedGenerations = lib.mkOption {
      type = lib.types.ints.between 1 16;
      default = 3;
      description = "Bounded finalizer-driven generation retention window.";
    };
  };
}
