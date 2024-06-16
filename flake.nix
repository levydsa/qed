{
  description = "A very basic flake";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    flake-utils = {
      url = "github:numtide/flake-utils";
    };
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
    };
    crane = {
      url = "github:ipetkov/crane";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs = { self, nixpkgs, rust-overlay, flake-utils, crane }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        fileset = pkgs.lib.fileset;
        rust = pkgs.rust-bin.nightly.latest.default;
        craneLib = (crane.mkLib pkgs).overrideToolchain rust;
        nodeModules = pkgs.buildNpmPackage {
          name = "qed-web-node";
          src = fileset.toSource {
            root = ./.;
            fileset = fileset.unions [
              ./package.json
              ./package-lock.json
            ];
          };

          npmDepsHash = "sha256-FPngX8x71AW7Zvqs9LPVf1FuJEMt9FlxLnGzhtDBYf0=";
          dontBuild = true;

          installPhase = ''
            cp -r node_modules $out/
          '';
        };
        fileSetForCrate = crate: fileset.toSource {
          root = ./.;
          fileset = fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./crates
            crate
          ];
        };
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };
      in
      rec {
        formatter = pkgs.nixpkgs-fmt;
        packages.default = packages.qed-web;
        packages.qed-web = craneLib.buildPackage {
          src = fileSetForCrate ./crates/qed-web;
          cargoToml = ./crates/qed-web/Cargo.toml;
          cargoExtraArgs = "-p ${packages.qed-web.pname} -vv --locked";

          strictDeps = true;

          LD_LIBRARY_PATH = "${pkgs.openssl.out}/lib";

          nativeBuildInputs = with pkgs; [
            pkg-config
          ];

          buildInputs = with pkgs; [
            openssl
          ];
        };
        packages.container = pkgs.dockerTools.buildImage {
          name = "qed-web";
          tag = packages.qed-web.version;
          created = "now";

          copyToRoot =
            (pkgs.buildEnv {
              name = "image-root";
              pathsToLink = [ "/bin" "/var" "/etc" ];
              paths = [
                packages.qed-web
                pkgs.cacert
                (
                  (pkgs.runCommand "var"
                    {
                      src = (fileset.toSource {
                        root = ./.;
                        fileset = fileset.unions [
                          ./package.json
                          ./package-lock.json
                          ./global.css
                          ./tailwind.config.js
                          ./justfile
                        ];
                      });
                      buildInputs = with pkgs; [ coreutils just fd nodejs_22 ];
                    }
                    (
                      # TODO: Make building the tailwind bundle a dependency too
                      ''
                        mkdir -p $out

                        ls -la ${nodeModules}

                        mkdir -p \
                          $out/var/ \
                          $out/var/assets/js/

                        cp -R ${nodeModules}/.     $out/var/node_modules
                        cp $src/package.json       $out/var/
                        cp $src/package-lock.json  $out/var/
                        cp $src/tailwind.config.js $out/var/

                        cp -r ${./assets}/.    $out/var/assets
                        cp -r ${./content}/.   $out/var/content
                        cp -r ${./templates}/. $out/var/templates

                        cd $out/var/

                        ls -la .
                        npx tailwindcss -c tailwind.config.js -i ${./global.css} -o assets/tw.css

                        cd $src

                        cp -r ${nodeModules}/htmx.org/dist/.        $out/var/assets/js
                        cp -r ${nodeModules}/hyperscript.org/dist/. $out/var/assets/js
                        cp -r ${nodeModules}/katex/dist/.           $out/var/assets/js

                      ''
                    ))
                )
              ];
            });

          config = {
            Cmd = [ "/bin/qed-web" ];
            WorkingDir = "/var";
          };
        };

        devShells.default =
          with pkgs;
          mkShell {
            LD_LIBRARY_PATH = "${openssl.out}/lib";
            PATH = "./node_modules/.bin/:$PATH";

            nativeBuildInputs = [
              nodejs_22
              atlas
              turso-cli
              flyctl
            ];

            buildInputs = [
              pkg-config
              openssl
            ];

            shellHook = ''
              export PATH="./node_modules/.bin/:$PATH"
            '';
          };
      });
}
