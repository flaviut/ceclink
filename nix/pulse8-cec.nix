{
  stdenv,
  lib,
  kernel,
  kernelModuleMakeFlags,
}:

stdenv.mkDerivation {
  pname = "pulse8-cec";
  inherit (kernel) version;

  # Build only the driver; the matching stock kernel already provides cec.ko.
  dontUnpack = true;
  nativeBuildInputs = kernel.moduleBuildDependencies;
  hardeningDisable = [ "pic" ];

  buildPhase = ''
    runHook preBuild
    tar -xOf ${kernel.src} \
      linux-${kernel.version}/drivers/media/cec/usb/pulse8/pulse8-cec.c \
      > pulse8-cec.c
    echo 'obj-m := pulse8-cec.o' > Makefile
    make ${lib.escapeShellArgs kernelModuleMakeFlags} \
      -C ${kernel.dev}/lib/modules/${kernel.modDirVersion}/build \
      M="$PWD" modules
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    install -Dm644 pulse8-cec.ko \
      "$out/lib/modules/${kernel.modDirVersion}/extra/pulse8-cec.ko"
    runHook postInstall
  '';
}
