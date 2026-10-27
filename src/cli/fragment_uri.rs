// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! Port of tools/ktx/fragment_uri.h (`SelectorRange`, `FragmentURI`).

use std::fmt;

pub type RangeIndex = u32;
pub const RANGE_END: RangeIndex = RangeIndex::MAX;

/// Half open range [begin, end).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HalfRange {
    pub begin: RangeIndex,
    pub end: RangeIndex,
}

/// Port of `SelectorRange`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectorRange {
    ranges: Vec<HalfRange>,
}

impl SelectorRange {
    pub fn new(begin: RangeIndex, end: RangeIndex) -> SelectorRange {
        SelectorRange {
            ranges: vec![HalfRange { begin, end }],
        }
    }

    /// `SelectorRange(all)`.
    pub fn all() -> SelectorRange {
        SelectorRange::new(0, RANGE_END)
    }

    /// `SelectorRange(index)`.
    pub fn index(index: RangeIndex) -> SelectorRange {
        SelectorRange::new(index, index.wrapping_add(1))
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.iter().all(|r| r.begin == r.end)
    }

    pub fn is_single(&self) -> bool {
        let Some(first) = self.ranges.first() else {
            return false;
        };
        let index = first.begin;
        self.ranges
            .iter()
            .all(|r| r.begin == index && r.end == index.wrapping_add(1))
    }

    pub fn is_multi(&self) -> bool {
        !self.is_single()
    }

    pub fn clear(&mut self) {
        self.ranges.clear();
    }

    pub fn is_undefined(&self) -> bool {
        self.ranges.is_empty()
    }

    pub fn add(&mut self, range: HalfRange) {
        self.ranges.push(range);
    }

    pub fn contains(&self, index: RangeIndex) -> bool {
        self.ranges
            .iter()
            .any(|r| r.begin <= index && index < r.end)
    }

    pub fn validate(&self, last: RangeIndex) -> bool {
        self.ranges
            .iter()
            .all(|r| r.begin <= last && (r.end <= last || r.end == RANGE_END))
    }

    pub fn last(&self) -> RangeIndex {
        let mut last: RangeIndex = 0;
        for r in &self.ranges {
            if r.begin != r.end && r.end > last.wrapping_add(1) {
                last = r.end.wrapping_sub(1);
            }
        }
        last
    }

    /// `== all`.
    pub fn is_all(&self) -> bool {
        *self == SelectorRange::all()
    }
}

/// `fmt::formatter<SelectorRange>`.
impl fmt::Display for SelectorRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_all() {
            return f.write_str("all");
        }
        if self.is_empty() {
            return f.write_str("none");
        }
        for (i, r) in self.ranges.iter().enumerate() {
            if i > 0 {
                f.write_str(",")?;
            }
            if r.begin.wrapping_add(1) == r.end {
                write!(f, "{}", r.begin)?;
            } else if r.end == RANGE_END {
                write!(f, "{}..last", r.begin)?;
            } else {
                write!(f, "{}..{}", r.begin, r.end.wrapping_sub(1))?;
            }
        }
        Ok(())
    }
}

/// Port of `FragmentURI`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FragmentURI {
    pub mip: SelectorRange,
    pub stratal: SelectorRange,
    pub facial: SelectorRange,
}

impl FragmentURI {
    pub fn validate(&self, num_levels: u32, num_layers: u32, num_faces: u32) -> bool {
        self.mip.validate(num_levels)
            && self.stratal.validate(num_layers)
            && self.facial.validate(num_faces)
    }
}

/// `std::stoi` errors; `what()` is "stoi" in libstdc++.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoiError {
    InvalidArgument,
    OutOfRange,
}

