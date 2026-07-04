//! パッケージマネージャの install 出力の圧縮（npm / pnpm / yarn / bun / pip）。
//!
//! install ログは進捗ノイズが大半を占める: registry への http 行、
//! `npm timing ... reify/idealTree`、ダウンロード/リンク進捗、`added pkg@x` の
//! パッケージ羅列、funding/notice バナー、プログレスバー。これらは捨てる。
//!
//! 残す「シグナル」: deprecation 警告、peerDependency 警告、エラー（`npm ERR!` /
//! ERESOLVE / `pip ERROR`）、サマリ行（`added N packages` / `removed N` /
//! `changed N` / `N packages are looking for funding` / `found N vulnerabilities` /
//! `Successfully installed ...`）。サマリは末尾に出るので、切り詰めは末尾を残す。
//! install ログと認識できる手掛かりが無ければ passthrough にフォールバック。

use super::common::{collapse_blank_runs, combine_raw, dedup_all, strip_ansi, truncate_head_tail};
use super::{FilterInput, FilterOutput, passthrough};
use crate::error::Result;

const MAX_LINES: usize = 50;
const HEAD: usize = 10;
const TAIL: usize = 38;

/// 純粋な進捗ノイズ行か（落としてよい）。シグナル（警告/エラー/サマリ）は決して落とさない。
fn is_noise(t: &str) -> bool {
    // npm: 1 パッケージごとの追加/取得進捗
    if let Some(rest) = t.strip_prefix("npm ") {
        let r = rest.trim_start();
        // http fetch / timing / sill / verbose / info(http) はノイズ。
        // ただし npm warn / npm WARN / npm ERR! はシグナルなので除外。
        if r.starts_with("http ")
            || r.starts_with("timing ")
            || r.starts_with("sill ")
            || r.starts_with("verbose ")
            || r.starts_with("notice ")
        {
            return true;
        }
    }
    // 個別パッケージの追加/取得/リンク行（"added pkg@x" 等。サマリ "added N packages" は別扱い）
    if is_per_package(t) {
        return true;
    }
    // pnpm の進捗: "Progress: resolved N, reused N, downloaded N, added N"
    // の途中経過（最終行はサマリで残したいが、ここでは保守的に進捗系を落とす）。
    if t.starts_with("Progress: resolved")
        || t.starts_with("Packages: +")
        || t.starts_with("Resolving:")
        || t.starts_with("Downloading ")
        || t.starts_with("Fetching ")
        || t.starts_with("Linking ")
    {
        return true;
    }
    // yarn: 進捗ステップ "[1/4] Resolving packages..." やスピナ行。
    if t.starts_with("[1/") || t.starts_with("[2/") || t.starts_with("[3/") || t.starts_with("[4/")
    {
        return true;
    }
    // pip: ダウンロード/収集の進捗
    if t.starts_with("Collecting ")
        || t.starts_with("Downloading ")
        || t.starts_with("Using cached ")
        || t.starts_with("Requirement already satisfied:")
        || t.starts_with("Installing collected packages:")
        || t.starts_with("Preparing metadata")
        || t.starts_with("Building wheel")
        || t.starts_with("Created wheel")
        || t.starts_with("Stored in directory:")
        || t.starts_with("Getting requirements")
    {
        return true;
    }
    false
}

/// "added react@18.2.0" のような 1 パッケージ単位の行か（"added 30 packages" は除外）。
fn is_per_package(t: &str) -> bool {
    for verb in ["added ", "removed ", "changed ", "reused ", "downloaded "] {
        if let Some(rest) = t.strip_prefix(verb) {
            let first = rest.split_whitespace().next().unwrap_or("");
            // 先頭トークンが数字ならサマリ（"added 30 packages"）→ ノイズではない。
            // それ以外（"react@18.2.0"）は個別パッケージ行 → ノイズ。
            if !first.is_empty() && !first.chars().next().unwrap_or(' ').is_ascii_digit() {
                return true;
            }
        }
    }
    false
}

/// install ログと認識できる手掛かりがあるか（最低1つあればフィルタを適用）。
fn looks_like_install(text: &str) -> bool {
    text.lines().any(|l| {
        let t = l.trim_start();
        t.starts_with("npm ")
            || t.starts_with("added ")
            || t.starts_with("removed ")
            || t.starts_with("changed ")
            || t.contains("looking for funding")
            || t.contains("found 0 vulnerabilities")
            || t.contains("vulnerabilit")
            || t.starts_with("Collecting ")
            || t.starts_with("Successfully installed")
            || t.starts_with("Requirement already satisfied:")
            || t.starts_with("Progress: resolved")
            || t.starts_with("Packages: +")
            || t.starts_with("yarn install")
            || t.starts_with("bun install")
            || t.starts_with("ERESOLVE")
            || t.starts_with("ERROR:")
    })
}

pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    let stdout = strip_ansi(&String::from_utf8_lossy(&input.stdout));
    let stderr = strip_ansi(&String::from_utf8_lossy(&input.stderr));
    // npm/pnpm/yarn は進捗を stderr に流すことが多い。両方を 1 本のテキストに結合して扱う。
    let text = crate::filters::common::combine_outputs(stdout, stderr);

    // install ログと認識できなければ汎用圧縮へ。
    if !looks_like_install(&text) {
        return passthrough::run(input);
    }

    let orig_lines = text.lines().count();

    // 進捗ノイズを落とす（シグナルは残す）。
    let kept: Vec<&str> = text.lines().filter(|l| !is_noise(l.trim_start())).collect();

    // 散発的に繰り返す警告（同一 deprecation 等）を回数付きで集約。
    let deduped = dedup_all(&kept);
    let collapsed = collapse_blank_runs(&deduped.join("\n"));
    let lines: Vec<String> = collapsed.lines().map(str::to_string).collect();

    // サマリは末尾に出るので末尾を厚めに残す。
    let (shown, truncated) = truncate_head_tail(lines, MAX_LINES, HEAD, TAIL);

    let shown_lines = shown.len();
    let compact = if shown.is_empty() {
        "(no install output)".to_string()
    } else {
        shown.join("\n")
    };

    let elided = truncated || shown_lines < orig_lines;
    let original = if elided {
        Some(combine_raw(&input.stdout, &input.stderr))
    } else {
        None
    };

    Ok(FilterOutput {
        filter_name: "pkg-install",
        compact,
        original,
        orig_lines,
        shown_lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npm_drops_noise_keeps_warnings_and_summary() {
        let stdout = include_str!("../../tests/fixtures/pkg-install/npm-install.stdout");
        let input = FilterInput {
            argv: vec!["npm".into(), "install".into()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();
        assert_eq!(out.filter_name, "pkg-install");

        // 進捗ノイズは消える。
        assert!(!out.compact.contains("npm http fetch"));
        assert!(!out.compact.contains("npm timing"));
        assert!(!out.compact.contains("idealTree"));
        assert!(!out.compact.contains("reifyNode"));
        // 個別パッケージ羅列は消える。
        assert!(!out.compact.contains("added react@18.2.0"));
        assert!(!out.compact.contains("added express@4.18.2"));

        // deprecation / peer / engine 警告は残る。
        assert!(out.compact.contains("deprecated"));
        assert!(out.compact.contains("peer dependency"));
        assert!(out.compact.contains("EBADENGINE"));
        // サマリは残る。
        assert!(out.compact.contains("added 30 packages"));
        assert!(out.compact.contains("looking for funding"));
        assert!(out.compact.contains("found 3 vulnerabilities"));

        // 圧縮されているはず。
        assert!(out.shown_lines < out.orig_lines);
        assert!(out.original.is_some());
    }

    #[test]
    fn repeated_deprecation_collapses_with_count() {
        // 同一 deprecation 行が複数回出ると (xN) に集約される。
        let stdout = "\
npm http fetch GET 200 https://registry.example.com/a 10ms
npm warn deprecated foo@1.0.0: do not use
added foo@1.0.0
npm warn deprecated foo@1.0.0: do not use

added 1 package, and audited 2 packages in 1s

found 0 vulnerabilities
";
        let input = FilterInput {
            argv: vec!["npm".into(), "i".into()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();
        assert!(
            out.compact
                .contains("npm warn deprecated foo@1.0.0: do not use  (x2)")
        );
        assert!(out.compact.contains("added 1 package"));
        assert!(out.compact.contains("found 0 vulnerabilities"));
        assert!(!out.compact.contains("npm http fetch"));
        assert!(!out.compact.contains("added foo@1.0.0"));
    }

    #[test]
    fn pip_keeps_errors_and_success() {
        let stdout = "\
Collecting requests
  Downloading requests-2.31.0-py3-none-any.whl (62 kB)
Collecting urllib3<3
  Using cached urllib3-2.0.7-py3-none-any.whl (124 kB)
Installing collected packages: urllib3, requests
Successfully installed requests-2.31.0 urllib3-2.0.7
";
        let input = FilterInput {
            argv: vec!["pip".into(), "install".into(), "requests".into()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();
        assert_eq!(out.filter_name, "pkg-install");
        assert!(!out.compact.contains("Collecting requests"));
        assert!(!out.compact.contains("Downloading"));
        assert!(!out.compact.contains("Using cached"));
        assert!(
            out.compact
                .contains("Successfully installed requests-2.31.0 urllib3-2.0.7")
        );
    }

    #[test]
    fn npm_eresolve_error_is_kept() {
        let stderr = "\
npm http fetch GET 200 https://registry.example.com/a 10ms
npm error code ERESOLVE
npm error ERESOLVE unable to resolve dependency tree
npm error
npm error While resolving: my-app@1.0.0
npm error Found: react@17.0.2
";
        let input = FilterInput {
            argv: vec!["npm".into(), "install".into()],
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        };
        let out = run(&input).unwrap();
        assert!(
            out.compact
                .contains("ERESOLVE unable to resolve dependency tree")
        );
        assert!(out.compact.contains("Found: react@17.0.2"));
        assert!(!out.compact.contains("npm http fetch"));
    }

    #[test]
    fn falls_back_when_not_install_output() {
        let input = FilterInput {
            argv: vec!["npm".into(), "install".into()],
            stdout: b"just some unrelated text\nmore unrelated lines\n".to_vec(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();
        assert_eq!(out.filter_name, "passthrough");
    }
}
