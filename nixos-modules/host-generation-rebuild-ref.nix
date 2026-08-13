# Validated target-closure carrier for activation-nixos.
{ config, lib, pkgs, ... }:

let
  value = config.d2b.site.hostGenerationRebuildRef;
  pattern = "^[A-Za-z0-9+._~:/?@%=&,-]+#[A-Za-z0-9][A-Za-z0-9_-]{0,63}$";
  valid = value == null
    || (builtins.isString value
      && builtins.stringLength value > 0
      && builtins.stringLength value <= 2048
      && builtins.match pattern value != null
      && !(lib.hasInfix "\n" value)
      && !(lib.hasInfix "\r" value)
      && value == lib.strings.trim value);
in
{
  options.d2b.site.hostGenerationRebuildRef = lib.mkOption {
    type = lib.types.nullOr lib.types.str;
    default = null;
    description = "Validated non-secret target closure rebuild reference.";
  };

  options.d2b._hostGenerationRebuildRef = lib.mkOption {
    type = lib.types.attrsOf lib.types.anything;
    default = { };
    internal = true;
    visible = false;
  };

  config = {
    assertions = [
      {
        assertion = valid;
        message = "d2b.site.hostGenerationRebuildRef must be one bounded single-line reference with a selector.";
      }
    ];
    d2b._hostGenerationRebuildRef = lib.mkIf (value != null) {
      bytes = value;
      carrier = pkgs.writeText "d2b-host-generation-rebuild-ref" value;
      publishedBy = "ApplyHostGenerationHandoff";
    };
  };
}
