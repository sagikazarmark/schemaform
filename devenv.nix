{ pkgs, lib, ... }:

{
  # dotenv.enable = true;

  dagger.enable = true;
  env.DAGGER_X_RELEASE = "v1.0.0-beta.11";

  # Required by arborium
  env.CC_wasm32_unknown_unknown = "${pkgs.llvmPackages.clang-unwrapped}/bin/clang";

  packages = with pkgs; [
    lld
    cargo-audit
    cargo-deny
    cargo-dist
    cargo-release
    cargo-watch

    # Browser test suite (README "Development"). The wasm-bindgen-cli version
    # must match the `wasm-bindgen` version in Cargo.lock; bump both together.
    wasm-bindgen-cli_0_2_126
    firefox
    geckodriver
  ] ++ lib.optionals stdenv.isLinux [
    # Desktop demo (`dx serve --platform desktop` from `demo/`, see
    # testing/desktop-smoke.md). The Linux WebView is WebKitGTK: `wry` and `tao`
    # link GTK 3, WebKitGTK 4.1, libsoup 3, OpenSSL, and libxdo through
    # pkg-config. macOS and Windows use the system WebView and need none of this.
    pkg-config
    gtk3
    webkitgtk_4_1
    libsoup_3
    openssl
    xdotool
    # Runs the desktop demo without a display for the checklist's headless
    # launch step.
    xvfb-run
  ];

  languages = {
    rust = {
      enable = true;
      channel = "stable";
      targets = [ "wasm32-unknown-unknown" ];
    };

    javascript = {
      enable = true;

      npm.enable = true;
    };
  };
}
