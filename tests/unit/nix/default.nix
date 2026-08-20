{ cases }:

let
  merge = acc: current:
    let
      duplicateNames = builtins.attrNames (builtins.intersectAttrs acc current);
    in
    if duplicateNames != [ ] then
      throw "nix surface has duplicate case names: ${builtins.concatStringsSep ", " duplicateNames}"
    else
      acc // current;
in
builtins.foldl' merge { } cases
