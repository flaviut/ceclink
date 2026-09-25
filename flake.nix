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
      overlays.default = final: prev: {
        pulse8Cec = kernelPackages: kernelPackages.callPackage ./nix/pulse8-cec.nix { };
        fwupd = prev.fwupd.overrideAttrs (old: {
          postInstall = (old.postInstall or "") + ''
            install -Dm644 ${./nix/ceclink.quirk} \
              "$out/share/fwupd/quirks.d/ceclink.quirk"
          '';
        });
      };

      nixosModules.default = { pkgs, config, ... }: {
        nixpkgs.overlays = [ self.overlays.default ];
        environment.systemPackages = [ pkgs.v4l-utils ];
        services.fwupd.enable = true;
        boot.kernelModules = [ "pulse8-cec" ];
        boot.extraModulePackages = [
          (pkgs.pulse8Cec config.boot.kernelPackages)
        ];

        services.udev.extraRules = ''
          SUBSYSTEM=="tty", KERNEL=="ttyACM[0-9]*", ATTRS{idVendor}=="2548", ATTRS{idProduct}=="1001", ACTION=="add", TAG+="systemd", ENV{SYSTEMD_WANTS}+="pulse8-cec-inputattach@%k.service"
          SUBSYSTEM=="tty", KERNEL=="ttyACM[0-9]*", ENV{ID_VENDOR_ID}=="2548", ENV{ID_MODEL_ID}=="1002", ENV{ID_USB_INTERFACE_NUM}=="00", ACTION=="add", TAG+="systemd", ENV{SYSTEMD_WANTS}+="pulse8-cec-inputattach@%k.service"
          SUBSYSTEM=="tty", KERNEL=="ttyACM[0-9]*", ENV{ID_VENDOR_ID}=="2548", ENV{ID_MODEL_ID}=="1002", ENV{ID_USB_INTERFACE_NUM}=="02", ACTION=="add", SYMLINK+="ceclink-debug", GROUP="video", MODE="0660"
        '';
        systemd.services."pulse8-cec-inputattach@" = {
          description = "Attach Pulse-Eight CEC adapter on %I";
          serviceConfig = {
            Type = "simple";
            ExecStart = "${pkgs.linuxConsoleTools}/bin/inputattach --pulse8-cec /dev/%I";
          };
        };
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
