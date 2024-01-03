{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    systems.url = "github:nix-systems/default-linux";
  };

  outputs = { self, nixpkgs, rust-overlay, systems, ... }:
    let
      eachSystem = nixpkgs.lib.genAttrs (import systems);

      mkRexoder = { rustPlatform, lib, pkg-config, fuse3, ... }: rustPlatform.buildRustPackage rec {
        pname = "redoxer";
        version = "0.2.38";

        src = builtins.path {
          path = ./.;
          name = pname;
        };
        cargoLock.lockFile = ./Cargo.lock;

        meta = {
          description = "The tool used to build/run Rust programs (and C/C++ programs with zero dependencies) inside of a Redox VM.";
          homepage = "https://gitlab.redox-os.org/redox-os/redoxer";
        };

        nativeBuildInputs = [
          pkg-config
        ];

        buildInputs = [
          fuse3
        ];
      };
    in
    {
      apps = eachSystem (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/redoxer";
        };
      });

      packages = eachSystem (system:
        let
          pkgs = import nixpkgs {
            inherit system;
          };
        in
        {
          default = pkgs.callPackage mkRexoder { };
        });

      overlays.default = final: prev: {
        redoxer = prev.callPackage mkRexoder { };
      };

      devShells = eachSystem (system:
        let
          pkgs = import nixpkgs {
            inherit system;

            overlays = [ rust-overlay.overlays.default ];
          };

          rust-toolchain = (pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml).override {
            extensions = [ "rust-src" "rust-analyzer" ];
          };
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              pkg-config
              fuse3
            ] ++ [ rust-toolchain ];
          };
        });
    };
}
