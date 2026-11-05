{ pkgs, rustPlatform }:
let
  inherit (pkgs) lib;
  version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;

  rustSrc = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./Cargo.toml
      ./Cargo.lock
      ./crates
    ];
  };

  # Rust を WebAssembly にして、wasm-bindgen の出力（.js / .d.ts / .wasm）を $out に置く。
  # wasm-bindgen-cli は nixpkgs のもので、Cargo.lock の wasm-bindgen と同じ版でなければならない
  wasm = rustPlatform.buildRustPackage {
    pname = "coneko-term-wasm";
    inherit version;
    src = rustSrc;
    cargoLock.lockFile = ./Cargo.lock;
    nativeBuildInputs = [ pkgs.wasm-bindgen-cli ];
    doCheck = false;
    buildPhase = ''
      runHook preBuild
      cargo build --frozen --release --target wasm32-unknown-unknown -p coneko-term-wasm
      wasm-bindgen --target web --out-dir out \
        target/wasm32-unknown-unknown/release/coneko_term_wasm.wasm
      runHook postBuild
    '';
    installPhase = ''
      runHook preInstall
      cp -r out $out
      runHook postInstall
    '';
  };

  # cargo test（本体と WASM のバインディング。ホストの target で回す）
  cargoTest = rustPlatform.buildRustPackage {
    pname = "coneko-term-cargo-test";
    inherit version;
    src = rustSrc;
    cargoLock.lockFile = ./Cargo.lock;
    doCheck = true;
    installPhase = "touch $out";
    buildPhase = "true";
  };

  # npm パッケージ（tarball）。ビルド済みの .wasm を dist/wasm に入れる。
  # インストール時のスクリプトに頼らないので、npm --ignore-scripts で入れても動く。
  # 作る途中で npm test（TypeScript のテスト）も回す
  npm = pkgs.buildNpmPackage {
    pname = "coneko-term";
    inherit version;
    src = lib.fileset.toSource {
      root = ./.;
      fileset = lib.fileset.unions [
        ./package.json
        ./package-lock.json
        ./tsconfig.json
        ./tsconfig.test.json
        ./src/animation.ts
        ./src/images.ts
        ./src/index.ts
        ./src/input.ts
        ./src/keyboard.ts
        ./src/kitty-keyboard.ts
        ./src/memory.ts
        ./src/renderer.ts
        ./src/selection.ts
        ./src/terminal.ts
        ./test
        ./LICENSE
      ];
    };
    # 依存は package-lock.json の integrity から取る。ハッシュを別に持たないので、
    # release.yml が版を上げて package-lock.json が変わっても、ここを直さなくてよい
    npmDeps = pkgs.importNpmLock {
      npmRoot = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./package.json
          ./package-lock.json
        ];
      };
    };
    npmConfigHook = pkgs.importNpmLock.npmConfigHook;
    npmFlags = [ "--ignore-scripts" ];
    postPatch = ''
      mkdir -p src/wasm
      cp ${wasm}/* src/wasm/
      chmod -R u+w src/wasm
    '';
    npmBuildScript = "build:ts";
    doCheck = true;
    checkPhase = ''
      runHook preCheck
      npm test
      runHook postCheck
    '';
    installPhase = ''
      runHook preInstall
      mkdir -p $out
      npm_config_cache=$TMPDIR/npm-cache npm pack --ignore-scripts --pack-destination $out
      # 配る tarball に .wasm と .d.ts が入っていること（prepack は --ignore-scripts では走らない）
      # （`grep -q` が先に終わって tar が SIGPIPE で落ちないよう、一覧をいったんファイルに置く）
      tar tzf $out/*.tgz > $TMPDIR/tarball-files
      for f in dist/index.d.ts dist/wasm/coneko_term_wasm_bg.wasm dist/wasm/coneko_term_wasm.d.ts; do
        grep -qx "package/$f" $TMPDIR/tarball-files
      done
      runHook postInstall
    '';
    dontNpmInstall = true;
  };
in
{
  inherit wasm cargoTest npm;
}
