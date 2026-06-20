# Homebrew 配布

`hush` は **prebuilt binary 方式**で配布する（tap の既存 formula と統一）。
利用者はコンパイル不要で即インストールできる。

tap: <https://github.com/Matuyuhi/homebrew-tools>

## 仕組み

`.github/workflows/release.yml` が `v*` タグ push をトリガに自動で:

1. 4 ターゲットをネイティブビルド
   - `macos-26`（arm）→ `aarch64-apple-darwin` と `x86_64-apple-darwin`（クロス）
   - `ubuntu-24.04` → `x86_64-linux`
   - `ubuntu-24.04-arm` → `aarch64-linux`
2. 各バイナリを `hush-<target>.tar.gz` にして GitHub Release に添付
3. `packaging/homebrew/hush.rb`（テンプレート）の `__VERSION__` と各 `__SHA_*__` を実値に差し替え、
   tap の `Formula/hush.rb` を更新してコミット/プッシュ

## 一度だけの準備: シークレット

tap への自動コミットに PAT が必要。

1. GitHub の Fine-grained PAT を発行（リポジトリ `Matuyuhi/homebrew-tools` のみ、`Contents: Read and write`）
2. hush リポジトリの Settings → Secrets and variables → Actions に
   `HOMEBREW_TAP_TOKEN` として登録

未設定でもビルド & Release までは走る（tap 更新だけスキップ）。

## リリース手順

```sh
# バージョンを上げて Cargo.lock も更新・コミット（main 上で）
git tag v0.1.0
git push origin v0.1.0
```

あとは release.yml が Release 添付と tap 更新まで行う。

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
