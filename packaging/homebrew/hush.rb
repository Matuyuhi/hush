# Matuyuhi/homebrew-tools に置く formula。
#
# このリポジトリ内のコピーは「正」ではなく参照用。実際に配布する版は
# tap リポジトリ (https://github.com/Matuyuhi/homebrew-tools) の
# Formula/hush.rb を更新する。リリース手順は packaging/homebrew/README.md を参照。
#
# ソースビルド方式: Homebrew が rust をビルド時依存として入れ、`cargo install`
# でビルドする。バイナリのクロスコンパイル/ホスティングが不要で最も軽い。

class Hush < Formula
  desc "Compress dev-command output for LLMs; the filter physically cannot send it anywhere"
  homepage "https://github.com/Matuyuhi/hush"
  url "https://github.com/Matuyuhi/hush/archive/refs/tags/v0.1.0.tar.gz"
  # 下記はタグ付け後に計算して差し替える（README.md の手順参照）。
  sha256 "REPLACE_WITH_TARBALL_SHA256"
  license "Apache-2.0"
  head "https://github.com/Matuyuhi/hush.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    # Homebrew の test サンドボックス内では doctor の sandbox_init が
    # 入れ子になり挙動が不安定なため、ここでは起動確認のみ。
    assert_match "hush", shell_output("#{bin}/hush --help")
  end
end
