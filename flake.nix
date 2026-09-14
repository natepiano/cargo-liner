# The development shell for this crate: the native libraries Bevy links or
# opens at run time on Linux, declared once here and entered by `nix develop`,
# by direnv through .envrc, and by CI through `nix develop -c`. The Rust
# toolchain is not in it -- rustup's is used everywhere, as before.
{
  description = "cargo-liner development shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        # cargo-tile's shim and reader tests run `ps`. On a non-NixOS host the
        # shell's LD_LIBRARY_PATH hands the host's procps nix's libsystemd,
        # whose RUNPATH loads nix glibc beside the host libc and `ps` dies on
        # a missing symbol version. procps from nixpkgs shares that glibc. It
        # is added here so the vendored bevy-shell.nix stays byte-identical.
        devShells.default = (import ./nix/bevy-shell.nix { inherit pkgs; }).overrideAttrs (shell: {
          nativeBuildInputs =
            shell.nativeBuildInputs ++ pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.procps ];
        });
      }
    );
}
