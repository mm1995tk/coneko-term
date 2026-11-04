{
  pkgs,
  project-name,
  rustToolchain,
  ...
}:
{
  default = pkgs.mkShell {
    buildInputs = [
      rustToolchain
      # Cargo.lock の wasm-bindgen と同じ版にそろえる（flake-packages.nix を見よ）
      pkgs.wasm-bindgen-cli
      pkgs.nodejs_24
      pkgs.git
    ];

    PROJECT_NAME = project-name;
  };
}
