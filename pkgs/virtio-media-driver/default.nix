{ lib, stdenv, fetchFromGitHub, kernel }:

stdenv.mkDerivation {
  pname = "virtio-media-driver";
  version = "0-unstable-2026-05-21";

  src = fetchFromGitHub {
    owner = "chromeos";
    repo = "virtio-media";
    rev = "ebcef1a5037a1dc5869af8aa82ed75a2e7739f0f";
    hash = "sha256-ojg8atMuBT4o/1oWBOc/0lkCKWQG74Ae91+Tc0fvT+I=";
    sparseCheckout = [ "driver" ];
  };

  nativeBuildInputs = kernel.moduleBuildDependencies;

  sourceRoot = "source/driver";

  buildPhase = ''
    runHook preBuild
    make -C ${kernel.dev}/lib/modules/${kernel.modDirVersion}/build \
      M=$PWD \
      modules
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    make -C ${kernel.dev}/lib/modules/${kernel.modDirVersion}/build \
      M=$PWD \
      INSTALL_MOD_PATH=$out \
      modules_install
    runHook postInstall
  '';

  meta = {
    description = "virtio-media V4L2 guest kernel driver (chromeos)";
    homepage = "https://github.com/chromeos/virtio-media";
    license = lib.licenses.gpl2Plus;
    platforms = lib.platforms.linux;
  };
}
