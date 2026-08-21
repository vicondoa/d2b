{ lib
, context
, surfaceName
}:

let
  formatSelectionError = { path, missingNames ? [ ], emptySelection ? false }:
    if emptySelection then
      "${surfaceName} surface case file ${builtins.baseNameOf (toString path)} has empty names selection"
    else
      "${surfaceName} surface case file ${builtins.baseNameOf (toString path)} missing requested names: ${builtins.concatStringsSep ", " missingNames}";

  selectCaseFile = spec:
    let
      isPathSpec = builtins.typeOf spec == "path";
      path =
        if isPathSpec
        then spec
        else spec.path;
    in
    if isPathSpec || !(spec ? names) then
      import path context
    else if spec.names == [ ] then
      throw (formatSelectionError {
        inherit path;
        emptySelection = true;
      })
    else
      let
        imported = import path context;
        missingNames = lib.filter
          (caseName: !builtins.hasAttr caseName imported)
          spec.names;
      in
      if missingNames != [ ] then
        throw (formatSelectionError {
          inherit path missingNames;
        })
      else
        lib.getAttrs spec.names imported;
in
{
  inherit formatSelectionError;
  selectCaseFiles = map selectCaseFile;
}
