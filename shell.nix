{ pkgs ? import (fetchTarball "https://github.com/NixOS/nixpkgs/archive/nixos-25.11.tar.gz") {} }:

pkgs.mkShell {
  buildInputs = [
    pkgs.rustup
    pkgs.protobuf
    pkgs.opkg-utils
    pkgs.jq
    # cargo-cross can be used once version > 0.2.5, as 0.2.5 does not work well
    # with nightly toolchain. It is for now installed through make dev-dependencies.
    # pkgs.cargo-cross
    pkgs.cargo-deb
  ];
  shellHook = ''
    export PATH=$PATH:~/.cargo/bin
  '';
  DOCKER_BUILDKIT = "1";
  NIX_STORE = "/nix/store";
}
