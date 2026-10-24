//! Port of the parts of cxxopts 3.1.1 (external/cxxopts) the `ktx` tool uses:
//! option declaration, command line parsing with its error messages, and the
//! `--help` text layout.

use std::collections::HashMap;

/// Port of `cxxopts::exceptions::parsing` (its `what()` text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

/// One declared option (`OptionDetails` + its `HelpOptionDetails`).
#[derive(Debug, Clone)]
pub struct OptionSpec {
    pub short: Option<String>,
    pub long: String,
    pub desc: String,
    /// `cxxopts::value<bool>()`: implicit "true", default "false".
    pub is_boolean: bool,
    pub default_value: Option<String>,
    pub implicit_value: Option<String>,
    pub arg_help: String,
    pub is_container: bool,
    pub group: String,
    /// `cxxopts::value<uint32_t>()`: values must parse as `u32`.
    pub is_u32: bool,
    /// `cxxopts::value<float>()`: values must parse as `float` (`istream >> float`).
    pub is_f32: bool,
}

impl OptionSpec {
    /// A flag (`("h,help", "desc")`).
    pub fn flag(names: &str, desc: &str) -> OptionSpec {
        let (short, long) = split_names(names);
        OptionSpec {
            short,
            long,
            desc: desc.to_string(),
            is_boolean: true,
            default_value: Some("false".into()),
            implicit_value: Some("true".into()),
            arg_help: String::new(),
            is_container: false,
            group: String::new(),
            is_u32: false,
            is_f32: false,
        }
    }

    /// A valued option (`cxxopts::value<T>()`).
    pub fn value(names: &str, desc: &str, arg_help: &str) -> OptionSpec {
        let (short, long) = split_names(names);
        OptionSpec {
            short,
            long,
            desc: desc.to_string(),
            is_boolean: false,
            default_value: None,
            implicit_value: None,
            arg_help: arg_help.to_string(),
            is_container: false,
            group: String::new(),
            is_u32: false,
            is_f32: false,
        }
    }

    /// `->default_value(v)`.
    pub fn default_value(mut self, v: &str) -> OptionSpec {
        self.default_value = Some(v.to_string());
        self
    }

    /// `->implicit_value(v)`.
    pub fn implicit_value(mut self, v: &str) -> OptionSpec {
        self.implicit_value = Some(v.to_string());
        self
    }

    /// `cxxopts::value<std::vector<T>>()`.
    pub fn container(mut self) -> OptionSpec {
        self.is_container = true;
        self
    }

    /// `cxxopts::value<uint32_t>()`.
    pub fn u32(mut self) -> OptionSpec {
        self.is_u32 = true;
        self
    }

    /// `cxxopts::value<float>()`.
    pub fn f32(mut self) -> OptionSpec {
        self.is_f32 = true;
        self
    }

    /// Option group (`add_options("Group")`).
    pub fn group(mut self, g: &str) -> OptionSpec {
        self.group = g.to_string();
        self
    }
}

fn split_names(names: &str) -> (Option<String>, String) {
    let parts: Vec<&str> = names
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    match parts.as_slice() {
        [s, l] if s.len() == 1 => (Some((*s).to_string()), (*l).to_string()),
        [one] if one.len() == 1 => (Some((*one).to_string()), String::new()),
        [l] => (None, (*l).to_string()),
        _ => (None, names.to_string()),
    }
}

/// Port of `cxxopts::Options`.
#[derive(Debug, Clone)]
pub struct Options {
    program: String,
    custom_help: String,
    positional_help: String,
    width: usize,
    allow_unrecognised: bool,
    specs: Vec<OptionSpec>,
    positional: Vec<String>,
}

/// Port of `cxxopts::ParseResult`.
#[derive(Debug, Clone, Default)]
pub struct ParseResult {
    values: HashMap<String, Vec<String>>,
    defaults: HashMap<String, String>,
    unmatched: Vec<String>,
}

