# Homebrew 配布手順

`hush` は **ソースビルド方式**で配布する。バイナリのクロスコンパイル・ホスティング・
署名が不要で最も軽い。利用者の環境で `cargo install` が走る（Homebrew が rust を
ビルド時依存として導入する）。

tap: <https://github.com/Matuyuhi/homebrew-tools>

## リリース手順

1. バージョンを上げる
   - `Cargo.toml` の `version` を更新し、`cargo build`（`Cargo.lock` を更新・コミット）。
   - **`Cargo.lock` は必ずコミットする**（formula が `--locked` でビルドするため）。

2. タグを打って push
   ```sh
   git tag v0.1.0
   git push origin v0.1.0
   ```
   GitHub がソース tarball を自動生成する:
   `https://github.com/Matuyuhi/hush/archive/refs/tags/v0.1.0.tar.gz`

3. tarball の sha256 を計算
   ```sh
   curl -fsSL https://github.com/Matuyuhi/hush/archive/refs/tags/v0.1.0.tar.gz \
     | shasum -a 256
   ```

4. tap の formula を更新
   - `homebrew-tools` の `Formula/hush.rb` に本ディレクトリの `hush.rb` を反映。
   - `url` のバージョンと `sha256` を 2〜3 の値に差し替えてコミット・push。

5. 動作確認
   ```sh
   brew tap Matuyuhi/tools
   brew install hush          # = Matuyuhi/tools/hush
   hush doctor                # 非送信サンドボックスの実測（手元で確認）
   ```

## 将来の自動化（任意・後回しで良い）

タグ push をトリガに sha256 を計算して tap の formula を自動更新する GitHub Actions も
組める。tap リポジトリへの書き込み権を持つ PAT（`HOMEBREW_TAP_TOKEN` 等のシークレット）が
必要になるため、必要になった段階で追加する。
