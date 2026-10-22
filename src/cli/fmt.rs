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
        format!("{v}")
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