impl ParseResult {
    /// `count(name)`: times the option was given (positionals included).
    pub fn count(&self, name: &str) -> usize {
        self.values.get(name).map_or(0, Vec::len)
    }

    /// Last given value, else the default.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values
            .get(name)
            .and_then(|v| v.last())
            .map(String::as_str)
            .or_else(|| self.defaults.get(name).map(String::as_str))
    }

    /// All given values (container options).
    pub fn get_all(&self, name: &str) -> &[String] {
        self.values.get(name).map_or(&[], Vec::as_slice)
    }

    /// `as<bool>()`.
    pub fn as_bool(&self, name: &str) -> bool {
        self.get(name).is_some_and(is_true_text)
    }

    /// `as<uint32_t>()` (values were checked while parsing).
    pub fn as_u32(&self, name: &str) -> u32 {
        self.get(name).and_then(parse_u32).unwrap_or(0)
    }

    /// `as<float>()` (values were checked while parsing).
    pub fn as_f32(&self, name: &str) -> f32 {
        self.get(name).and_then(parse_f32).unwrap_or(0.0)
    }

    /// `unmatched()`.
    pub fn unmatched(&self) -> &[String] {
        &self.unmatched
    }
}

/// Port of `integer_parser<uint32_t>`: `(-)?(0x)?([0-9a-zA-Z]+)|((0x)?0)`,
/// decimal or hex digits, overflow detected like C (`result > next` after a
/// wrapping multiply-add), negative values rejected.
pub fn parse_u32(text: &str) -> Option<u32> {
    let all_alnum = |s: &str| !s.is_empty() && s.bytes().all(is_alnum);
    let (negative, rest) = match text.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, text),
    };
    let (base, digits) = match rest.strip_prefix("0x") {
        Some(d) if all_alnum(d) => (16u32, d),
        _ => (10u32, rest),
    };
    if !all_alnum(digits) {
        return None;
    }
    let mut result = 0u32;
    for ch in digits.bytes() {
        let digit = match ch {
            b'0'..=b'9' => u32::from(ch - b'0'),
            b'a'..=b'f' if base == 16 => u32::from(ch - b'a' + 10),
            b'A'..=b'F' if base == 16 => u32::from(ch - b'A' + 10),
            _ => return None,
        };
        let next = result.wrapping_mul(base).wrapping_add(digit);
        if result > next {
            return None;
        }
        result = next;
    }
    if negative { None } else { Some(result) }
}

/// Port of cxxopts' `stringstream_parser<float>` (`std::istringstream >> float`
/// with libstdc++ `num_get`): skips leading whitespace, accepts the longest
/// `[+-]?digits[.digits][e[+-]digits]` prefix (trailing text is ignored) and
/// fails on no digits, a dangling exponent or overflow to infinity.
pub fn parse_f32(text: &str) -> Option<f32> {
    let b = text
        .trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c'])
        .as_bytes();
    let mut i = 0;
    if matches!(b.first(), Some(b'+' | b'-')) {
        i += 1;
    }
    let digits = |i: &mut usize| {
        let start = *i;
        while b.get(*i).is_some_and(u8::is_ascii_digit) {
            *i += 1;
        }
        *i - start
    };
    let mut n = digits(&mut i);
    if b.get(i) == Some(&b'.') {
        i += 1;
        n += digits(&mut i);
    }
    if n == 0 {
        return None;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if digits(&mut i) == 0 {
            return None;
        }
    }
    let v: f32 = std::str::from_utf8(b.get(..i)?).ok()?.parse().ok()?;
    if v.is_infinite() { None } else { Some(v) }
}

/// `truthy_pattern`: `(t|T)(rue)?|1`.
fn is_true_text(s: &str) -> bool {
    matches!(s, "t" | "T" | "true" | "True" | "1")
}

/// `falsy_pattern`: `(f|F)(alse)?|0`.
fn is_false_text(s: &str) -> bool {
    matches!(s, "f" | "F" | "false" | "False" | "0")
}

