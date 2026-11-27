{
  lib,
  stdenv,
  rustPlatform,
  installShellFiles,
}:
rustPlatform.buildRustPackage {
  pname = "trayler";
  version = "0.2.2";
  src = lib.cleanSource ./.;
  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = [ installShellFiles ];

  # The generators run the built binary, which a cross build cannot execute.
  postInstall = lib.optionalString (stdenv.buildPlatform.canExecute stdenv.hostPlatform) ''
    installShellCompletion --cmd trayler \
      --bash <(TRAYLER_GENERATE=bash $out/bin/trayler) \
      --zsh <(TRAYLER_GENERATE=zsh $out/bin/trayler) \
      --fish <(TRAYLER_GENERATE=fish $out/bin/trayler)
    TRAYLER_GENERATE=man $out/bin/trayler > trayler.1
    installManPage trayler.1
  '';

  meta = {
    description = "A configurable tray icon and menu for any process";
    license = lib.licenses.gpl3Plus;
    mainProgram = "trayler";
    platforms = lib.platforms.linux;
  };
}
