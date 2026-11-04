{
  description = "coneko-term: terminal building blocks (VT parser, screen model) in Rust, for WebAssembly and native";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
    treefmt-nix.url = "github:numtide/treefmt-nix";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
      treefmt-nix,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        project-name = "mm1995tk/coneko-term";
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        treefmtEval = treefmt-nix.lib.evalModule pkgs ./flake-treefmt.nix;

        # Rust と wasm32 の target。nixpkgs の rustc には wasm32 の標準ライブラリが入らないので rust-overlay を使う
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          targets = [ "wasm32-unknown-unknown" ];
        };
        rustPlatform = pkgs.makeRustPlatform {
          cargo = rustToolchain;
          rustc = rustToolchain;
        };

        packages = import ./flake-packages.nix { inherit pkgs rustPlatform; };
      in
      {
        packages = {
          default = packages.npm;
          inherit (packages) wasm npm;
          treefmt = treefmtEval.config.build.configFile;
        };

        devShells = import ./flake-shell.nix { inherit pkgs project-name rustToolchain; };
        formatter = treefmtEval.config.build.wrapper;

        # nix flake check で回す検査。CI（check.yml）も同じものを回す
        checks = {
          formatting = treefmtEval.config.build.check self;
          # cargo test（全 crate）
          cargo-test = packages.cargoTest;
          # npm パッケージを作り、その中で TypeScript のテスト（npm test）を回す
          npm = packages.npm;
        };
      }
    );
}
