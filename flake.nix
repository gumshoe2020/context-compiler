{
  description = "context-compiler";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    crane.url = "github:ipetkov/crane";
    rust-overlay.url = "github:oxalica/rust-overlay";
    advisory-db = {
      url = "github:rustsec/advisory-db";
      flake = false;
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      crane,
      rust-overlay,
      advisory-db,
      ...
    }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      perSystem = forAllSystems (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ (import rust-overlay) ];
          };
          lib = pkgs.lib;
          rustToolchain = pkgs.rust-bin.stable."1.97.1".default.override {
            extensions = [
              "clippy"
              "rust-analyzer"
              "rust-src"
              "rustfmt"
            ];
          };
          craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;
          src = lib.cleanSourceWith {
            src = self;
            filter =
              path: type:
              let
                relativePath = lib.removePrefix "${self}/" (toString path);
              in
              (
                relativePath == "Cargo.toml"
                || relativePath == "Cargo.lock"
                || relativePath == "deny.toml"
                || relativePath == ".cargo"
                || lib.hasPrefix ".cargo/" relativePath
                || relativePath == "crates"
                || lib.hasPrefix "crates/" relativePath
                || relativePath == "prompts"
                || lib.hasPrefix "prompts/" relativePath
              )
              && !(lib.hasPrefix "fixtures/" relativePath)
              && !(lib.hasPrefix "expected/" relativePath)
              && !(lib.hasInfix "/tests/fixture_" relativePath);
          };
          commonArgs = {
            inherit src;
            cargoLock = ./Cargo.lock;
            pname = "context-compiler";
            strictDeps = true;
            version = "0.1.0";
          };
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
          testExclusions = [
            "the_real_rollups_parse_as_before_and_keep_their_no_compress_entries"
            "the_assistants_oracle"
            "the_improve_fixture_pairs_and_parses"
            "expected_files_equal_the_oracle"
            "the_port_matches_the_oracle"
            "the_graph_block_carries_the_hooks_lines"
            "missing_files_print_nothing_unless_strict"
            "a_held_database_exits_three_naming_it"
            "a_held_kg_is_a_retryable_error"
          ];
          skippedTests = lib.concatMapStringsSep " " (name: "--skip ${name}") testExclusions;
        in
        rec {
          packages = rec {
            default = context-compiler;
            context-compiler = craneLib.buildPackage (
              commonArgs
              // {
                inherit cargoArtifacts;
                doCheck = false;
                meta = {
                  description = "A memory compiler for coding and chat assistants";
                  license = lib.licenses.mit;
                  mainProgram = "palace";
                };
              }
            );
          };
          checks = {
            test = craneLib.cargoTest (
              commonArgs
              // {
                inherit cargoArtifacts;
                nativeBuildInputs = [ pkgs.python3 ];
                postPatch = ''
                  mkdir -p scripts
                  cp ${./scripts/mempalace-check.sh} scripts/mempalace-check.sh
                '';
                cargoExtraArgs = "--workspace --locked --no-default-features";
                cargoTestExtraArgs = "--jobs 8 -- ${skippedTests}";
              }
            );
            clippy = craneLib.cargoClippy (
              commonArgs
              // {
                inherit cargoArtifacts;
                cargoClippyExtraArgs = "--workspace --all-targets -- -D warnings";
              }
            );
            fmt = craneLib.cargoFmt (
              commonArgs
              // {
                cargoFmtExtraArgs = "--check";
                # rustfmt follows `mod doctors;` even under cfg(doctors). The public export omits
                # that private module, so give rustfmt an empty stand-in, inside this check only.
                preBuild = ''
                  if [ ! -e crates/palace-cli/src/doctors ]; then
                    mkdir -p crates/palace-cli/src/doctors
                    echo > crates/palace-cli/src/doctors/mod.rs
                    echo > crates/palace-cli/src/doctors/regress_fixtures.rs
                  fi
                '';
              }
            );
            deny = craneLib.mkCargoDerivation (
              commonArgs
              // {
                pname = "context-compiler-deny";
                cargoArtifacts = null;
                nativeBuildInputs = [
                  pkgs.cargo-deny
                  pkgs.git
                ];
                buildPhaseCargoCommand = ''
                  mkdir -p .cargo-home/advisory-dbs
                  db="$PWD/.cargo-home/advisory-dbs/advisory-db-3157b0e258782691"
                  cp -R ${advisory-db} "$db"
                  chmod -R u+w "$db"
                  git -C "$db" init -q
                  git -C "$db" add .
                  GIT_AUTHOR_DATE="@${toString advisory-db.lastModified} +0000" \
                  GIT_COMMITTER_DATE="@${toString advisory-db.lastModified} +0000" \
                    git -C "$db" -c user.name=Nix -c user.email=nix@localhost commit -qm "Pinned advisory snapshot"
                  cargo deny --offline --locked check --disable-fetch --config deny.toml
                '';
                installPhaseCommand = "mkdir -p $out";
                doCheck = false;
              }
            );
          };
          devShells.default = pkgs.mkShell {
            packages = [
              rustToolchain
              pkgs.cargo-deny
              pkgs.gitleaks
              pkgs.jq
              pkgs.python3
              pkgs.sqlite
              pkgs.uv
            ];
          };
          formatter = pkgs.nixfmt-rfc-style;
        }
      );
    in
    {
      packages = forAllSystems (system: perSystem.${system}.packages);
      checks = forAllSystems (system: perSystem.${system}.checks);
      devShells = forAllSystems (system: perSystem.${system}.devShells);
      formatter = forAllSystems (system: perSystem.${system}.formatter);
    };
}
