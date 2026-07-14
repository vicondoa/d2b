# Independent pure-Nix implementation of ADR 0045 canonical runtime IDs.
let
  prefix = "d2b-id-v2;";
  alphabet = "abcdefghijklmnopqrstuvwxyz234567";
  hexAlphabet = "0123456789abcdef";
  printable =
    " !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~";

  contains = needle: value:
    let
      needleLength = builtins.stringLength needle;
      valueLength = builtins.stringLength value;
    in
      needleLength <= valueLength
      && builtins.any
        (index: builtins.substring index needleLength value == needle)
        (builtins.genList (index: index) (valueLength - needleLength + 1));

  domains = {
    realm = "d2b-v2:realm";
    workload = "d2b-v2:workload";
    provider = "d2b-v2:provider";
    role = "d2b-v2:role";
  };

  expectedPartCount = domain:
    if domain == domains.realm then 1
    else if domain == domains.workload then 2
    else if domain == domains.provider || domain == domains.role then 3
    else throw "v2 identity: unknown domain";

  providerTypes = [
    "runtime"
    "infrastructure"
    "transport"
    "substrate"
    "credential"
    "display"
    "network"
    "storage"
    "device"
    "audio"
    "observability"
  ];

  roleKinds = [
    "store-virtiofs-preflight"
    "swtpm-pre-start-flush"
    "swtpm"
    "virtiofsd"
    "video"
    "gpu"
    "gpu-render-node"
    "audio"
    "cloud-hypervisor"
    "qemu-media"
    "vsock-relay"
    "guest-control-health"
    "usbip"
    "security-key-frontend"
    "wayland-proxy"
  ];

  mod = dividend: divisor:
    dividend - (builtins.div dividend divisor) * divisor;

  findChar = haystack: needle:
    let
      go = index:
        if index == builtins.stringLength haystack then
          throw "v2 identity: non-printable ASCII"
        else if builtins.substring index 1 haystack == needle then
          index
        else
          go (index + 1);
    in
    go 0;

  isPrintableAscii = value:
    builtins.match "[ -~]+" value != null;

  validateCanonicalName = kind: value:
    if builtins.stringLength value <= 63
      && builtins.match "[a-z][a-z0-9-]*" value != null
    then value
    else throw "v2 identity: invalid canonical ${kind}";

  splitDots = value:
    let
      go = remaining:
        let match = builtins.match "([^.]*)[.](.*)" remaining;
        in if match == null then [ remaining ]
        else [ (builtins.elemAt match 0) ] ++ go (builtins.elemAt match 1);
    in
    go value;

  validateRealmPath = value:
    let
      components = splitDots value;
      count = builtins.length components;
      labels = builtins.genList (index: builtins.elemAt components index) (count - 1);
      validLabels = builtins.all
        (label:
          (builtins.tryEval
            (validateCanonicalName "realm label" label)).success)
        labels;
    in
    if value != ""
      && isPrintableAscii value
      && !(builtins.match ".*[.]d2b" value != null)
      && count >= 1
      && builtins.elemAt components (count - 1) == "local-root"
      && validLabels
    then value
    else throw "v2 identity: invalid canonical realm path";

  field = value:
    if isPrintableAscii value then
      "${builtins.toString (builtins.stringLength value)}:${value};"
    else
      throw "v2 identity: field must be non-empty printable ASCII";

  encode = domain: parts:
    if builtins.length parts != expectedPartCount domain then
      throw "v2 identity: invalid domain part count"
    else
      "${prefix}${field domain}${builtins.toString (builtins.length parts)};"
      + builtins.concatStringsSep "" (builtins.map field parts);

  decimal = value:
    if builtins.match "(0|[1-9][0-9]*)" value == null then
      throw "v2 identity: noncanonical decimal"
    else
      let parsed = builtins.tryEval (builtins.fromJSON value);
      in if parsed.success && builtins.isInt parsed.value && parsed.value >= 0
         then parsed.value
         else throw "v2 identity: decimal out of range";

  delimiterOffset = delimiter: value:
    let
      go = index:
        if index == builtins.stringLength value then
          throw "v2 identity: missing delimiter"
        else if builtins.substring index 1 value == delimiter then
          index
        else
          go (index + 1);
    in
    go 0;

  takeToken = delimiter: value:
    let offset = delimiterOffset delimiter value;
    in {
      token = builtins.substring 0 offset value;
      rest = builtins.substring (offset + 1)
        (builtins.stringLength value - offset - 1) value;
    };

  parseField = value:
    let
      lengthToken = takeToken ":" value;
      length = decimal lengthToken.token;
      available = builtins.stringLength lengthToken.rest;
      parsedValue = builtins.substring 0 length lengthToken.rest;
      separator = builtins.substring length 1 lengthToken.rest;
    in
    if length == 0 || available <= length || separator != ";"
      || !isPrintableAscii parsedValue
    then throw "v2 identity: malformed field"
    else {
      value = parsedValue;
      rest = builtins.substring (length + 1) (available - length - 1)
        lengthToken.rest;
    };

  parseEncoded = encoded:
    let
      prefixLength = builtins.stringLength prefix;
      hasPrefix =
        builtins.stringLength encoded >= prefixLength
        && builtins.substring 0 prefixLength encoded == prefix;
      body = builtins.substring prefixLength
        (builtins.stringLength encoded - prefixLength) encoded;
      domainField = parseField body;
      countToken = takeToken ";" domainField.rest;
      count = decimal countToken.token;
      wanted = expectedPartCount domainField.value;
      parseParts = remaining: left: acc:
        if left == 0 then { inherit remaining acc; }
        else
          let parsed = parseField remaining;
          in parseParts parsed.rest (left - 1) (acc ++ [ parsed.value ]);
      parsed = parseParts countToken.rest count [ ];
    in
    if !hasPrefix || !isPrintableAscii encoded || count != wanted
      || parsed.remaining != ""
    then throw "v2 identity: malformed canonical encoding"
    else {
      domain = domainField.value;
      parts = parsed.acc;
    };

  hexNibbles = {
    "0" = 0; "1" = 1; "2" = 2; "3" = 3;
    "4" = 4; "5" = 5; "6" = 6; "7" = 7;
    "8" = 8; "9" = 9; "a" = 10; "b" = 11;
    "c" = 12; "d" = 13; "e" = 14; "f" = 15;
  };

  hexNibble = char:
    if builtins.hasAttr char hexNibbles
    then builtins.getAttr char hexNibbles
    else throw "v2 identity: digest is not lowercase hexadecimal";

  byteAt = hex: index:
    16 * hexNibble (builtins.substring (2 * index) 1 hex)
    + hexNibble (builtins.substring (2 * index + 1) 1 hex);

  powersOfTwo = [ 1 2 4 8 16 32 64 128 ];

  bitAt = hex: bit:
    let
      byte = byteAt hex (builtins.div bit 8);
      divisor = builtins.elemAt powersOfTwo (7 - mod bit 8);
    in
    mod (builtins.div byte divisor) 2;

  symbolAt = hex: index:
    let
      firstBit = index * 5;
      value = builtins.foldl'
        (acc: offset:
          acc * 2
          + (if firstBit + offset < 96
             then bitAt hex (firstBit + offset)
             else 0))
        0
        (builtins.genList (offset: offset) 5);
    in
    builtins.substring value 1 alphabet;

  base32First96 = hex:
    if builtins.stringLength hex != 64
      || builtins.match "[0-9a-f]*" hex == null
    then throw "v2 identity: digest is not 64 lowercase hexadecimal characters"
    else builtins.concatStringsSep ""
      (builtins.genList (index: symbolAt hex index) 20);

  shortId = domain: parts:
    base32First96 (builtins.hashString "sha256" (encode domain parts));

  validateShortId = value:
    let last = builtins.substring 19 1 value;
    in if builtins.stringLength value == 20
      && builtins.match "[a-z2-7]*" value != null
      && (last == "a" || last == "q")
    then value
    else throw "v2 identity: invalid canonical short id";

  deriveRealmId = path:
    shortId domains.realm [ (validateRealmPath path) ];

  deriveWorkloadId = realmId: workloadName:
    shortId domains.workload [
      (validateShortId realmId)
      (validateCanonicalName "workload name" workloadName)
    ];

  deriveProviderId = realmId: providerType: configuredProviderId:
    if !(builtins.elem providerType providerTypes) then
      throw "v2 identity: invalid provider type"
    else shortId domains.provider [
      (validateShortId realmId)
      providerType
      (validateCanonicalName "configured provider instance id" configuredProviderId)
    ];

  deriveRoleId = realmId: workloadId: roleKind:
    if !(builtins.elem roleKind roleKinds) then
      throw "v2 identity: invalid role kind"
    else shortId domains.role [
      (validateShortId realmId)
      (validateShortId workloadId)
      roleKind
    ];

  recompute = encoded:
    let
      parsed = parseEncoded encoded;
      p = parsed.parts;
      id =
        if parsed.domain == domains.realm then
          deriveRealmId (builtins.elemAt p 0)
        else if parsed.domain == domains.workload then
          deriveWorkloadId (builtins.elemAt p 0) (builtins.elemAt p 1)
        else if parsed.domain == domains.provider then
          deriveProviderId
            (builtins.elemAt p 0) (builtins.elemAt p 1) (builtins.elemAt p 2)
        else if parsed.domain == domains.role then
          deriveRoleId
            (builtins.elemAt p 0) (builtins.elemAt p 1) (builtins.elemAt p 2)
        else throw "v2 identity: unknown domain";
    in {
      inherit (parsed) domain parts;
      inherit id;
    };

  charHex = char:
    let
      code = 32 + findChar printable char;
      high = builtins.div code 16;
      low = mod code 16;
    in
    builtins.substring high 1 hexAlphabet
    + builtins.substring low 1 hexAlphabet;

  asciiHex = value:
    if !isPrintableAscii value then
      throw "v2 identity: hexadecimal input is not printable ASCII"
    else builtins.concatStringsSep ""
      (builtins.genList
        (index: charHex (builtins.substring index 1 value))
        (builtins.stringLength value));

  insertUnique = error: seen: value:
    if builtins.hasAttr value seen then throw error
    else seen // { ${value} = true; };

  validateGlobalIdentities =
    { realms ? [ ], workloads ? [ ], providers ? [ ], roles ? [ ] }:
    let
      checkedProviders = builtins.foldl'
        (insertUnique "v2 identity: duplicate globally scoped provider id")
        { }
        (builtins.map validateShortId providers);
      all = realms ++ workloads ++ providers ++ roles;
      checkedAll = builtins.foldl'
        (insertUnique "v2 identity: canonical short-id collision")
        { }
        (builtins.map validateShortId all);
    in
    builtins.deepSeq checkedProviders (builtins.deepSeq checkedAll true);

  hasNul = value:
    contains "\\u0000" (builtins.toJSON value);

  unixPathHeadroom = path:
    if hasNul path then throw "v2 identity: Unix pathname contains NUL"
    else if builtins.stringLength path > 107 then
      throw "v2 identity: Unix pathname exceeds 107 bytes"
    else 107 - builtins.stringLength path;
in
{
  inherit
    asciiHex
    base32First96
    domains
    encode
    parseEncoded
    providerTypes
    recompute
    roleKinds
    shortId
    unixPathHeadroom
    validateCanonicalName
    validateGlobalIdentities
    validateRealmPath
    validateShortId
    ;
  inherit
    deriveProviderId
    deriveRealmId
    deriveRoleId
    deriveWorkloadId
    ;
  shortIdLength = 20;
  linuxUnixPathMaxBytes = 107;
}
