//! Port of the KTX-Software-CTS runner (tests/cts/clitests/clitest.py).
//!
//! Each JSON test case under `tests/resources/cts/tests/<command>/` describes a
//! `ktx` command line, its expected exit status and golden stdout/stderr (with
//! `` `regex` `` sections). Every argument combination (subcase) is run against
//! the `ktx` binary of this crate from the `tests/resources/cts` directory.
//! Data comes from KTX-Software-CTS 59f0a653 (Apache-2.0, see LICENSE there).
#![allow(clippy::panic)]

mod common;

use regex::Regex;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn cts_dir() -> PathBuf {
    common::resources_path().join("cts")
}

/// Large sample inputs (> 128 KiB) that are not vendored, one path per line
/// in `NOT_VENDORED.txt`; subcases using them are skipped.
fn not_vendored() -> Vec<String> {
    std::fs::read_to_string(cts_dir().join("NOT_VENDORED.txt"))
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Python `str()` of a JSON value.
fn py_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => if *b { "True" } else { "False" }.to_string(),
        other => other.to_string(),
    }
}

/// Port of `SubcaseContext`.
struct Subcase<'a> {
    args: Vec<(&'a str, &'a Value)>,
}

impl Subcase<'_> {
    /// `eval`: substitutes `${name}` and `${name[key]}`.
    fn eval(&self, text: &str) -> String {
        let re = match Regex::new(r"\$\{([a-zA-Z0-9_\-]+)(?:\[([a-zA-Z0-9_\-']+)\])?\}") {
            Ok(r) => r,
            Err(e) => panic!("{e}"),
        };
        re.replace_all(text, |c: &regex::Captures| {
            let name = c.get(1).map_or("", |m| m.as_str());
            let Some((_, v)) = self.args.iter().find(|(n, _)| *n == name) else {
                panic!("unknown argument {name} in {text}")
            };
            match c.get(2) {
                Some(k) => py_str(&v[k.as_str().trim_matches('\'')]),
                None => py_str(v),
            }
        })
        .into_owned()
    }

    /// `match`: golden text with `` `regex` `` sections, matched at the start.
    fn matches(&self, expected: &str, actual: &str) -> bool {
        let expected = self.eval(expected);
        let mut pattern = String::from("^");
        let parts: Vec<&str> = expected.split('`').collect();
        for (i, part) in parts.iter().enumerate() {
            if i % 2 == 1 {
                pattern.push_str(part);
            } else if i > 0 && i + 1 == parts.len() {
                // clitest.py slices the text after the last backtick with
                // `expected[start:-1]`, dropping its final character.
                let mut chars = part.chars();
                chars.next_back();
                pattern.push_str(&regex::escape(chars.as_str()));
            } else {
                pattern.push_str(&regex::escape(part));
            }
        }
        match Regex::new(&pattern) {
            Ok(re) => re.is_match(actual),
            Err(e) => panic!("bad golden regex {pattern}: {e}"),
        }
    }

    fn describe(&self) -> String {
        self.args
            .iter()
            .map(|(n, v)| format!("--{n} \"{}\"", py_str(v)))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn read_text(path: &Path) -> Option<String> {
    std::fs::read(path)
        .ok()
        .map(|b| String::from_utf8_lossy(&b).replace("\r\n", "\n"))
}

/// Runs every subcase of one JSON test case; returns (passed, skipped, failures).
fn run_case(file: &Path) -> (usize, usize, Vec<String>) {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) => panic!("{}: {e}", file.display()),
    };
    let case: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => panic!("{}: {e}", file.display()),
    };
    let dir = cts_dir();
    let mut names: Vec<&str> = Vec::new();
    let mut lists: Vec<Vec<&Value>> = Vec::new();
    if let Some(Value::Object(args)) = case.get("args") {
        for (k, v) in args {
            names.push(k);
            lists.push(v.as_array().map(|a| a.iter().collect()).unwrap_or_default());
        }
    }
    // itertools.product
    let mut combos: Vec<Vec<&Value>> = vec![vec![]];
    for list in &lists {
        combos = combos
            .iter()
            .flat_map(|c| list.iter().map(move |v| [c.clone(), vec![*v]].concat()))
            .collect();
    }

    let not_vendored = not_vendored();
    let (mut passed, mut skipped) = (0, 0);
    let mut failures = Vec::new();
    for combo in combos {
        let ctx = Subcase {
            args: names.iter().copied().zip(combo).collect(),
        };
        let command = ctx.eval(case["command"].as_str().unwrap_or_default());
        let stdin_path = case
            .get("stdin")
            .and_then(Value::as_str)
            .map(|s| ctx.eval(s));
        if not_vendored
            .iter()
            .any(|f| command.contains(f.as_str()) || stdin_path.as_deref() == Some(f.as_str()))
        {
            skipped += 1;
            continue;
        }
        let cmd_args: Vec<&str> = command.split(' ').filter(|a| !a.is_empty()).collect();
        let mut proc = Command::new(env!("CARGO_BIN_EXE_ktx"));
        proc.args(cmd_args.iter().skip(1))
            .current_dir(&dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        match &stdin_path {
            Some(p) => match std::fs::File::open(dir.join(p)) {
                Ok(f) => {
                    proc.stdin(f);
                }
                Err(e) => panic!("{p}: {e}"),
            },
            None => {
                proc.stdin(Stdio::null());
            }
        }
        let out = match proc.output() {
            Ok(o) => o,
            Err(e) => panic!("{command}: {e}"),
        };
        let status = out.status.code().unwrap_or(-1);
        let mut problems = Vec::new();
        let expected_status = match &case["status"] {
            Value::String(s) => ctx.eval(s),
            v => v.to_string(),
        };
        if expected_status != "*" && expected_status != status.to_string() {
            problems.push(format!(
                "expected return code {expected_status} but got {status}"
            ));
        }
        let binary = case
            .get("stdoutBinary")
            .is_some_and(|v| v.as_bool() == Some(true) || ctx.eval(&py_str(v)) == "True");
        for (key, actual) in [("stdout", &out.stdout), ("stderr", &out.stderr)] {
            let Some(golden) = case.get(key).and_then(Value::as_str) else {
                continue; // re.match('', ...) always matches
            };
            let golden_path = dir.join(ctx.eval(golden));
            if key == "stdout" && binary {
                if std::fs::read(&golden_path).ok().as_ref() != Some(actual) {
                    problems.push(format!(
                        "binary {key} differs from {}",
                        golden_path.display()
                    ));
                }
                continue;
            }
            let Some(expected) = read_text(&golden_path) else {
                problems.push(format!(
                    "cannot find reference {key} file {}",
                    golden_path.display()
                ));
                continue;
            };
            let actual = String::from_utf8_lossy(actual).replace("\r\n", "\n");
            if !ctx.matches(&expected, &actual) {
                problems.push(format!(
                    "{key} mismatch vs {}:\n--- got ---\n{actual}--- want ---\n{expected}",
                    golden_path.display()
                ));
            }
        }
        // Output files must match their references exactly (primary platform).
        if let Some(outputs) = case.get("outputs").and_then(Value::as_object) {
            for (cur, reference) in outputs {
                let cur = dir.join(ctx.eval(cur));
                let reference = dir.join(ctx.eval(reference.as_str().unwrap_or_default()));
                match (std::fs::read(&cur), std::fs::read(&reference)) {
                    (Err(_), _) => {
                        problems.push(format!("cannot find output file {}", cur.display()))
                    }
                    (_, Err(_)) => problems.push(format!(
                        "cannot find reference file {}",
                        reference.display()
                    )),
                    (Ok(a), Ok(b)) if a == b => {
                        let _ = std::fs::remove_file(&cur);
                    }
                    (Ok(_), Ok(_)) => problems.push(format!(
                        "mismatch between output file {} and reference file {}",
                        cur.display(),
                        reference.display()
                    )),
                }
            }
        }
        if problems.is_empty() {
            passed += 1;
        } else {
            failures.push(format!(
                "{} [{}]\n  {command}\n  {}",
                file.display(),
                ctx.describe(),
                problems.join("\n  ")
            ));
        }
    }
    (passed, skipped, failures)
}