fn is_alnum(c: u8) -> bool {
    c.is_ascii_alphanumeric()
}

/// `ParseArgument` (the `option_matcher` regex):
/// `--([[:alnum:]][-_[:alnum:]\.]+)(=(.*))?|-([[:alnum:]].*)`.
/// Returns (name, Some(value) if `=` given, grouping).
fn parse_argument(arg: &str) -> Option<(String, Option<String>, bool)> {
    let b = arg.as_bytes();
    if let Some(rest) = arg.strip_prefix("--") {
        let rb = rest.as_bytes();
        let name_end = rb.iter().position(|&c| c == b'=').unwrap_or(rb.len());
        let name = rest.get(..name_end)?;
        let nb = name.as_bytes();
        if nb.len() < 2 || !is_alnum(nb[0]) {
            return None;
        }
        if !nb[1..]
            .iter()
            .all(|&c| is_alnum(c) || c == b'-' || c == b'_' || c == b'.')
        {
            return None;
        }
        let value = if name_end < rb.len() {
            Some(rest.get(name_end + 1..)?.to_string())
        } else {
            None
        };
        return Some((name.to_string(), value, false));
    }
    if b.len() >= 2 && b[0] == b'-' && is_alnum(b[1]) {
        return Some((arg.get(1..)?.to_string(), None, true));
    }
    None
}

impl Options {
    /// `cxxopts::Options(program, "")` + `custom_help("[OPTION...]")` + `set_width(100)`.
    pub fn new(program: &str) -> Options {
        Options {
            program: program.to_string(),
            custom_help: "[OPTION...]".into(),
            positional_help: String::new(),
            width: 100,
            allow_unrecognised: false,
            specs: Vec::new(),
            positional: Vec::new(),
        }
    }

    /// `custom_help`.
    pub fn custom_help(&mut self, s: &str) {
        self.custom_help = s.to_string();
    }

    /// `set_width`.
    pub fn set_width(&mut self, w: usize) {
        self.width = w;
    }

    /// `allow_unrecognised_options`.
    pub fn allow_unrecognised_options(&mut self) {
        self.allow_unrecognised = true;
    }

    /// `add_options()(...)`.
    pub fn add(&mut self, spec: OptionSpec) {
        self.specs.push(spec);
    }

    /// `parse_positional`.
    pub fn parse_positional(&mut self, names: &[&str]) {
        self.positional = names.iter().map(|s| (*s).to_string()).collect();
    }

    /// `positional_help`.
    pub fn positional_help(&mut self, s: &str) {
        self.positional_help = s.to_string();
    }

    /// `program()`.
    pub fn program(&self) -> &str {
        &self.program
    }

    fn find(&self, name: &str) -> Option<&OptionSpec> {
        self.specs
            .iter()
            .find(|s| s.long == name || s.short.as_deref() == Some(name))
    }

    /// Canonical key of an option: its long name (or short if it has none).
    fn key(spec: &OptionSpec) -> String {
        if spec.long.is_empty() {
            spec.short.clone().unwrap_or_default()
        } else {
            spec.long.clone()
        }
    }

    fn parse_option(
        &self,
        result: &mut ParseResult,
        spec: &OptionSpec,
        arg: &str,
    ) -> Result<(), ParseError> {
        if spec.is_boolean && !is_true_text(arg) && !is_false_text(arg) {
            return Err(ParseError(format!("Argument '{arg}' failed to parse")));
        }
        if spec.is_u32 && parse_u32(arg).is_none() {
            return Err(ParseError(format!("Argument '{arg}' failed to parse")));
        }
        if spec.is_f32 && parse_f32(arg).is_none() {
            return Err(ParseError(format!("Argument '{arg}' failed to parse")));
        }
        result
            .values
            .entry(Self::key(spec))
            .or_default()
            .push(arg.to_string());
        Ok(())
    }

