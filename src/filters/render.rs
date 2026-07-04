//! compact 出力に付ける expand フッタの整形。

/// 圧縮で原文を削ったときに付けるフッタ。
///
/// 例: [hush:git-status id=ab12cd34ef56 lines=210->18 - `hush expand ab12cd34ef56` for full output]
pub fn footer(filter: &str, id: &str, orig_lines: usize, shown_lines: usize) -> String {
    format!(
        "\n[hush:{filter} id={id} lines={orig_lines}->{shown_lines} - `hush expand {id}` for full output]"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_footer() {
        let result = footer("git-status", "ab12cd34ef56", 210, 18);
        assert_eq!(
            result,
            "\n[hush:git-status id=ab12cd34ef56 lines=210->18 - `hush expand ab12cd34ef56` for full output]"
        );

        let result = footer("custom-filter", "000000000000", 0, 0);
        assert_eq!(
            result,
            "\n[hush:custom-filter id=000000000000 lines=0->0 - `hush expand 000000000000` for full output]"
        );
    }
}
