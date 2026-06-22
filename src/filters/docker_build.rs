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

use super::common::{combine_raw, dedup_all, strip_ansi, truncate_head_tail};
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
    // BuildKit の内部ステップ・レイヤー取得・エクスポート進捗。
    "[internal] ",
    "transferring context",
    "transferring dockerfile",
    "load build definition",
    "load .dockerignore",
    "load metadata",
    "resolve docker.io",
    "sha256:", // レイヤー blob の DL 進捗（最終イメージは "writing image sha256:" で残る）。
    "extracting", // BuildKit の小文字 extracting（レガシーの "Extracting" は別途）。
    "exporting ", // exporting layers / exporting to image（結果は writing image / naming to で残る）。
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
        return false; // 空行は run() 側で別途除外する。
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
    // BuildKit のステップ完了行 `DONE 3.6s`（直前の `#N [stage]` 見出しと重複する）。
    // 期間（数字+`s`）の形のみを対象にし、RUN が出力する "DONE ..." 等は誤爆させない。
    if let Some(rest) = body.strip_prefix("DONE ") {
        let r = rest.trim();
        if r.len() >= 2
            && r.ends_with('s')
            && r[..r.len() - 1]
                .chars()
                .all(|c| c.is_ascii_digit() || c == '.')
        {
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

    // ノイズ行と空行（ステップ間の視覚的区切り）を落とす。構造は `Step`/`#N [stage]`
    // 見出しが示すので、空行が無くても読みやすさは保たれる。
    let kept: Vec<&str> = combined
        .lines()
        .filter(|l| !l.trim().is_empty() && !is_noise(l))
        .collect();
    let deduped = dedup_all(&kept);

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
        // レイヤー blob DL・extracting・内部ステップ・ステップ完了はノイズ。
        assert!(is_noise(
            "#4 sha256:aaaa1111bbbb2222 30.43MB / 30.43MB 1.2s done"
        ));
        assert!(is_noise("#4 extracting sha256:aaaa1111bbbb2222 1.1s done"));
        assert!(is_noise(
            "#1 [internal] load build definition from Dockerfile"
        ));
        assert!(is_noise("#5 DONE 5.3s"));
        // ステップ見出し・エラー・最終イメージは残す。
        assert!(!is_noise("#5 [2/5] RUN apt-get update"));
        assert!(!is_noise("#8 ERROR: process did not complete"));
        assert!(!is_noise("#11 writing image sha256:d4d4d4d4e5e5 done"));
        // RUN が出力する "DONE ..." は期間形でないので誤爆しない。
        assert!(!is_noise("#9 12.3 DONE building assets"));
    }

    #[test]
    fn summarizes_buildkit_progress_output() {
        let out = run_stdout(
            "\
#1 [internal] load build definition from Dockerfile
#1 transferring dockerfile: 412B done
#1 DONE 0.0s

#4 [1/3] FROM docker.io/library/alpine@sha256:abcd1234
#4 sha256:aaaa1111bbbb2222 3.40MB / 3.40MB 0.2s done
#4 extracting sha256:aaaa1111bbbb2222 0.1s done
#4 DONE 0.4s

#5 [2/3] RUN apk add --no-cache curl
#5 1.234 fetch https://dl-cdn.alpinelinux.org/alpine/v3.19/main
#5 DONE 1.5s

#6 [3/3] COPY . .
#6 DONE 0.0s

#7 exporting to image
#7 exporting layers 0.3s done
#7 writing image sha256:d4d4d4d4e5e5f6f6 done
#7 naming to docker.io/library/myapp:latest done
#7 DONE 0.5s
",
        );
        assert_eq!(out.filter_name, "docker-build");
        // レイヤー blob・extracting・内部ステップ・DONE・空行は消える。
        assert!(!out.compact.contains("sha256:aaaa1111bbbb2222"));
        assert!(!out.compact.contains("extracting"));
        assert!(!out.compact.contains("[internal]"));
        assert!(!out.compact.contains("DONE"));
        assert!(!out.compact.contains("exporting"));
        assert!(!out.compact.contains("transferring"));
        // ステップ見出し・最終イメージは残る。
        assert!(out.compact.contains("[1/3] FROM"));
        assert!(out.compact.contains("[2/3] RUN apk add --no-cache curl"));
        assert!(out.compact.contains("[3/3] COPY . ."));
        assert!(
            out.compact
                .contains("writing image sha256:d4d4d4d4e5e5f6f6 done")
        );
        assert!(
            out.compact
                .contains("naming to docker.io/library/myapp:latest done")
        );
        assert!(out.shown_lines < out.orig_lines);
    }
}