/// `std::stoi`: skips leading whitespace, accepts a sign and decimal digits,
/// ignores trailing characters.
pub fn stoi(s: &str) -> Result<i32, StoiError> {
    let t = s.trim_start_matches([' ', '\t', '\n', '\x0b', '\x0c', '\r']);
    let (neg, digits) = match t.as_bytes().first() {
        Some(b'-') => (true, t.get(1..).unwrap_or_default()),
        Some(b'+') => (false, t.get(1..).unwrap_or_default()),
        _ => (false, t),
    };
    let n = digits.bytes().take_while(u8::is_ascii_digit).count();
    if n == 0 {
        return Err(StoiError::InvalidArgument);
    }
    let mut v: i64 = 0;
    for c in digits.bytes().take(n) {
        v = v * 10 + i64::from(c - b'0');
        if v > i64::from(i32::MAX) + 1 {
            return Err(StoiError::OutOfRange);
        }
    }
    let v = if neg { -v } else { v };
    i32::try_from(v).map_err(|_| StoiError::OutOfRange)
}

/// Port of `parseHalfRange`; the error is the exception's `what()`.
pub fn parse_half_range(key: &str, s: &str) -> Result<HalfRange, String> {
    let (str_begin, str_end) = match s.find(',') {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, ""),
    };
    let parse = || -> Result<HalfRange, StoiError> {
        let begin = if str_begin.is_empty() {
            0
        } else {
            stoi(str_begin)? as u32
        };
        let end = if str_end.is_empty() {
            RANGE_END
        } else {
            (stoi(str_end)? as u32).wrapping_add(1)
        };
        Ok(HalfRange { begin, end })
    };
    parse().map_err(|_| format!("Invalid key-value \"{key}={s}\": stoi"))
}

/// Port of `parseFragmentURI`; the error is the exception's `what()`.
pub fn parse_fragment_uri(mut s: &str) -> Result<FragmentURI, String> {
    // Name and value components are separated by an equal sign (=),
    // while multiple name-value pairs are separated by an ampersand (&).
    let mut result = FragmentURI::default();
    loop {
        let index_amp = s.find('&');
        let key_value = index_amp.map_or(s, |i| &s[..i]);
        let (key, value) = match key_value.find('=') {
            Some(i) => (&key_value[..i], &key_value[i + 1..]),
            None => (key_value, ""),
        };
        match key {
            "m" | "%6D" => result.mip.add(parse_half_range(key, value)?),
            "a" | "%61" => result.stratal.add(parse_half_range(key, value)?),
            "t" | "%74" => return Err("Temporal selector (t) is not yet supported.".to_string()),
            "f" => result.facial.add(parse_half_range(key, value)?),
            "xyzwhd" => {
                return Err("Spatial selector (xyzwhd) is not yet supported.".to_string());
            }
            "" => {}
            _ => return Err(format!("Unknown key \"{key}\"")),
        }
        match index_amp {
            Some(i) => s = &s[i + 1..],
            None => break,
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_range() {
        assert_eq!(SelectorRange::all().to_string(), "all");
        assert_eq!(SelectorRange::index(3).to_string(), "3");
        assert_eq!(SelectorRange::new(10, RANGE_END).to_string(), "10..last");
        assert!(SelectorRange::index(0).is_single());
        assert!(SelectorRange::all().is_multi());
        assert_eq!(SelectorRange::new(2, 5).last(), 4);
        assert!(!SelectorRange::new(10, RANGE_END).validate(4));
        assert!(SelectorRange::new(0, RANGE_END).validate(4));
    }

    #[test]
    fn fragment_uri() {
        let u = parse_fragment_uri("m=1,2&a=0&f=").unwrap_or_default();
        assert_eq!(u.mip, SelectorRange::new(1, 3));
        assert_eq!(u.stratal, SelectorRange::new(0, RANGE_END));
        assert_eq!(u.facial, SelectorRange::new(0, RANGE_END));
        assert_eq!(
            parse_fragment_uri("m=x").err().as_deref(),
            Some("Invalid key-value \"m=x\": stoi")
        );
        assert_eq!(
            parse_fragment_uri("q=1").err().as_deref(),
            Some("Unknown key \"q\"")
        );
        assert_eq!(stoi(" -12ab"), Ok(-12));
        assert_eq!(stoi("99999999999"), Err(StoiError::OutOfRange));
        assert_eq!(stoi("-2147483648"), Ok(i32::MIN));
    }
}
