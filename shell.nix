{ pkgs ? import (fetchTarball "https://github.com/NixOS/nixpkgs/archive/nixos-23.05.tar.gz") {} }:

pkgs.mkShell {
  buildInputs = [
    pkgs.rustup
    pkgs.protobuf
    pkgs.opkg-utils
    pkgs.jq
    pkgs.cargo-cross
    pkgs.cargo-deb
  ];
  DOCKER_BUILDKIT = "1";
  NIX_STORE = "/nix/store";
}