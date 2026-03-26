{
  description = "sema-codegen — per-domain triad → Cap'n Proto codegen";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    criome-cozo = { url = "github:LiGoldragon/criome-cozo"; flake = false; };
    sema-core = { url = "github:LiGoldragon/sema-core"; flake = false; };
    sema = { url = "github:LiGoldragon/sema"; flake = false; };
  };

  outputs = { self, nixpkgs, flake-utils, crane, fenix, criome-cozo, sema-core, sema, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        rustToolchain = fenix.packages.${system}.latest.toolchain;
        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        src = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter = craneLib.filterCargoSources;
        };

        commonArgs = {
          inherit src;
          pname = "sema-codegen";
          postUnpack = ''
            mkdir -p $sourceRoot/flake-crates
            cp -rL ${criome-cozo} $sourceRoot/flake-crates/criome-cozo
            cp -rL ${sema-core} $sourceRoot/flake-crates/sema-core
            cp -rL ${sema} $sourceRoot/flake-crates/sema
          '';
          nativeBuildInputs = [ pkgs.capnproto ];
        };
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;
      in
      {
        packages.default = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
        });

        checks = {
          build = craneLib.buildPackage (commonArgs // {
            inherit cargoArtifacts;
          });
          tests = craneLib.cargoTest (commonArgs // {
            inherit cargoArtifacts;
          });
        };

        devShells.default = craneLib.devShell {
          packages = with pkgs; [ rust-analyzer sqlite capnproto jujutsu ];
        };
      }
    );
}
