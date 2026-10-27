// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! Minimal `{fmt}` subset used by the tools' messages: `{}`, `{{`, `}}` and
//! `{:[0][width][d|b|X]}`. Output is bytes because keys and values from files
//! are printed raw, like the C++ tools do.

/// One formatting argument.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    /// Unsigned integer.
    U(u64),
    /// Signed integer.
    I(i64),
    /// Raw string bytes.
    S(Vec<u8>),
    /// A C `char`: printed as the byte itself.
    C(u8),
    /// A float (`{fmt}` shortest form for the non-finite values the tools print).
    F(f32),
    /// A bool (`true` / `false`).
    B(bool),
}

macro_rules! from_uint {
    ($($t:ty),*) => {$(
        impl From<$t> for Arg {
            fn from(v: $t) -> Arg {
                Arg::U(v as u64)
            }
        }
    )*};
}
from_uint!(u8, u16, u32, u64, usize);

impl From<i32> for Arg {
    fn from(v: i32) -> Arg {
        Arg::I(i64::from(v))
    }
}

impl From<i64> for Arg {
    fn from(v: i64) -> Arg {
        Arg::I(v)
    }
}

impl From<&str> for Arg {
    fn from(v: &str) -> Arg {
        Arg::S(v.as_bytes().to_vec())
    }
}

impl From<String> for Arg {
    fn from(v: String) -> Arg {
        Arg::S(v.into_bytes())
    }
}

impl From<&String> for Arg {
    fn from(v: &String) -> Arg {
        Arg::S(v.as_bytes().to_vec())
    }
}

impl From<&[u8]> for Arg {
    fn from(v: &[u8]) -> Arg {
        Arg::S(v.to_vec())
    }
}

impl From<char> for Arg {
    fn from(v: char) -> Arg {
        Arg::C(v as u8)
    }
}

impl From<f32> for Arg {
    fn from(v: f32) -> Arg {
        Arg::F(v)
    }
}

impl From<bool> for Arg {
    fn from(v: bool) -> Arg {
        Arg::B(v)
    }
}

/// Builds an argument list: `args![a, b, c]`.
#[macro_export]
#[doc(hidden)]
macro_rules! fmt_args {
    ($($a:expr),* $(,)?) => {
        vec![$($crate::cli::fmt::Arg::from($a)),*]
    };
}

fn float_str(v: f32) -> String {
    if v.is_nan() {
        if v.is_sign_negative() { "-nan" } else { "nan" }.to_string()
    } else if v.is_infinite() {
        if v < 0.0 { "-inf" } else { "inf" }.to_string()
    } else {
        float_shortest(v)
    }
}

/// Splits `{:e}` output into (digits, decimal exponent).
fn split_exp(e: &str) -> (String, i32) {
    let (mantissa, exp) = e.split_once('e').unwrap_or((e, "0"));
    let digits = mantissa.chars().filter(|c| *c != '.').collect();
    (digits, exp.parse().unwrap_or(0))
}

/// Shortest round-trip digits of a positive finite `v`. Dragonbox breaks an
/// exact tie between two shortest candidates towards the even digit, Rust
/// rounds it up.
fn shortest_digits(v: f32) -> (String, i32) {
    let (digits, exp) = split_exp(&format!("{v:e}"));
    // The exact decimal expansion (an f32 has at most 112 significant digits)
    let (exact, exact_exp) = split_exp(&format!("{:.120e}", f64::from(v)));
    let n = digits.len();
    let (Some(truncated), Some(rest)) = (exact.get(..n), exact.get(n..)) else {
        return (digits, exp);
    };
    let tie = rest
        .strip_prefix('5')
        .is_some_and(|r| r.bytes().all(|c| c == b'0'));
    let even = truncated
        .bytes()
        .last()
        .is_some_and(|c| (c - b'0').is_multiple_of(2));
    if tie && even && (truncated != digits || exp != exact_exp) {
        let candidate = format!("{}.{}e{exact_exp}", &truncated[..1], &truncated[1..]);
        if candidate.parse::<f32>() == Ok(v) {
            return (truncated.to_string(), exact_exp);
        }
    }
    (digits, exp)
}