fn run_command_dir(command: &str) {
    let dir = cts_dir().join("tests").join(command);
    let mut files: Vec<PathBuf> = match std::fs::read_dir(&dir) {
        Ok(d) => d
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect(),
        Err(e) => panic!("{}: {e}", dir.display()),
    };
    files.sort();
    let (mut passed, mut skipped) = (0, 0);
    let mut failures = Vec::new();
    for f in &files {
        let (p, s, mut fl) = run_case(f);
        passed += p;
        skipped += s;
        failures.append(&mut fl);
    }
    eprintln!(
        "cts {command}: {passed} passed, {skipped} skipped, {} failed",
        failures.len()
    );
    if !failures.is_empty() {
        let shown: Vec<&String> = failures.iter().take(10).collect();
        panic!(
            "{} of {} subcases failed:\n{}",
            failures.len(),
            failures.len() + passed,
            shown
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    assert!(passed > 0);
}

#[test]
fn cts_validate() {
    run_command_dir("validate");
}

#[test]
fn cts_info() {
    run_command_dir("info");
}

#[test]
fn cts_help() {
    run_command_dir("help");
}

#[test]
fn cts_transcode() {
    run_command_dir("transcode");
}

#[test]
fn cts_deflate() {
    run_command_dir("deflate");
}

#[test]
fn cts_extract() {
    run_command_dir("extract");
}

#[test]
fn cts_compare() {
    run_command_dir("compare");
}

#[test]
fn cts_tools() {
    run_command_dir("tools");
}
