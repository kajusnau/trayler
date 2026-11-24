{ lib, rustPlatform }:
rustPlatform.buildRustPackage {
  pname = "trayler";
  version = "0.1.0";
  src = lib.cleanSource ./.;
  cargoLock.lockFile = ./Cargo.lock;

  meta = {
    description = "Tray icon with a menu of commands for a running process";
    license = lib.licenses.gpl3Plus;
    mainProgram = "trayler";
    platforms = lib.platforms.linux;
  };
}
