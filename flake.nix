{
  description = "CECLink firmware for the XIAO RP2350 HDMI CEC adapter";

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
      overlays.pulse8Cec = _final: _prev: {
        pulse8Cec = kernelPackages: kernelPackages.callPackage ./nix/pulse8-cec.nix { };
      };

      overlays.default = final: prev: {
        fwupd = prev.fwupd.overrideAttrs (old: {
          postInstall = (old.postInstall or "") + ''
            install -Dm644 ${./nix/ceclink.quirk} \
              "$out/share/fwupd/quirks.d/ceclink.quirk"
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
            pname = "ceclink-rp2350";
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
              cp target/${target}/release-with-debug/ceclink "$out/ceclink.elf"
              picotool uf2 convert "$out/ceclink.elf" "$out/ceclink.uf2"
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
