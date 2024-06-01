{
  description = "A very basic flake";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, rust-overlay }:
    let 
      system = "x86_64-linux";
      overlays = [(import rust-overlay)];
      pkgs = import nixpkgs {
        inherit system overlays;
      };
      rustPlatform = with pkgs; makeRustPlatform {
        cargo = rust-bin.selectLatestNightlyWith (toolchain: toolchain.default);
        rustc = rust-bin.selectLatestNightlyWith (toolchain: toolchain.default);
      };
    in
    {
      packages.${system}.qed = rustPlatform.buildRustPackage (with pkgs; {
        pname = "qed";
        version = "0.1.0";
        cargoLock.lockFile = ./Cargo.lock;
        src = pkgs.lib.cleanSource ./.;

        PKG_CONFIG_PATH = "${openssl.dev}/lib/pkgconfig";

        nativeBuildInputs = [
          pkg-config
        ];

        buildInputs = [
          openssl
        ];
      });

      devShells.${system}.default =
        with pkgs;
        mkShell {
          LD_LIBRARY_PATH="${openssl.out}/lib";

          nativeBuildInputs = [
            atlas
            turso-cli
            flyctl
            pkg-config
          ];

          buildInputs = [
            openssl
          ];
      };
    };
}
