# 公開の手順

coneko-term は crate（crates.io）と npm パッケージ（npm）を、同じ版で一緒に公開する。
GitHub Actions の「リリース」（[`release.yml`](.github/workflows/release.yml)）を main で起動すると、版を上げて両方に公開する。
仕組みは shijimi-editor の `release.yml` とそろえてある。
長く使えるトークンは置かない。どちらも trusted publishing（GitHub Actions の OIDC）で認証する。

## 新しい版を出す

1. Actions の「リリース」を main で起動し、上げる桁（`patch`・`minor`・`major`）を選ぶ。

   ```sh
   gh workflow run release.yml -f bump=patch
   ```

2. Actions の「リリース」が全部通ったことを確かめる。

版を手で書き換える PR は要らない。

## release.yml がすること

1. `release`: `scripts/bump-version.sh` で版を上げる（`Cargo.toml`・`Cargo.lock`・`package.json`・`package-lock.json`）。上げた版で `nix flake check` を回す。通ったらコミットし、タグ `vX.Y.Z` を打ち、main と `release` ブランチと一緒に push する。GitHub Release を作る（リリースノートは自動生成）。
2. `crates-io`: タグから `cargo publish -p coneko-term`。
3. `npm`: タグから `nix build .#npm` で作った tarball を `npm publish`（provenance 付き）。`nix flake check` でテスト済みの tarball をそのまま配る。

`nix flake check` が落ちたら、コミットもタグも push しない。
`crates-io` と `npm` は、その版が公開済みなら飛ばす。途中で落ちたら、Actions の画面で失敗したジョブだけ再実行できる。
`release` ブランチは最新のリリースを指す。Nix の利用者は `github:mm1995tk/coneko-term/release` で、main の未リリースの変更を取らずに済む。

`release` ジョブは github-actions[bot] として main に直接 push する。そのため、main のルールには、PR と検査を求めるものを入れない（shijimi-editor と同じく、ブランチの削除の禁止と強制 push の禁止だけにする）。

CI（`check.yml`）の `publish-dry-run` ジョブは、PR ごとに `cargo publish --dry-run` と `npm publish --dry-run` を回す。
リリースを起動する前に、受け取れる形かどうかが分かる。
いまの版は公開済みなので、試す前に CI の中だけで仮の版（`0.0.0-ci`）に替える（コミットしない）。

手元で確かめるときは、次を使う。

```sh
sh scripts/check-release-version.sh            # Cargo.toml と package.json の版がそろっているか
cargo publish -p coneko-term --locked --dry-run
nix build .#npm --out-link npm-package && npm publish ./npm-package/*.tgz --access public --dry-run
```

## ユーザーがやること（最初の 1 回）

どちらのレジストリも、trusted publisher を登録するには、そのパッケージが先に存在している必要がある。
そのため、最初の版だけは手で公開する。2 回目からは「リリース」の起動だけで公開できる。

### 1. 最初の版のタグを打つ

Actions の「リリース」を `minor` で起動する（0.0.0 → 0.1.0）。
タグ `v0.1.0` と GitHub Release ができる。`crates-io` と `npm` のジョブは、trusted publisher がまだ無いので落ちる。これは想定どおり。

### 2. crates.io

1. crates.io にログインし、Account Settings → API Tokens で、`publish-new` と `publish-update` の権限を持つ短命のトークンを作る。
2. タグ `v0.1.0` を取り出して、手元で公開する。

   ```sh
   git fetch --tags && git switch --detach v0.1.0
   nix develop -c cargo publish -p coneko-term --locked --token <作ったトークン>
   ```

3. 公開できたら、そのトークンは revoke（失効）する。
4. crates.io の `coneko-term` のページ → Settings → Trusted Publishing → Add で、GitHub の設定を次のように登録する。
   - Repository owner: `mm1995tk`
   - Repository name: `coneko-term`
   - Workflow filename: `release.yml`
   - Environment: 空のまま
5. 余裕があれば「Enforce trusted publishing」を有効にして、トークンでの公開を止める。

### 3. npm

1. 同じくタグ `v0.1.0` で、手元で公開する（`npm login` 済みで、2FA を使える状態で）。

   ```sh
   nix build .#npm --out-link npm-package
   nix develop -c npm publish ./npm-package/*.tgz --access public
   ```

2. npmjs.com の `coneko-term` のページ → Settings → Trusted Publisher で GitHub Actions を選び、次を登録する。
   - Organization or user: `mm1995tk`
   - Repository: `coneko-term`
   - Workflow filename: `release.yml`
   - Environment name: 空のまま
3. 余裕があれば、同じ Settings の Publishing access で「Require two-factor authentication and disallow tokens」にする。

npm の trusted publishing は npm CLI 11.5.1 以上が要る。`nix develop` の Node.js 24 に付いてくる npm はこれを満たす。
npm は、登録した内容をその場では検査しない。間違いは、公開したときの `ENEEDAUTH` で分かる。ワークフローのファイル名（大文字と小文字を区別する）と、`id-token: write` の権限を見直す。

手で出した 0.1.0 には provenance が付かない。次の版（0.1.1 など）から、「リリース」の起動だけで provenance 付きで公開される。

## 補足

- 登録の画面や条件は、レジストリ側で変わることがある。[crates.io の Trusted Publishing](https://crates.io/docs/trusted-publishing) と [npm の Trusted publishing](https://docs.npmjs.com/trusted-publishers/) の最新の説明を見て確かめる。
- `src/wasm/` は git に入れない生成物で、npm パッケージの `.wasm` は `nix build .#npm` が作る。install 時のスクリプトには頼らない（shijimi-editor が `npm --ignore-scripts` で入れるため）。
- `nix build .#npm` は、npm の依存を `package-lock.json` の integrity から取る（`importNpmLock`）。`package-lock.json` が変わっても、Nix の側に直すハッシュは無い。