/// `{fmt}` 11 `{}` of a finite `float`: shortest round-trip digits, fixed
/// notation for decimal exponents in [-4, 7) and `d.ddde±XX` otherwise.
pub fn float_shortest(v: f32) -> String {
    let sign = if v.is_sign_negative() { "-" } else { "" };
    if v == 0.0 {
        return format!("{sign}0");
    }
    let (digits, output_exp) = shortest_digits(v.abs());
    let n = digits.len() as i32;
    if !(-4..7).contains(&output_exp) {
        let (first, rest) = digits.split_at(1);
        let point = if rest.is_empty() { "" } else { "." };
        let exp_sign = if output_exp < 0 { '-' } else { '+' };
        return format!(
            "{sign}{first}{point}{rest}e{exp_sign}{:02}",
            output_exp.abs()
        );
    }
    let int_digits = output_exp + 1;
    if int_digits >= n {
        format!("{sign}{digits}{}", "0".repeat((int_digits - n) as usize))
    } else if int_digits > 0 {
        let (a, b) = digits.split_at(int_digits as usize);
        format!("{sign}{a}.{b}")
    } else {
        format!("{sign}0.{}{digits}", "0".repeat((-int_digits) as usize))
    }
}

fn render(arg: &Arg, ty: Option<u8>) -> Vec<u8> {
    match (arg, ty) {
        (Arg::U(v), Some(b'b')) => format!("{v:b}").into_bytes(),
        (Arg::U(v), Some(b'X')) => format!("{v:X}").into_bytes(),
        (Arg::U(v), Some(b'x')) => format!("{v:x}").into_bytes(),
        (Arg::U(v), _) => v.to_string().into_bytes(),
        (Arg::I(v), Some(b'X')) => format!("{v:X}").into_bytes(),
        (Arg::I(v), _) => v.to_string().into_bytes(),
        (Arg::S(s), _) => s.clone(),
        (Arg::C(c), Some(b'd')) => c.to_string().into_bytes(),
        (Arg::C(c), _) => vec![*c],
        (Arg::F(f), _) => float_str(*f).into_bytes(),
        (Arg::B(b), _) => if *b { "true" } else { "false" }.as_bytes().to_vec(),
    }
}

/// `{fmt}`'s `format_error` text for a placeholder without an argument.
pub const ARGUMENT_NOT_FOUND: &str = "argument not found";

/// Formats `fmt` with `args`, failing like `fmt::format` (which throws
/// `format_error`) when a placeholder has no argument.
pub fn try_format(fmt: &str, args: &[Arg]) -> Result<Vec<u8>, &'static str> {
    let wanted = count_placeholders(fmt);
    if wanted.is_some_and(|n| n > args.len()) {
        return Err(ARGUMENT_NOT_FOUND);
    }
    Ok(format(fmt, args))
}

/// Highest argument index + 1 referenced by `fmt`.
fn count_placeholders(fmt: &str) -> Option<usize> {
    let f = fmt.as_bytes();
    let (mut i, mut next, mut max) = (0, 0usize, 0usize);
    while let Some(&c) = f.get(i) {
        if (c == b'{' || c == b'}') && f.get(i + 1) == Some(&c) {
            i += 2;
            continue;
        }
        if c == b'{' {
            let end = i + f.get(i..)?.iter().position(|&x| x == b'}')?;
            let spec = f.get(i + 1..end)?;
            let index = spec.split(|&x| x == b':').next().unwrap_or_default();
            let idx = if index.is_empty() {
                next += 1;
                next - 1
            } else {
                std::str::from_utf8(index).ok()?.parse().ok()?
            };
            max = max.max(idx + 1);
            i = end + 1;
        } else {
            i += 1;
        }
    }
    Some(max)
}

