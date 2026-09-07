{ pkgs, ... }:

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
