{
  description = "XIAO RP2350 HDMI CEC adapter firmware";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forSystems =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          f (
            import nixpkgs {
              inherit system;
              overlays = [
                rust-overlay.overlays.default
                self.overlays.default
              ];
            }
          )
        );
      target = "thumbv8m.main-none-eabihf";
    in
    {
      overlays.default = final: prev: {
        fwupd = prev.fwupd.overrideAttrs (old: {
          postInstall = (old.postInstall or "") + ''
            install -Dm644 ${./nix/cec-4k.quirk} \
              "$out/share/fwupd/quirks.d/cec-4k.quirk"
          '';
        });
      };

      nixosModules.fwupd = { ... }: {
        nixpkgs.overlays = [ self.overlays.default ];
        services.fwupd.enable = true;
      };

      formatter = forSystems (
        pkgs:
        pkgs.writeShellApplication {
          name = "treefmt";
          runtimeInputs = [
            pkgs.treefmt
            pkgs.nixfmt
            pkgs.rustfmt
          ];
          text = ''exec treefmt --config-file ${./treefmt.toml} "$@"'';
        }
      );

      devShells = forSystems (
        pkgs:
        let
          rust = pkgs.rust-bin.stable."1.98.1".minimal.override {
            targets = [ target ];
            extensions = [
              "rustfmt"
              "clippy"
            ];
          };
        in
        {
          default = pkgs.mkShell {
            packages = [
              rust
              pkgs.picotool
              pkgs.elf2uf2-rs
            ];
            shellHook = ''
              echo 'Build: cargo build --profile release-with-debug'
            '';
          };
        }
      );

      packages = forSystems (
        pkgs:
        let
          rust = pkgs.rust-bin.stable."1.98.1".minimal.override { targets = [ target ]; };
          rustPlatform = pkgs.makeRustPlatform {
            cargo = rust;
            rustc = rust;
          };
          firmware = rustPlatform.buildRustPackage {
            pname = "cec-4k-rp2350";
            version = "0.1.0";
            src = self;
            cargoLock.lockFile = ./Cargo.lock;
            auditable = false;
            doCheck = false;
            nativeBuildInputs = [ pkgs.picotool ];
            buildPhase = ''
              runHook preBuild
              cargo build --frozen --target ${target} --profile release-with-debug
              runHook postBuild
            '';
            installPhase = ''
              runHook preInstall
              mkdir -p "$out"
              cp target/${target}/release-with-debug/cec-4k "$out/cec-4k.elf"
              picotool uf2 convert "$out/cec-4k.elf" "$out/cec-4k.uf2"
              runHook postInstall
            '';
          };
        in
        {
          default = firmware;
          firmware = firmware;
          fwupd = pkgs.fwupd;
        }
      );
    };
}
