{
  description = "recipes — a cooking recipe aggregator (Rust + SvelteKit)";

  inputs = {
    rainix.url = "github:rainlanguage/rainix";
    flake-utils.follows = "rainix/flake-utils";
    nixpkgs.follows = "rainix/nixpkgs";
  };

  outputs =
    {
      rainix,
      flake-utils,
      nixpkgs,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
        # rainix `wasm-shell`: the org toolchain for a Rust crate compiled to
        # WASM plus a node frontend — Rust 1.94 + cargo + wasm-pack, Node 22,
        # pre-commit, rainix-static. We compile the shared recipe-core crate to
        # wasm32 for in-browser processing AND native for the backend proxy.
        # We layer on the rainix-curated prettier bundle (prettier +
        # plugin-svelte + plugin-tailwindcss) so the SvelteKit frontend formats
        # to the org standard; wasm-shell doesn't export it by default.
        inherit (rainix.packages.${system}) prettier-bundle;
      in
      {
        devShells.default = rainix.devShells.${system}.wasm-shell.overrideAttrs (old: {
          # Turso CLI for DB provisioning + migrations, reproducibly via
          # `nix develop` (follows rainix's nixpkgs — no second nixpkgs pulled).
          buildInputs = (old.buildInputs or [ ]) ++ [ pkgs.turso-cli ];
          shellHook = (old.shellHook or "") + ''
            export RAINIX_PRETTIER_BUNDLE_DIR=${prettier-bundle}
          '';
        });
      }
    );
}
