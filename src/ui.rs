//! 端末出力の共通ヘルパ。
//!
//! **意図的に ASCII のみ**を使う: East-Asian "ambiguous" 幅の文字
//! （`─ · → ×` 等）は CJK 端末で 2 カラム幅に描画され桁ズレを起こすため、
//! 罫線・記号には使わない。幅計算は `chars().count()`（ASCII では = 表示桁数）。

/// 枠付きブロックの 1 行。
pub enum Row {
    /// ブロック幅いっぱいの水平罫線。
    Rule,
    /// そのまま出力する行（インデント済み前提）。
    Line(String),
    /// ブロック幅の中央に寄せる行。
    Center(String),
}

/// 行群を枠付きブロックとして出力する。
pub fn render(rows: &[Row]) {
    println!("{}", render_to_string(rows));
}

/// 行群を枠付きブロックの文字列にする（末尾改行なし）。
///
/// 幅は Line/Center 行の最大幅。罫線は常にコンテンツ全体に伸び、
/// どの行も溢れない（= 数値が巨大でも名前が長くてもレイアウトが崩れない）。
/// 端末出力は `render`、文字列が要る用途（README 埋め込み等）はこちらを使う。
pub fn render_to_string(rows: &[Row]) -> String {
    let width = rows
        .iter()
        .filter_map(|r| match r {
            Row::Line(s) | Row::Center(s) => Some(s.chars().count()),
            Row::Rule => None,
        })
        .max()
        .unwrap_or(0);
    let bar = "-".repeat(width);
    rows.iter()
        .map(|r| match r {
            Row::Rule => bar.clone(),
            Row::Line(s) => s.clone(),
            Row::Center(s) => center(s, width),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `s` を `width` カラムの中央に寄せる（左側を空白で詰める。ASCII 前提）。
pub fn center(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        return s.to_string();
    }
    " ".repeat((width - len) / 2) + s
}

/// "16005" -> "16,005"
pub fn commas(n: u64) -> String {
    let digits = n.to_string();
    let len = digits.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// 1000 進数で K/M/B にスケールしたカウント表記（token 数など）。
/// 例: 453 -> "453", 4001 -> "4.0K", 250000 -> "250K", 1_200_000 -> "1.2M"。
pub fn human_count(n: u64) -> String {
    let (val, suffix) = if n < 1_000 {
        return n.to_string();
    } else if n < 1_000_000 {
        (n as f64 / 1e3, "K")
    } else if n < 1_000_000_000 {
        (n as f64 / 1e6, "M")
    } else {
        (n as f64 / 1e9, "B")
    };
    // 1 桁に丸めてから「100 以上は整数」を判定（境界で小数が残らない）。
    let rounded = (val * 10.0).round() / 10.0;
    if rounded >= 100.0 {
        format!("{rounded:.0}{suffix}")
    } else {
        format!("{rounded:.1}{suffix}")
    }
}

/// 1000 進数のバイト表記。例: 999 -> "999 B", 16_005 -> "16.0 KB", 1_200_000 -> "1.2 MB"。
pub fn human_bytes(n: u64) -> String {
    let (val, suffix) = if n < 1_000 {
        return format!("{n} B");
    } else if n < 1_000_000 {
        (n as f64 / 1e3, "KB")
    } else if n < 1_000_000_000 {
        (n as f64 / 1e6, "MB")
    } else {
        (n as f64 / 1e9, "GB")
    };
    let rounded = (val * 10.0).round() / 10.0;
    if rounded >= 100.0 {
        format!("{rounded:.0} {suffix}")
    } else {
        format!("{rounded:.1} {suffix}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_count_scales() {
        assert_eq!(human_count(453), "453");
        assert_eq!(human_count(4_001), "4.0K");
        assert_eq!(human_count(250_000), "250K");
        assert_eq!(human_count(1_200_000), "1.2M");
        assert_eq!(human_count(3_400_000_000), "3.4B");
        // 丸めで 100 を跨ぐ境界は整数表記（小数を残さない）。
        assert_eq!(human_count(99_950), "100K");
    }

    #[test]
    fn human_bytes_scales() {
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(16_005), "16.0 KB");
        assert_eq!(human_bytes(1_200_000), "1.2 MB");
        assert_eq!(human_bytes(2_500_000_000), "2.5 GB");
        assert_eq!(human_bytes(99_950), "100 KB");
    }

    #[test]
    fn commas_groups_thousands() {
        assert_eq!(commas(0), "0");
        assert_eq!(commas(16_005), "16,005");
        assert_eq!(commas(1_000_000), "1,000,000");
    }
}
