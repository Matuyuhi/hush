//! compact 出力に付ける expand フッタの整形。

/// 圧縮で原文を削ったときに付けるフッタ。
///
/// 例: `[hush:git-status id=ab12cd34ef56 lines=210→18 · `hush expand ab12cd34ef56` で全文]`
pub fn footer(filter: &str, id: &str, orig_lines: usize, shown_lines: usize) -> String {
    format!(
        "\n[hush:{filter} id={id} lines={orig_lines}→{shown_lines} · `hush expand {id}` で全文]"
    )
}
