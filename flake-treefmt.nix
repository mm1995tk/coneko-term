_: {
  projectRootFile = "flake.nix";
  programs.nixfmt.enable = true;
  programs.taplo.enable = true;
  programs.rustfmt = {
    enable = true;
    edition = "2021";
  };
  # TypeScript と、根に置いたその設定。Markdown には触れない
  programs.prettier = {
    enable = true;
    includes = [
      "package.json"
      "tsconfig.json"
      "src/**/*.ts"
      "test/**/*.ts"
    ];
    excludes = [ "src/wasm/*" ];
  };
  # package-lock.json は npm が書く。整形すると次の npm install で差分が出る。
  # src/wasm は wasm-bindgen が書く
  settings.global.excludes = [
    "package-lock.json"
    "src/wasm/*"
  ];
}
