{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        rustToolchain = pkgs.rust-bin.stable."1.98.0".default.override {
          extensions = [
            "rust-src"
            "llvm-tools"
          ];
          targets = [ "wasm32-unknown-unknown" ];
        };

        rustfmtNightly = pkgs.rust-bin.nightly.latest.rustfmt;

        alto = pkgs.callPackage ./nix/alto/package.nix { };
        tornadocashRelayer = pkgs.callPackage ./nix/tornadocash-relayer/package.nix { };
      in
      {
        packages = {
          inherit alto tornadocashRelayer;
        };

        devShells = {
          default = pkgs.mkShell {
            packages = with pkgs; [
              rustfmtNightly
              rustToolchain
              rust-analyzer
              bacon
              cargo-audit
              cargo-autoinherit
              cargo-machete
              cargo-sort
              cargo-insta
              cargo-semver-checks
              git-cliff

              # JS tooling
              wasm-bindgen-cli_0_2_108
              nodejs_26

              # Solidity tooling
              foundry
              alto
              tornadocashRelayer
              redis

              just
              sops

              # alloy's hyper feature pulls hyper-tls, which links OpenSSL on Linux.
              pkg-config
              openssl
            ];
          };

          ci = pkgs.mkShell {
            packages = with pkgs; [
              rustfmtNightly
              rustToolchain
              cargo-audit

              # JS tooling
              wasm-bindgen-cli_0_2_108
              nodejs_26

              # Solidity tooling
              foundry
              alto
              tornadocashRelayer
              redis

              # alloy's hyper feature pulls hyper-tls, which links OpenSSL on Linux.
              pkg-config
              openssl
            ];
          };
        };
      }
    );
}
