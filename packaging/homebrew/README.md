# Homebrew 配布

`hush` は **prebuilt binary 方式**で配布する（tap の既存 formula と統一）。
利用者はコンパイル不要で即インストールできる。

tap: <https://github.com/Matuyuhi/homebrew-tools>

## 仕組み（タグ手打ち不要）

`Cargo.toml` の `version` が「リリース信号」。`.github/workflows/release.yml` が
**main への push** で起動し:

0. `Cargo.toml` の version を読む。その `v<version>` の Release が既にあれば何もしない
1. 無ければ `v<version>` タグ + GitHub Release をその commit に作成
2. 4 ターゲットをネイティブビルド
   - `macos-26`（arm）→ `aarch64-apple-darwin` と `x86_64-apple-darwin`（クロス）
   - `ubuntu-24.04` → `x86_64-linux`
   - `ubuntu-24.04-arm` → `aarch64-linux`
3. 各バイナリを `hush-<target>.tar.gz` にして Release に添付
4. `packaging/homebrew/hush.rb`（テンプレート）の `__VERSION__` と各 `__SHA_*__` を実値に差し替え、
   tap の `Formula/hush.rb` を更新してコミット/プッシュ

→ **version を上げて PR をマージするだけ**でリリースされる（手動 `git tag` 不要）。
手動で回したいときは Actions → Release → "Run workflow"。

## 一度だけの準備: シークレット

tap への自動コミットに PAT が必要。

1. GitHub の Fine-grained PAT を発行（リポジトリ `Matuyuhi/homebrew-tools` のみ、`Contents: Read and write`）
2. hush リポジトリの Settings → Secrets and variables → Actions に
   `HOMEBREW_TAP_TOKEN` として登録

未設定でもビルド & Release までは走る（tap 更新だけスキップ）。

## リリース手順

`Cargo.toml` の `version` を上げて（`cargo build` で `Cargo.lock` も更新）、PR をマージするだけ。
main への push を CI が検知し、その version の Release が無ければ自動でタグ作成 → ビルド →
Release 添付 → tap 更新まで行う。

初回 `v0.1.0` は、この仕組みがある状態で main が更新された時点（例: 本 PR のマージ）で走る。
tap 更新まで効かせたい場合は事前に `HOMEBREW_TAP_TOKEN` を登録しておくこと。

## 動作確認

```sh
brew tap Matuyuhi/tools
brew install hush            # = Matuyuhi/tools/hush
hush doctor                  # 非送信サンドボックスの実測
```

## メモ

- macOS は arm ランナー1種で両 arch をビルドする（`x86_64-apple-darwin` はクロス。
  tree-sitter の C も含めてビルド可能なことを確認済み）。
- Linux arm は `ubuntu-24.04-arm` ネイティブランナーを使う（クロス不要）。
