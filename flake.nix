{
  description = "recipes — a cooking recipe aggregator (Rust + SvelteKit)";

  inputs = {
    rainix.url = "github:rainlanguage/rainix";
    flake-utils.follows = "rainix/flake-utils";
  };

  outputs =
    {
      rainix,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        # rainix exposes its own (overlaid, pinned) nixpkgs as `pkgs` — reuse it
        # instead of pulling a second nixpkgs.
        pkgs = rainix.pkgs.${system};
        # rainix `rust-node-shell`: the org toolchain for a Rust backend plus a
        # node frontend — Rust 1.94 + cargo, Node 22, pre-commit, rainix-static.
        # No wasm toolchain: normalization runs server-side (see README), so
        # nothing is compiled to wasm32.
        # We layer on the rainix-curated prettier bundle (prettier +
        # plugin-svelte + plugin-tailwindcss) so the SvelteKit frontend formats
        # to the org standard; the shell doesn't export it by default.
        inherit (rainix.packages.${system}) prettier-bundle;
        mkTask = rainix.mkTask.${system};

        # Headless chromium carries no fonts. Without a fontconfig it renders
        # text invisibly (harfbuzz reports `font:''`, `glyph_count:0`) and can
        # take the renderer down with it. Supplying font *directories* alone is
        # not enough either: Tailwind's `font-sans` stack leads with
        # `ui-sans-serif`/`system-ui`, which match no installed family, so a
        # sans-serif UI silently falls back to DejaVu **Serif** and the capture
        # misrepresents the app. Alias the generics explicitly.
        fontsConf = pkgs.writeText "recipes-fonts.conf" ''
          <?xml version="1.0"?>
          <!DOCTYPE fontconfig SYSTEM "fonts.dtd">
          <fontconfig>
            <dir>${pkgs.dejavu_fonts}/share/fonts</dir>
            <dir>${pkgs.liberation_ttf}/share/fonts</dir>
            <cachedir prefix="xdg">fontconfig</cachedir>

            <alias><family>sans-serif</family><prefer><family>DejaVu Sans</family></prefer></alias>
            <alias><family>serif</family><prefer><family>DejaVu Serif</family></prefer></alias>
            <alias><family>monospace</family><prefer><family>DejaVu Sans Mono</family></prefer></alias>

            <alias><family>ui-sans-serif</family><prefer><family>DejaVu Sans</family></prefer></alias>
            <alias><family>system-ui</family><prefer><family>DejaVu Sans</family></prefer></alias>
            <alias><family>-apple-system</family><prefer><family>DejaVu Sans</family></prefer></alias>
            <alias><family>BlinkMacSystemFont</family><prefer><family>DejaVu Sans</family></prefer></alias>
            <alias><family>Segoe UI</family><prefer><family>DejaVu Sans</family></prefer></alias>
            <alias><family>Roboto</family><prefer><family>DejaVu Sans</family></prefer></alias>
            <alias><family>Helvetica Neue</family><prefer><family>DejaVu Sans</family></prefer></alias>
            <alias><family>Helvetica</family><prefer><family>DejaVu Sans</family></prefer></alias>
            <alias><family>Arial</family><prefer><family>DejaVu Sans</family></prefer></alias>
            <alias><family>ui-monospace</family><prefer><family>DejaVu Sans Mono</family></prefer></alias>
          </fontconfig>
        '';

        # Screenshot Storybook stories. Every story is its own URL, so a state is
        # *declared* rather than driven — capture needs no browser automation
        # (no puppeteer, no CDP), just navigate + `--screenshot`.
        #
        #   nix run .#storybook-shot              # every story
        #   nix run .#storybook-shot -- results   # only ids matching a regex
        #
        # Env: OUT_DIR, SB_DIR, PORT, WIDTH, HEIGHT, SCALE.
        storybook-shot = mkTask {
          name = "storybook-shot";
          # `chromium`, NOT `ungoogled-chromium` — the latter crashes headless
          # (crashpad/ptrace) in sandboxed environments.
          additionalBuildInputs = [
            pkgs.chromium
            pkgs.python3
            pkgs.jq
          ];
          body = ''
            #!/usr/bin/env bash
            set -euo pipefail

            SB_DIR="''${SB_DIR:-frontend/storybook-static}"
            OUT_DIR="''${OUT_DIR:-screenshots}"
            PORT="''${PORT:-6008}"
            WIDTH="''${WIDTH:-1280}"
            HEIGHT="''${HEIGHT:-720}"
            SCALE="''${SCALE:-2}"

            if [ ! -f "$SB_DIR/index.json" ]; then
              echo "no storybook build at $SB_DIR" >&2
              echo "build it first:  (cd frontend && npm ci && npm run build-storybook)" >&2
              exit 1
            fi

            export FONTCONFIG_FILE=${fontsConf}
            mkdir -p "$OUT_DIR"

            python3 -m http.server "$PORT" --directory "$SB_DIR" >/dev/null 2>&1 &
            server=$!
            trap 'kill $server 2>/dev/null || true' EXIT
            for _ in $(seq 1 40); do
              (exec 3<>/dev/tcp/127.0.0.1/"$PORT") 2>/dev/null && break
              sleep 0.25
            done

            # Story ids come from the build index, so stories added later are
            # picked up with no list to maintain here.
            ids=$(jq -r '.entries | keys[]' "$SB_DIR/index.json")
            if [ "$#" -gt 0 ]; then
              ids=$(printf '%s\n' "$ids" | grep -E "$1" || true)
            fi
            if [ -z "$ids" ]; then
              echo "no stories matched" >&2
              exit 1
            fi

            for id in $ids; do
              out="$OUT_DIR/$id.png"
              chromium \
                --headless --no-sandbox --disable-gpu --disable-dev-shm-usage \
                --hide-scrollbars \
                --window-size="$WIDTH,$HEIGHT" \
                --force-device-scale-factor="$SCALE" \
                --virtual-time-budget=15000 \
                --screenshot="$out" \
                "http://127.0.0.1:$PORT/iframe.html?id=$id&viewMode=story" \
                >/dev/null 2>&1
              echo "$out"
            done
          '';
        };

        # Full-page deterministic shots for visual regression. Unlike
        # storybook-shot (fixed viewport, for ad-hoc manual shots), this drives
        # puppeteer so every story is captured whole — a cropped page would let a
        # change below the fold land unreviewed, which is the whole thing the
        # visual fence exists to catch. Same pinned chromium + fonts, so a
        # baseline made here reproduces in CI.
        visual-shoot = mkTask {
          name = "visual-shoot";
          additionalBuildInputs = [
            pkgs.chromium
            pkgs.nodejs_22
          ];
          body = ''
            #!/usr/bin/env bash
            set -euo pipefail
            if [ ! -f frontend/storybook-static/index.json ]; then
              echo "no storybook build — (cd frontend && npm run build-storybook) first" >&2
              exit 1
            fi
            export CHROMIUM_BIN="${pkgs.chromium}/bin/chromium"
            export FONTCONFIG_FILE=${fontsConf}
            node frontend/scripts/visual-shoot.mjs
          '';
        };
      in
      {
        packages = {
          inherit storybook-shot visual-shoot;
        };

        devShells.default = rainix.devShells.${system}.rust-node-shell.overrideAttrs (old: {
          # Turso CLI (from rainix's exposed pkgs) for DB provisioning +
          # migrations, reproducibly via `nix develop`. storybook-shot puts the
          # screenshot harness on PATH. rclone uploads the shots it takes to R2
          # — the README documents it as *the* way (awscli's TLS fails against
          # R2 here, and curl 7.81's --aws-sigv4 omits R2's required
          # x-amz-content-sha256), so the shell has to actually provide it.
          buildInputs = (old.buildInputs or [ ]) ++ [
            pkgs.turso-cli
            pkgs.rclone
            storybook-shot
          ];
          shellHook = (old.shellHook or "") + ''
            export RAINIX_PRETTIER_BUNDLE_DIR=${prettier-bundle}
          '';
        });
      }
    );
}
