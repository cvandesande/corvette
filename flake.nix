{
  description = "corvette -- Rust NVR work for frigate-vulkan, starting with the ncnn/Vulkan spike";

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
      # arm64 is unvalidated rather than unsupported; see docs/roadmap.md.
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          f (
            import nixpkgs {
              inherit system;
              overlays = [ (import rust-overlay) ];
            }
          )
        );

      # Same tag docker/Dockerfile.spike pins, so the Nix path and the
      # container path test the same ncnn.
      ncnnTag = "20260526";
    in
    {
      packages = forAllSystems (pkgs: rec {
        default = ncnn-spike;

        # Built from source rather than taken from nixpkgs: nixpkgs is on an
        # older tag, and this one has been soaked. System glslang, so no
        # submodules are needed.
        ncnn = pkgs.stdenv.mkDerivation {
          pname = "ncnn";
          version = ncnnTag;
          src = pkgs.fetchFromGitHub {
            owner = "Tencent";
            repo = "ncnn";
            rev = ncnnTag;
            hash = "sha256-4osyxUbnge4Zjecw0t+dJSDqfQf2u0vY3DwUVlluoxU=";
          };
          nativeBuildInputs = [ pkgs.cmake ];
          buildInputs = [
            pkgs.glslang
            pkgs.vulkan-headers
            pkgs.vulkan-loader
          ];
          cmakeFlags = [
            "-DNCNN_VULKAN=ON"
            "-DNCNN_SHARED_LIB=ON"
            "-DNCNN_SYSTEM_GLSLANG=ON"
            "-DNCNN_BUILD_EXAMPLES=OFF"
            "-DNCNN_BUILD_TOOLS=OFF"
            "-DNCNN_BUILD_BENCHMARK=OFF"
            "-DNCNN_BUILD_TESTS=OFF"
          ];
          # ncnn's ncnn.pc joins ''${prefix} with paths that are already
          # absolute, which nixpkgs rejects. See NixOS/nixpkgs#144170.
          postInstall = ''
            substituteInPlace $out/lib/pkgconfig/ncnn.pc \
              --replace-fail '=''${prefix}/' '='
          '';
          # ncnn dlopens libvulkan.so.1 instead of linking it, so there is no
          # DT_NEEDED entry and fixupPhase's RPATH shrinking drops the loader.
          # Without this every device enumeration comes back empty.
          postFixup = ''
            patchelf --add-rpath ${pkgs.lib.makeLibraryPath [ pkgs.vulkan-loader ]} \
              "$(readlink -f $out/lib/libncnn.so)"
          '';
        };

        ncnn-spike =
          let
            toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
            rustPlatform = pkgs.makeRustPlatform {
              cargo = toolchain;
              rustc = toolchain;
            };
          in
          rustPlatform.buildRustPackage {
            pname = "ncnn-spike";
            version = "0.1.0";
            src = self;
            cargoLock.lockFile = ./Cargo.lock;
            buildInputs = [
              ncnn
              pkgs.vulkan-loader
            ];
            # build.rs compiles csrc/c_api_ext.cpp against these headers and
            # records an rpath, so the binary needs no wrapper.
            env.NCNN_DIR = "${ncnn}";
            meta = {
              description = "Runs the frigate-vulkan detector's inference path through ncnn's C API from Rust";
              mainProgram = "ncnn-spike";
              platforms = systems;
            };
          };
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          # rust-toolchain.toml is the single source of truth; rustup users and
          # this shell cannot drift apart.
          packages = [
            (pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml)
            self.packages.${pkgs.stdenv.hostPlatform.system}.ncnn
            pkgs.clang-tools
            pkgs.cmake
            pkgs.gnumake
            pkgs.nixfmt
            pkgs.pkg-config
            pkgs.shellcheck
            pkgs.vulkan-loader
            pkgs.vulkan-tools
          ];
          NCNN_DIR = "${self.packages.${pkgs.stdenv.hostPlatform.system}.ncnn}";
        };
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt);
    };
}
