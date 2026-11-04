# 公開の手順

coneko-term は crate（crates.io）と npm パッケージ（npm）を、同じ版で一緒に公開する。
git のタグ（`v0.1.0` の形）を push すると、GitHub Actions の [`release.yml`](.github/workflows/release.yml) が両方に公開する。
長く使えるトークンは置かない。どちらも trusted publishing（GitHub Actions の OIDC）で認証する。

## 公開の流れ（自動）

`release.yml` はタグを push すると次の順に動く。

1. `verify`: タグと `Cargo.toml`・`package.json` の版がそろっていることを確かめ、`nix flake check` を回す。
2. `crates-io`: `cargo publish -p coneko-term`。
3. `npm`: `nix build .#npm` で作った tarball を `npm publish`（provenance 付き）。`nix flake check` でテスト済みの tarball をそのまま配る。
4. `github-release`: 2 と 3 が通ったら、自動生成のリリースノートで GitHub のリリースを作る。

`crates-io` と `npm` は、その版が公開済みなら飛ばす。途中で落ちたら、Actions の画面で失敗したジョブだけ再実行できる。
`v0.2.0-rc.1` のようにハイフンの入ったタグは、npm では dist-tag `next`（`latest` にならない）、GitHub では pre-release になる。

CI（`check.yml`）の `publish-dry-run` ジョブは、PR ごとに `cargo publish --dry-run` と `npm publish --dry-run` を回す。
タグを打つ前に、受け取れる形かどうかが分かる。

## 新しい版を出す

1. `Cargo.toml` の `[workspace.package]` の `version` と、`package.json` の `version` を同じ値にする。
2. `cargo check` で `Cargo.lock` を更新する（`coneko-term` の版がそこにもある）。
3. PR を出して main に入れる。
4. main で `git tag v0.1.0 && git push origin v0.1.0`。
5. Actions の「リリース」が全部通ったことを確かめる。

手元で確かめるときは、次を使う。

```sh
sh scripts/check-release-version.sh v0.1.0     # 版とタグがそろっているか
cargo publish -p coneko-term --locked --dry-run
nix build .#npm --out-link npm-package && npm publish ./npm-package/*.tgz --access public --dry-run
```

## ユーザーがやること（最初の 1 回）

どちらのレジストリも、trusted publisher を登録するには、そのパッケージが先に存在している必要がある。
そのため、最初の 1 回だけは手で公開する。2 回目からはタグだけで公開できる。

### 1. GitHub に environment `release` を作る

Settings → Environments → New environment で `release` を作る（任意で必須レビュアーや、タグだけを許す制限を付ける）。
`release.yml` の `crates-io` と `npm` のジョブがこれを使う。名前は、下の trusted publisher の登録と同じにする。

### 2. crates.io

1. crates.io にログインし、Account Settings → API Tokens で、`publish-new` と `publish-update` の権限を持つ短命のトークンを作る。
2. 手元で最初の版を公開する。

   ```sh
   nix develop -c cargo publish -p coneko-term --locked --token <作ったトークン>
   ```

3. 公開できたら、そのトークンは revoke（失効）する。
4. crates.io の `coneko-term` のページ → Settings → Trusted Publishing → Add で、GitHub の設定を次のように登録する。
   - Repository owner: `mm1995tk`
   - Repository name: `coneko-term`
   - Workflow filename: `release.yml`
   - Environment: `release`
5. 余裕があれば「Enforce trusted publishing」を有効にして、トークンでの公開を止める。

### 3. npm

1. 手元で最初の版を公開する（`npm login` 済みで、2FA を使える状態で）。

   ```sh
   nix build .#npm --out-link npm-package
   nix develop -c npm publish ./npm-package/*.tgz --access public
   ```

2. npmjs.com の `coneko-term` のページ → Settings → Trusted Publisher で GitHub Actions を選び、次を登録する。
   - Organization or user: `mm1995tk`
   - Repository: `coneko-term`
   - Workflow filename: `release.yml`
   - Environment name: `release`
3. 余裕があれば、同じ Settings の Publishing access で「Require two-factor authentication and disallow tokens」にする。

npm の trusted publishing は npm CLI 11.5.1 以上が要る。`nix develop` の Node.js 24 に付いてくる npm はこれを満たす。
npm は、登録した内容をその場では検査しない。間違いは、公開したときの `ENEEDAUTH` で分かる。ワークフローのファイル名（大文字と小文字を区別する）と、environment 名、`id-token: write` の権限を見直す。

### 4. 最初の版について

手で出す最初の版は、0.1.0 そのものでよい。この版には provenance が付かない。
登録を済ませたあとの版（0.1.1 など）から、タグの push だけで公開される。
0.1.0 を自動で出したい場合は、名前を確保するためだけに小さい版（0.0.1 など）を手で出し、登録してから 0.1.0 のタグを push する。

## 補足

- 登録の画面や条件は、レジストリ側で変わることがある。[crates.io の Trusted Publishing](https://crates.io/docs/trusted-publishing) と [npm の Trusted publishing](https://docs.npmjs.com/trusted-publishers/) の最新の説明を見て確かめる。
- `src/wasm/` は git に入れない生成物で、npm パッケージの `.wasm` は `nix build .#npm` が作る。install 時のスクリプトには頼らない（shijimi-editor が `npm --ignore-scripts` で入れるため）。
