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
      path =
        if builtins.typeOf spec == "path"
        then spec
        else spec.path;
    in
    if builtins.typeOf spec == "path" || !(spec ? names) then
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
        lib.filterAttrs (caseName: _: builtins.elem caseName spec.names) imported;
in
{
  inherit formatSelectionError;
  selectCaseFiles = map selectCaseFile;
}
