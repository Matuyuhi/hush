//! `docker build` 出力の圧縮。
//!
//! docker build の出力は大半がベースイメージのレイヤー取得進捗
//! （`Pulling fs layer` / `Downloading` / `Extracting` / `Pull complete`）と
//! apt 等のダウンロード行で、トークン的にはノイズ。これらを落とし、ビルドの構造
//! （`Step N/M : ...`・BuildKit の `#N [stage] ...`）・エラー・最終イメージ
//! （`Successfully built/tagged`・`writing image sha256:`・`naming to`）を残す。
//!
//! 進捗は stdout（レガシー）にも stderr（BuildKit `--progress=plain`）にも出るため
//! 両方を処理する。docker build に見えない出力は passthrough に委ねる。

use super::common::{collapse_blank_runs, combine_raw, dedup_all, strip_ansi, truncate_head_tail};
use super::{FilterInput, FilterOutput, passthrough};
use crate::error::Result;

const MAX_LINES: usize = 60;
const HEAD: usize = 48;
const TAIL: usize = 12;

/// レイヤー取得・ダウンロード進捗など、落としてよい純粋ノイズの行頭。
const NOISE_PREFIXES: &[&str] = &[
    "Downloading",
    "Download complete",
    "Extracting",
    "Pull complete",
    "Pulling fs layer",
    "Waiting",
    "Verifying Checksum",
    "Already exists",
    "Get:",
    "Fetched ",
    "Reading package lists",
    "Sending build context",
    "Removing intermediate container",
    "---> Running in",
    // BuildKit の内部ステップ（コンテキスト/定義の転送・メタ解決）。
    "transferring context",
    "transferring dockerfile",
    "load build definition",
    "load .dockerignore",
    "load metadata",
    "resolve docker.io",
];

/// BuildKit 行の `#<N>` マーカーと、続く小数タイムスタンプを取り除いた本体を返す。
/// 例: `#8 0.521 Get:1 ...` -> `Get:1 ...`、`#5 DONE 5.3s` -> `DONE 5.3s`、
/// `#5 [2/5] RUN ...` -> `[2/5] RUN ...`。BuildKit でない行はそのまま返す。
fn buildkit_body(t: &str) -> &str {
    let Some(rest) = t.strip_prefix('#') else {
        return t;
    };
    // `#` 直後のステップ番号を飛ばす。
    let rest = rest
        .trim_start_matches(|c: char| c.is_ascii_digit())
        .trim_start();
    // 続くトークンが小数タイムスタンプ（`0.521` 等）ならそれも飛ばす。
    let mut it = rest.splitn(2, ' ');
    let first = it.next().unwrap_or("");
    if !first.is_empty()
        && first.contains('.')
        && first.chars().all(|c| c.is_ascii_digit() || c == '.')
    {
        it.next().unwrap_or("").trim_start()
    } else {
        rest
    }
}

/// docker の進捗行 `<layer-id>: <status>` から layer-id を外した status 部を返す。
/// layer-id は短い hex トークン。該当しなければそのまま返す。
/// 例: `a803e7c4b030: Downloading [..]` -> `Downloading [..]`。
fn strip_layer_id(t: &str) -> &str {
    if let Some((head, rest)) = t.split_once(": ")
        && !head.is_empty()
        && head.len() <= 16
        && head.chars().all(|c| c.is_ascii_hexdigit())
    {
        return rest;
    }
    t
}

/// 落としてよい純粋なノイズ行か。エラー・失敗・ビルド構造・最終イメージは決して落とさない。
fn is_noise(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false; // 空行は collapse_blank_runs に任せる。
    }
    let body = buildkit_body(t);
    let lower = body.to_ascii_lowercase();
    // 失敗・エラーは絶対に残す。
    if lower.contains("error") || lower.contains("failed") || lower.contains("cannot ") {
        return false;
    }
    // 中間レイヤー（`---> <hash>` / `---> Using cache`）。最終イメージは Successfully... で残る。
    if let Some(after) = body.strip_prefix("---> ") {
        let r = after.trim();
        if r == "Using cache" || (!r.is_empty() && r.chars().all(|c| c.is_ascii_hexdigit())) {
            return true;
        }
    }
    // `<layer-id>: <status>` の layer-id を外してから進捗ノイズを判定する。
    let core = strip_layer_id(body);
    NOISE_PREFIXES.iter().any(|p| core.starts_with(p))
}

pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    let stdout = strip_ansi(&String::from_utf8_lossy(&input.stdout));
    let stderr = strip_ansi(&String::from_utf8_lossy(&input.stderr));
    // 進捗は両ストリームに出るので連結して 1 つの流れとして扱う
    // （原文は combine_raw でバイト厳密に保存される）。
    let combined = match (stdout.trim().is_empty(), stderr.trim().is_empty()) {
        (false, false) => format!("{}\n{}", stdout.trim_end_matches('\n'), stderr),
        (true, false) => stderr,
        (_, true) => stdout,
    };

    let orig_lines = combined.lines().count();

    let kept: Vec<&str> = combined.lines().filter(|l| !is_noise(l)).collect();
    let collapsed = collapse_blank_runs(&kept.join("\n"));
    let lines: Vec<&str> = collapsed.lines().collect();
    let deduped = dedup_all(&lines);

    // docker build に見えない（何も落とせず行数も変わらない）なら passthrough に委ねる。
    if deduped.len() == orig_lines && orig_lines <= MAX_LINES {
        return passthrough::run(input);
    }

    let (shown, truncated) = truncate_head_tail(deduped, MAX_LINES, HEAD, TAIL);
    let shown_lines = shown.len();
    let compact = if shown.is_empty() {
        "(no output)".to_string()
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
        filter_name: "docker-build",
        compact,
        original,
        orig_lines,
        shown_lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_stdout(s: &str) -> FilterOutput {
        let input = FilterInput {
            argv: vec!["docker".into(), "build".into(), ".".into()],
            stdout: s.as_bytes().to_vec(),
            stderr: Vec::new(),
        };
        run(&input).unwrap()
    }

    #[test]
    fn drops_layer_pull_and_apt_noise_keeps_steps_and_result() {
        let out = run_stdout(
            "\
Sending build context to Docker daemon  2.5MB
Step 1/3 : FROM python:3.11-slim
a803e7c4b030: Pulling fs layer
a803e7c4b030: Downloading [====>   ]  3.4MB/30MB
a803e7c4b030: Download complete
a803e7c4b030: Extracting [=====>  ]  10MB/30MB
a803e7c4b030: Pull complete
 ---> 1a2b3c4d5e6f
Step 2/3 : RUN apt-get update
 ---> Running in aabbccddeeff
Get:1 http://deb.debian.org/debian bookworm InRelease [151 kB]
Fetched 8986 kB in 2s (4493 kB/s)
Removing intermediate container aabbccddeeff
 ---> 1234abcd5678
Step 3/3 : CMD [\"python\", \"app.py\"]
Successfully built d4d4d4d4e5e5
Successfully tagged myapp:latest
",
        );
        assert_eq!(out.filter_name, "docker-build");
        // 進捗・DL・中間コンテナ・中間レイヤー ID は消える。
        assert!(!out.compact.contains("Pulling fs layer"));
        assert!(!out.compact.contains("Downloading"));
        assert!(!out.compact.contains("Pull complete"));
        assert!(!out.compact.contains("Get:1"));
        assert!(!out.compact.contains("Removing intermediate container"));
        assert!(!out.compact.contains("1a2b3c4d5e6f"));
        // 構造・結果は残る。
        assert!(out.compact.contains("Step 1/3 : FROM python:3.11-slim"));
        assert!(out.compact.contains("Step 2/3 : RUN apt-get update"));
        assert!(out.compact.contains("Successfully built d4d4d4d4e5e5"));
        assert!(out.compact.contains("Successfully tagged myapp:latest"));
    }

    #[test]
    fn keeps_error_lines() {
        let out = run_stdout(
            "\
Step 1/2 : FROM alpine
 ---> abc123
Step 2/2 : RUN exit 1
 ---> Running in deadbeef
The command '/bin/sh -c exit 1' returned a non-zero code: 1
ERROR: failed to solve: process did not complete successfully
",
        );
        assert!(out.compact.contains("returned a non-zero code: 1"));
        assert!(out.compact.contains("ERROR: failed to solve"));
        // 中間レイヤー ID と Running in は消える。
        assert!(!out.compact.contains("---> abc123"));
        assert!(!out.compact.contains("Running in deadbeef"));
    }

    #[test]
    fn buildkit_prefix_stripping() {
        // BuildKit の `#N <ts>` プレフィックス付きでもノイズ判定が効く。
        assert!(is_noise(
            "#8 0.521 Get:1 http://deb.debian.org/debian bookworm InRelease [151 kB]"
        ));
        // ステップ見出し・DONE・エラーは残す。
        assert!(!is_noise("#5 [2/5] RUN apt-get update"));
        assert!(!is_noise("#5 DONE 5.3s"));
        assert!(!is_noise("#8 ERROR: process did not complete"));
    }
}
