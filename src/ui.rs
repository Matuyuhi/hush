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
///
/// 幅は Line/Center 行の最大幅。罫線は常にコンテンツ全体に伸び、
/// どの行も溢れない（= 数値が巨大でも名前が長くてもレイアウトが崩れない）。
pub fn render(rows: &[Row]) {
    let width = rows
        .iter()
        .filter_map(|r| match r {
            Row::Line(s) | Row::Center(s) => Some(s.chars().count()),
            Row::Rule => None,
        })
        .max()
        .unwrap_or(0);
    let bar = "-".repeat(width);
    for r in rows {
        match r {
            Row::Rule => println!("{bar}"),
            Row::Line(s) => println!("{s}"),
            Row::Center(s) => println!("{}", center(s, width)),
        }
    }
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
