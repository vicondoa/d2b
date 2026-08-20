{ lib }:

{
  evalModules =
    { modules
    , specialArgs ? { }
    }:
    lib.evalModules {
      inherit modules specialArgs;
    };

  setup =
    { modules
    , specialArgs ? { }
    }:
    {
      inherit modules specialArgs;
      eval = extraModules:
        lib.evalModules {
          modules = modules ++ extraModules;
          inherit specialArgs;
        };
    };
}
