//! Port of the helpers in tools/ktx/utility.h used by the tools.

/// Port of `align` (round `value` up to a multiple of `alignment`).
pub fn align(value: u64, alignment: u64) -> u64 {
    if alignment == 0 {
        return value;
    }
    (alignment - 1).wrapping_add(value) / alignment * alignment
}

/// Port of `ceil_div`.
pub fn ceil_div(x: u32, y: u32) -> u32 {
    if y == 0 {
        return 0;
    }
    x.wrapping_add(y - 1) / y
}

/// Port of `log2` (floor; 0 for 0).
pub fn log2(v: u32) -> u32 {
    if v == 0 { 0 } else { 31 - v.leading_zeros() }
}

/// Port of `popcount`.
pub fn popcount(value: u32) -> u32 {
    value.count_ones()
}

/// Port of `sequenceLength`.
fn sequence_length(lead: u8) -> usize {
    if lead & 0b1000_0000 == 0 {
        1
    } else if lead & 0b1110_0000 == 0b1100_0000 {
        2
    } else if lead & 0b1111_0000 == 0b1110_0000 {
        3
    } else if lead & 0b1111_1000 == 0b1111_0000 {
        4
    } else {
        0
    }
}

fn is_overlong_sequence(codepoint: u32, length: usize) -> bool {
    if codepoint < 0x80 {
        length != 1
    } else if codepoint < 0x800 {
        length != 2
    } else if codepoint < 0x10000 {
        length != 3
    } else {
        false
    }
}

fn is_codepoint_valid(codepoint: u32) -> bool {
    codepoint <= 0x0010_FFFF && !(0xD800..=0xDBFF).contains(&codepoint)
}

/// Port of `advanceUTF8`: on success returns the sequence length.
fn advance_utf8(text: &[u8]) -> Option<usize> {
    let lead = *text.first()?;
    let length = sequence_length(lead);
    if length == 0 || text.len() < length {
        return None;
    }
    let seq = text.get(..length)?;
    if seq.iter().skip(1).any(|&b| b & 0b1100_0000 != 0b1000_0000) {
        return None;
    }
    let t = |i: usize| u32::from(seq[i] & 0b0011_1111);
    let codepoint = match length {
        1 => u32::from(lead),
        2 => (u32::from(lead & 0b0001_1111) << 6) | t(1),
        3 => (u32::from(lead & 0b0000_1111) << 12) | (t(1) << 6) | t(2),
        _ => (u32::from(lead & 0b0000_0111) << 18) | (t(1) << 12) | (t(2) << 6) | t(3),
    };
    if !is_codepoint_valid(codepoint) || is_overlong_sequence(codepoint, length) {
        return None;
    }
    Some(length)
}

/// Port of `validateUTF8`: `None` if valid, else the first invalid index.
pub fn validate_utf8(text: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i < text.len() {
        match advance_utf8(text.get(i..).unwrap_or_default()) {
            Some(n) => i += n,
            None => return Some(i),
        }
    }
    None
}

/// Port of `escape_json_copy`.
pub fn escape_json_copy(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for &c in s {
        match c {
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'"' => out.extend_from_slice(b"\\\""),
            b'\n' => out.extend_from_slice(b"\\n"),
            _ => out.push(c),
        }
    }
    out
}

/// Port of `fmtInFile`.
pub fn fmt_in_file(filepath: &str) -> String {
    if filepath == "-" {
        "stdin".to_string()
    } else {
        filepath.to_string()
    }
}

/// Port of `fmtOutFile`.
pub fn fmt_out_file(filepath: &str) -> String {
    if filepath == "-" {
        "stdout".to_string()
    } else {
        filepath.to_string()
    }
}

/// Port of `PrintIndent`: each call writes `indent_width * (indent_base + depth)`
/// spaces and then the text.
#[derive(Debug, Clone, Copy)]
pub struct PrintIndent {
    pub indent_base: usize,
    pub indent_width: usize,
}

impl PrintIndent {
    /// Appends the indented `text` to `out`.
    pub fn print(&self, out: &mut Vec<u8>, depth: usize, text: &[u8]) {
        out.extend(std::iter::repeat_n(
            b' ',
            self.indent_width * (self.indent_base + depth),
        ));
        out.extend_from_slice(text);
    }
}

/// Port of `errnoMessage` for an I/O error (the `strerror` text).
pub fn io_error_message(e: &std::io::Error) -> String {
    let s = e.to_string();
    // Rust appends " (os error N)" to the strerror text.
    match s.rfind(" (os error ") {
        Some(p) => s.get(..p).unwrap_or_default().to_string(),
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8() {
        assert_eq!(validate_utf8("KTXwriter é 😀".as_bytes()), None);
        assert_eq!(validate_utf8(b"ab\xC0\x80"), Some(2));
        assert_eq!(validate_utf8(b"a\xED\xA0\x80"), Some(1));
        assert_eq!(validate_utf8(b"\xE2\x82"), Some(0));
    }

    #[test]
    fn math() {
        assert_eq!(align(13, 4), 16);
        assert_eq!(log2(1), 0);
        assert_eq!(log2(1024), 10);
        assert_eq!(log2(1023), 9);
        assert_eq!(ceil_div(5, 4), 2);
    }
}