/// Formats `fmt` with `args` (missing arguments print nothing).
pub fn format(fmt: &str, args: &[Arg]) -> Vec<u8> {
    let f = fmt.as_bytes();
    let mut out = Vec::with_capacity(f.len() + 16);
    let mut next = 0usize;
    let mut i = 0usize;
    while let Some(&c) = f.get(i) {
        match c {
            b'{' if f.get(i + 1) == Some(&b'{') => {
                out.push(b'{');
                i += 2;
            }
            b'}' if f.get(i + 1) == Some(&b'}') => {
                out.push(b'}');
                i += 2;
            }
            b'{' => {
                let end = match f.get(i..).and_then(|s| s.iter().position(|&x| x == b'}')) {
                    Some(e) => i + e,
                    None => {
                        out.extend_from_slice(f.get(i..).unwrap_or_default());
                        break;
                    }
                };
                let spec = f.get(i + 1..end).unwrap_or_default();
                let (index, spec) = match spec.iter().position(|&x| x == b':') {
                    Some(p) => (
                        spec.get(..p).unwrap_or_default(),
                        spec.get(p + 1..).unwrap_or_default(),
                    ),
                    None => (spec, &[][..]),
                };
                let arg_index = if index.is_empty() {
                    next += 1;
                    next - 1
                } else {
                    std::str::from_utf8(index)
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0)
                };
                let zero = spec.first() == Some(&b'0');
                let digits: Vec<u8> = spec
                    .iter()
                    .copied()
                    .take_while(u8::is_ascii_digit)
                    .collect();
                let width: usize = std::str::from_utf8(&digits)
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let ty = spec.get(digits.len()).copied();
                if let Some(arg) = args.get(arg_index) {
                    let s = render(arg, ty);
                    let numeric = !matches!(arg, Arg::S(_))
                        && !(matches!(arg, Arg::C(_)) && ty != Some(b'd'));
                    if s.len() < width {
                        let pad = width - s.len();
                        if zero {
                            let (sign, digits) = match s.split_first() {
                                Some((b'-', rest)) => (&s[..1], rest),
                                _ => (&s[..0], &s[..]),
                            };
                            out.extend_from_slice(sign);
                            out.extend(std::iter::repeat_n(b'0', pad));
                            out.extend_from_slice(digits);
                        } else if numeric {
                            out.extend(std::iter::repeat_n(b' ', pad));
                            out.extend_from_slice(&s);
                        } else {
                            out.extend_from_slice(&s);
                            out.extend(std::iter::repeat_n(b' ', pad));
                        }
                    } else {
                        out.extend_from_slice(&s);
                    }
                }
                i = end + 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_shortest_like_fmt() {
        for (v, want) in [
            (0.0f32, "0"),
            (-0.0, "-0"),
            (3.25, "3.25"),
            (f32::from_bits(0x4048_f5c3), "3.14"),
            (42.0, "42"),
            (1120.0, "1120"),
            (1234567.0, "1234567"),
            (12345678.0, "1.2345678e+07"),
            (1.6023064e19, "1.6023064e+19"),
            (0.00010716915, "0.00010716915"),
            (1.0860433e-05, "1.0860433e-05"),
            (1.8436203e-27, "1.8436203e-27"),
            (1e10, "1e+10"),
            (-4.2, "-4.2"),
            (2.0f32.powi(-12), "0.00024414062"),
            (0.5, "0.5"),
        ] {
            assert_eq!(float_shortest(v), want);
        }
    }

    #[test]
    fn specs() {
        let s = |f: &str, a: Vec<Arg>| String::from_utf8_lossy(&format(f, &a)).into_owned();
        assert_eq!(
            s("{}-{:04}: {}", fmt_args!["error", 3001u16, "x"]),
            "error-3001: x"
        );
        assert_eq!(s("{:08b}", fmt_args![0xC1u8]), "11000001");
        assert_eq!(s("0x{:08X}", fmt_args![0xABu32]), "0x000000AB");
        assert_eq!(s("{:d} {}", fmt_args![7u8, "a"]), "7 a");
        assert_eq!(s("{{{}}}", fmt_args!['r']), "{r}");
        assert_eq!(s("{} {}", fmt_args![f32::INFINITY, f32::NAN]), "inf nan");
    }
}
