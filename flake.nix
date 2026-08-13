{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      treefmt-nix,
      ...
    }:
    let
      systems = [
        "i686-linux"
        "x86_64-linux"
        "aarch64-linux"
      ];

      overlays = [ rust-overlay.overlays.default ];

      pkgsFor =
        system:
        import nixpkgs {
          inherit system overlays;
        };

      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f (pkgsFor system));
      treefmtEval = forAllSystems (pkgs: treefmt-nix.lib.evalModule pkgs ./treefmt.nix);
    in
    {
      packages = forAllSystems (
        pkgs:
        let
          manifest = (pkgs.lib.importTOML ./Cargo.toml).package;

          rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

          rustPlatform = pkgs.makeRustPlatform {
            cargo = rustToolchain;
            rustc = rustToolchain;
          };
        in
        {
          default = rustPlatform.buildRustPackage (finalAttrs: {
            pname = manifest.name;
            version = manifest.version;
            src = pkgs.lib.cleanSource ./.;
            cargoLock.lockFile = ./Cargo.lock;

            meta = {
              description = manifest.description;
              homepage = manifest.repository;
            };

            nativeBuildInputs = with pkgs; [
              pkg-config
            ];

            buildInputs = with pkgs; [
              fuse3
            ];
          });
        }
      );

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [ self.packages.${pkgs.stdenv.hostPlatform.system}.default ];

          packages = with pkgs; [
            nixd
          ];
        };
      });

      formatter = forAllSystems (
        pkgs: treefmtEval.${pkgs.stdenv.hostPlatform.system}.config.build.wrapper
      );

      checks = forAllSystems (pkgs: {
        formatting = treefmtEval.${pkgs.stdenv.hostPlatform.system}.config.build.check self;
      });

    };
}