    fn checked_parse_arg(
        &self,
        result: &mut ParseResult,
        args: &[String],
        current: &mut usize,
        spec: &OptionSpec,
        name: &str,
    ) -> Result<(), ParseError> {
        if let Some(implicit) = &spec.implicit_value {
            return self.parse_option(result, spec, implicit);
        }
        match args.get(*current + 1) {
            Some(next) => {
                *current += 1;
                self.parse_option(result, spec, next)
            }
            None => Err(ParseError(format!(
                "Option '{name}' is missing an argument"
            ))),
        }
    }

    fn consume_positional(
        &self,
        result: &mut ParseResult,
        a: &str,
        next: &mut usize,
    ) -> Result<bool, ParseError> {
        while let Some(pname) = self.positional.get(*next) {
            let Some(spec) = self.find(pname) else {
                return Err(ParseError(format!("Option '{pname}' does not exist")));
            };
            if !spec.is_container {
                if result.count(&Self::key(spec)) == 0 {
                    self.parse_option(result, spec, a)?;
                    *next += 1;
                    return Ok(true);
                }
                *next += 1;
                continue;
            }
            self.parse_option(result, spec, a)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Port of `OptionParser::parse`. `args[0]` is the program name.
    pub fn parse(&self, args: &[String]) -> Result<ParseResult, ParseError> {
        let mut result = ParseResult::default();
        let mut current = 1usize;
        let mut consume_remaining = false;
        let mut next_positional = 0usize;

        while let Some(arg) = args.get(current) {
            if arg == "--" {
                consume_remaining = true;
                current += 1;
                break;
            }
            match parse_argument(arg) {
                None => {
                    // not a flag, but if it starts with a `-`, then it's an error
                    if arg.len() > 1 && arg.starts_with('-') && !self.allow_unrecognised {
                        return Err(ParseError(format!(
                            "Argument '{arg}' starts with a - but has incorrect syntax"
                        )));
                    }
                    if !self.consume_positional(&mut result, arg, &mut next_positional)? {
                        result.unmatched.push(arg.clone());
                    }
                }
                Some((s, _, true)) => {
                    let chars: Vec<char> = s.chars().collect();
                    for (i, c) in chars.iter().enumerate() {
                        let name = c.to_string();
                        let Some(spec) = self.find(&name).cloned() else {
                            if self.allow_unrecognised {
                                result.unmatched.push(format!("-{c}"));
                                continue;
                            }
                            return Err(ParseError(format!("Option '{name}' does not exist")));
                        };
                        if i + 1 == chars.len() {
                            // it must be the last argument
                            self.checked_parse_arg(&mut result, args, &mut current, &spec, &name)?;
                        } else if let Some(implicit) = &spec.implicit_value {
                            self.parse_option(&mut result, &spec, implicit)?;
                        } else {
                            let rest: String =
                                chars.get(i + 1..).unwrap_or_default().iter().collect();
                            self.parse_option(&mut result, &spec, &rest)?;
                            break;
                        }
                    }
                }
                Some((name, value, false)) => {
                    let Some(spec) = self.find(&name).cloned() else {
                        if self.allow_unrecognised {
                            // keep unrecognised options in argument list, skip to next argument
                            result.unmatched.push(arg.clone());
                            current += 1;
                            continue;
                        }
                        return Err(ParseError(format!("Option '{name}' does not exist")));
                    };
                    match value {
                        Some(v) => self.parse_option(&mut result, &spec, &v)?,
                        None => {
                            self.checked_parse_arg(&mut result, args, &mut current, &spec, &name)?
                        }
                    }
                }
            }
            current += 1;
        }

        for spec in &self.specs {
            if let Some(d) = &spec.default_value {
                result.defaults.insert(Self::key(spec), d.clone());
            }
        }

        if consume_remaining {
            while let Some(arg) = args.get(current) {
                if !self.consume_positional(&mut result, arg, &mut next_positional)? {
                    break;
                }
                current += 1;
            }
            // adjust argv for any that couldn't be swallowed
            result.unmatched.extend(args.iter().skip(current).cloned());
        }
        Ok(result)
    }

    fn format_option(spec: &OptionSpec) -> String {
        let mut result = String::from("  ");
        match &spec.short {
            Some(s) => {
                result.push('-');
                result.push_str(s);
                if !spec.long.is_empty() {
                    result.push(',');
                }
            }
            None => result.push_str("   "),
        }
        if !spec.long.is_empty() {
            result.push_str(" --");
            result.push_str(&spec.long);
        }
        let arg = if spec.arg_help.is_empty() {
            "arg"
        } else {
            spec.arg_help.as_str()
        };
        if !spec.is_boolean {
            match &spec.implicit_value {
                Some(iv) => result.push_str(&format!(" [={arg}(={iv})]")),
                None => {
                    result.push(' ');
                    result.push_str(arg);
                }
            }
        }
        result
    }

    /// Port of `format_description` (tab expansion off).
    fn format_description(spec: &OptionSpec, start: usize, allowed: usize) -> String {
        let mut desc = spec.desc.clone();
        if let Some(d) = &spec.default_value
            && (!spec.is_boolean || d != "false")
        {
            if d.is_empty() {
                desc.push_str(" (default: \"\")");
            } else {
                desc.push_str(&format!(" (default: {d})"));
            }
        }
        desc.push(' ');
        let d = desc.as_bytes();
        let at = |i: usize| d.get(i).copied().unwrap_or(0);
        let slice = |a: usize, b: usize| d.get(a..b).unwrap_or_default();

        let mut result: Vec<u8> = Vec::new();
        let (mut current, mut previous, mut start_line, mut last_space) =
            (0usize, 0usize, 0usize, 0usize);
        let mut size = 0usize;
        let mut only_white_space = true;

        while current < d.len() {
            let mut append_new_line = false;
            if at(previous) == b' ' || at(previous) == b'\t' {
                last_space = current;
            }
            if at(current) != b' ' && at(current) != b'\t' {
                only_white_space = false;
            }
            while at(current) == b'\n' {
                previous = current;
                current += 1;
                append_new_line = true;
            }
            if !append_new_line && size >= allowed {
                if last_space != start_line {
                    current = last_space;
                    previous = current;
                }
                append_new_line = true;
            }
            if append_new_line {
                result.extend_from_slice(slice(start_line, current));
                start_line = current;
                last_space = current;
                if at(previous) != b'\n' {
                    result.push(b'\n');
                }
                result.extend(std::iter::repeat_n(b' ', start));
                if at(previous) != b'\n' {
                    result.extend_from_slice(slice(last_space, current));
                }
                only_white_space = true;
                size = 0;
            }
            previous = current;
            current += 1;
            size += 1;
        }
        // append whatever is left but ignore whitespace
        if !only_white_space {
            result.extend_from_slice(slice(start_line, previous));
        }
        String::from_utf8_lossy(&result).into_owned()
    }

    fn help_one_group(&self, group: &str) -> String {
        const OPTION_LONGEST: usize = 30;
        const OPTION_DESC_GAP: usize = 2;
        let shown: Vec<&OptionSpec> = self
            .specs
            .iter()
            .filter(|o| {
                o.group == group && !(!o.long.is_empty() && self.positional.contains(&o.long))
            })
            .collect();
        let mut result = String::new();
        if !group.is_empty() {
            result.push_str(&format!(" {group} options:\n"));
        }
        let formatted: Vec<String> = shown.iter().map(|o| Self::format_option(o)).collect();
        let longest = formatted
            .iter()
            .map(String::len)
            .max()
            .unwrap_or(0)
            .min(OPTION_LONGEST);
        // widest allowed description -- min 10 chars for helptext/line
        let mut allowed = 10;
        if self.width > allowed + longest + OPTION_DESC_GAP {
            allowed = self.width - longest - OPTION_DESC_GAP;
        }
        for (o, f) in shown.iter().zip(&formatted) {
            let d = Self::format_description(o, longest + OPTION_DESC_GAP, allowed);
            result.push_str(f);
            if f.len() > longest {
                result.push('\n');
                result.push_str(&" ".repeat(longest + OPTION_DESC_GAP));
            } else {
                result.push_str(&" ".repeat(longest + OPTION_DESC_GAP - f.len()));
            }
            result.push_str(&d);
            result.push('\n');
        }
        result
    }

    /// Port of `Options::help()` (all groups, sorted by name like `std::map`).
    pub fn help(&self) -> String {
        let mut result = format!("\nUsage:\n  {}", self.program);
        if !self.custom_help.is_empty() {
            result.push(' ');
            result.push_str(&self.custom_help);
        }
        if !self.positional.is_empty() && !self.positional_help.is_empty() {
            result.push(' ');
            result.push_str(&self.positional_help);
        }
        result.push_str("\n\n");
        let mut groups: Vec<&str> = self.specs.iter().map(|s| s.group.as_str()).collect();
        groups.sort_unstable();
        groups.dedup();
        for (i, g) in groups.iter().enumerate() {
            let text = self.help_one_group(g);
            if text.is_empty() {
                continue;
            }
            result.push_str(&text);
            if i + 1 < groups.len() {
                result.push('\n');
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_parser_u32() {
        assert_eq!(parse_u32("11"), Some(11));
        assert_eq!(parse_u32("0"), Some(0));
        assert_eq!(parse_u32("0x1F"), Some(31));
        assert_eq!(parse_u32("0x0"), Some(0));
        assert_eq!(parse_u32("4294967295"), Some(u32::MAX));
        assert_eq!(parse_u32("4294967296"), None);
        for bad in ["", "-1", "-0", "0x", "1a", "abc", " 1", "1.5", "0xg"] {
            assert_eq!(parse_u32(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn float_parser() {
        assert_eq!(parse_f32("1.5"), Some(1.5));
        assert_eq!(parse_f32(" -2e3x"), Some(-2000.0));
        assert_eq!(parse_f32(".5"), Some(0.5));
        assert_eq!(parse_f32("5."), Some(5.0));
        for bad in ["", "abc", ".", "1e", "1e+", "inf", "nan", "1e60", "-"] {
            assert_eq!(parse_f32(bad), None, "{bad:?}");
        }
    }

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    fn opts() -> Options {
        let mut o = Options::new("ktx test");
        o.add(OptionSpec::flag(
            "e,warnings-as-errors",
            "Treat warnings as errors.",
        ));
        o.add(OptionSpec::value("f,format", "Format", "text|json").default_value("text"));
        o.add(OptionSpec::value(
            "i,input-file",
            "The input file.",
            "filepath",
        ));
        o.parse_positional(&["input-file"]);
        o
    }

    #[test]
    fn parses_like_cxxopts() {
        let o = opts();
        let r = o
            .parse(&args(&["ktx", "-e", "--format=json", "a.ktx2", "b.ktx2"]))
            .unwrap_or_default();
        assert!(r.as_bool("warnings-as-errors"));
        assert_eq!(r.get("format"), Some("json"));
        assert_eq!(r.get("input-file"), Some("a.ktx2"));
        assert_eq!(r.unmatched(), &["b.ktx2".to_string()]);
        let r = o.parse(&args(&["ktx", "-fmini-json"])).unwrap_or_default();
        assert_eq!(r.get("format"), Some("mini-json"));
        assert_eq!(r.count("input-file"), 0);
        assert_eq!(
            o.parse(&args(&["ktx", "--unknown"])).err(),
            Some(ParseError("Option 'unknown' does not exist".into()))
        );
        assert_eq!(
            o.parse(&args(&["ktx", "--format"])).err(),
            Some(ParseError("Option 'format' is missing an argument".into()))
        );
        assert!(o.parse(&args(&["ktx", "--x"])).is_err());
    }
}
