#!/usr/bin/env python3
"""Translate KTX-Software lib/src/gl_format.h and vk_format.h into Rust.

Writes src/gl_format.rs and src/vk_format.rs. The headers use a small C subset
(switch/case groups, return, ternary, field assignment, ASTC SELECTED macros)
which is parsed by a tiny recursive-descent translator.
"""
import re
import sys
from pathlib import Path

ref = Path(sys.argv[1] if len(sys.argv) > 1 else "ref/ktx") / "lib/src"
KNOWN_VK = set(re.findall(r"(VK_FORMAT_\w+)\s*=", (ref / "vkformat_enum.h").read_text()))


def vk(name):
    if name not in KNOWN_VK:
        for suffix in ("_EXT", "_KHR", "_IMG"):
            if name.endswith(suffix) and name[: -len(suffix)] in KNOWN_VK:
                name = name[: -len(suffix)]
                break
        else:
            sys.exit(f"unknown {name}")
    return "VkFormat::" + name[len("VK_FORMAT_"):]


def strip_comments(s):
    s = re.sub(r"/\*.*?\*/", "", s, flags=re.S)
    return re.sub(r"//[^\n]*", "", s)


TOK = re.compile(r"\s*(0x[0-9A-Fa-f]+|\d+|\w+|->|\|\||&&|!=|==|\?|:|;|\(|\)|\{|\}|=|\*|\||,|##)")


def tokenize(s):
    toks, i = [], 0
    while i < len(s):
        m = TOK.match(s, i)
        if not m:
            if s[i:].strip() == "":
                break
            sys.exit(f"tokenize error near: {s[i:i+40]!r}")
        toks.append(m.group(1))
        i = m.end()
    return toks


FIELD = {"flags": "flags", "paletteSizeInBits": "palette_size_in_bits", "blockSizeInBits": "block_size_in_bits",
         "blockWidth": "block_width", "blockHeight": "block_height", "blockDepth": "block_depth",
         "minBlocksX": "min_blocks_x", "minBlocksY": "min_blocks_y"}


class P:
    def __init__(self, toks, ctx):
        self.t, self.i, self.ctx = toks, 0, ctx

    def peek(self, k=0):
        return self.t[self.i + k] if self.i + k < len(self.t) else None

    def eat(self, v=None):
        tok = self.t[self.i]
        if v is not None and tok != v:
            sys.exit(f"expected {v} got {tok} at {self.t[self.i-5:self.i+5]}")
        self.i += 1
        return tok

    def expr(self):
        # expression up to ';' or ':' at depth 0 (ternary handled)
        parts, depth = [], 0
        while True:
            tok = self.peek()
            if depth == 0 and tok in (";", ":", ")", "?"):
                break
            if tok == "(":
                depth += 1
            if tok == ")":
                depth -= 1
            parts.append(self.eat())
        e = self.conv_expr(parts)
        if self.peek() == "?":
            self.eat("?")
            a = self.expr()
            self.eat(":")
            b = self.expr()
            return f"if {e} {{ {a} }} else {{ {b} }}"
        return e

    def conv_expr(self, parts):
        if len(parts) >= 3 and parts[0] in ("SELECTED", "SELECTED_EXT"):
            base = parts[2]
            ext = "_EXT" if parts[0] == "SELECTED_EXT" else ""
            return (f"if mapping == RgbaAstcMapping::ToHdr {{ {vk('VK_FORMAT_' + base + '_SFLOAT_BLOCK' + ext)} }} "
                    f"else {{ {vk('VK_FORMAT_' + base + '_UNORM_BLOCK' + ext)} }}")
        out = []
        for p in parts:
            if p.startswith("VK_FORMAT_"):
                out.append(vk(p))
            else:
                out.append(p)
        s = " ".join(out)
        return s.replace("( ", "(").replace(" )", ")")

    def stmts(self, stop):
        out = []
        while self.peek() is not None and self.peek() not in stop:
            out.append(self.stmt())
        return out

    def stmt(self):
        tok = self.peek()
        if tok == "{":
            self.eat("{")
            body = self.stmts(("}",))
            self.eat("}")
            return "\n".join(body)
        if tok == "switch":
            self.eat()
            self.eat("(")
            scrut = self.expr()
            self.eat(")")
            self.eat("{")
            arms = []
            while self.peek() != "}":
                labels = []
                while self.peek() in ("case", "default"):
                    if self.eat() == "case":
                        lab = self.eat()
                        labels.append(vk(lab) if lab.startswith("VK_FORMAT_") else lab)
                    else:
                        labels.append("_")
                    self.eat(":")
                body = self.stmts(("case", "default", "}"))
                pat = "_" if "_" in labels else " | ".join(labels)
                arms.append(f"{pat} => {{\n" + "\n".join(b for b in body if b) + "\n}")
            self.eat("}")
            if not any(a.startswith("_ =>") for a in arms):
                arms.append("_ => {}")
            return f"match {scrut} {{\n" + "\n".join(arms) + "\n}"
        if tok == "return":
            self.eat()
            if self.peek() == ";":
                self.eat()
                return "return;"
            e = self.expr()
            self.eat(";")
            return f"return {e};"
        if tok == "break":
            self.eat()
            self.eat(";")
            return ""
        if tok == ";":
            self.eat()
            return ""
        if tok == "assert":
            # Debug-only in C; no effect in release builds.
            self.eat()
            depth = 0
            while True:
                t = self.eat()
                if t == "(":
                    depth += 1
                elif t == ")":
                    depth -= 1
                    if depth == 0:
                        break
            self.eat(";")
            return ""
        # assignment chain: pFormatSize->a = pFormatSize->b = value;
        targets = []
        while self.peek() == "pFormatSize" and self.peek(1) == "->":
            self.eat()
            self.eat("->")
            targets.append(FIELD[self.eat()])
            self.eat("=")
        if not targets:
            sys.exit(f"unknown statement at {self.t[self.i:self.i+8]}")
        e = self.expr()
        self.eat(";")
        return "\n".join(f"fs.{t} = {e};" for t in targets)


def func_body(src, name):
    m = re.search(r"static inline [\w ]+?\b" + name + r"\s*\(([^)]*)\)\s*\{", src)
    if not m:
        sys.exit(f"no function {name}")
    i, depth = m.end() - 1, 0
    for j in range(i, len(src)):
        if src[j] == "{":
            depth += 1
        elif src[j] == "}":
            depth -= 1
            if depth == 0:
                return src[i + 1:j]
    sys.exit("unbalanced")


def translate(src, name, ctx=None):
    body = func_body(src, name)
    body = re.sub(r"#define\s+SELECTED\w*\(x\)[^\n]*\n", "", body)
    body = re.sub(r"^\s*#.*$", "", body, flags=re.M)
    p = P(tokenize(body), ctx)
    return "\n".join(s for s in p.stmts((None,)) if s)


HDR = ["#![allow(non_snake_case, non_upper_case_globals, unused_parens, unreachable_code, clippy::field_reassign_with_default, clippy::erasing_op, clippy::match_same_arms, clippy::needless_return, unreachable_patterns, clippy::identity_op)]", ""]

# ------------------------------------------------------------------ gl_format.rs
gl_src = strip_comments((ref / "gl_format.h").read_text())
defines = []
seen = set()
for m in re.finditer(r"#define\s+(GL_\w+)\s+(0x[0-9A-Fa-f]+|\d+)", gl_src):
    if m.group(1) not in seen:
        seen.add(m.group(1))
        defines.append((m.group(1), m.group(2)))
gl = ["// @generated by scripts/gen_gl_format.py from lib/src/gl_format.h. Do not edit.",
      "//! Port of `gl_format.h`: OpenGL format constants and size queries."] + HDR + [
      "use crate::formatsize::*;", ""]
for n, v in defines:
    gl.append(f"pub const {n}: u32 = {v};")
gl += ["", "/// Port of `glGetFormatFromInternalFormat`.",
       "pub fn gl_get_format_from_internal_format(internal_format: u32) -> u32 {",
       "let internalFormat = internal_format;",
       translate(gl_src, "glGetFormatFromInternalFormat"), "}", "",
       "/// Port of `glGetTypeFromInternalFormat`.",
       "pub fn gl_get_type_from_internal_format(internal_format: u32) -> u32 {",
       "let internalFormat = internal_format;",
       translate(gl_src, "glGetTypeFromInternalFormat"), "}", "",
       "/// Port of `glGetTypeSizeFromType`.",
       "pub fn gl_get_type_size_from_type(type_: u32) -> u32 {",
       "let r#type = type_;",
       translate(gl_src, "glGetTypeSizeFromType"), "}", "",
       "/// Port of `glGetFormatSize`.",
       "pub fn gl_get_format_size(internal_format: u32) -> FormatSize {",
       "let internalFormat = internal_format;",
       "let mut fs = FormatSize::default();",
       translate(gl_src, "glGetFormatSize"), "fs", "}", ""]
Path("src/gl_format.rs").write_text("\n".join(gl).replace("(type)", "(r#type)").replace("match type ", "match r#type "))

# ------------------------------------------------------------------ vk_format.rs
vk_src = strip_comments((ref / "vk_format.h").read_text())
vkr = ["// @generated by scripts/gen_gl_format.py from lib/src/vk_format.h. Do not edit.",
       "//! Port of `vk_format.h`: OpenGL -> Vulkan format mapping and Vulkan format sizes."] + HDR + [
       "use crate::formatsize::*;", "use crate::gl_format::*;", "use crate::vkformat::VkFormat;", "",
       "/// Port of `ktx_rgba_astc_mapping_e`.",
       "#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]",
       "pub enum RgbaAstcMapping {", "    /// map to `VK_FORMAT_ASTC_*_SFLOAT_BLOCK` formats.", "    #[default]", "    ToHdr,",
       "    /// map to `VK_FORMAT_ASTC_*_UNORM_BLOCK` formats.", "    ToLdr,", "}", "",
       "/// Port of `vkGetFormatFromOpenGLFormat`.",
       "pub fn vk_get_format_from_opengl_format(format: u32, type_: u32) -> VkFormat {",
       "let r#type = type_;",
       translate(vk_src, "vkGetFormatFromOpenGLFormat"), "}", "",
       "/// Port of `vkGetFormatFromOpenGLType`.",
       "pub fn vk_get_format_from_opengl_type(type_: u32, num_components: u32, normalized: bool) -> VkFormat {",
       "let r#type = type_;", "let numComponents = num_components;",
       translate(vk_src, "vkGetFormatFromOpenGLType"), "}", "",
       "/// Port of `vkGetFormatFromOpenGLInternalFormat`.",
       "pub fn vk_get_format_from_opengl_internal_format(internal_format: u32, mapping: RgbaAstcMapping) -> VkFormat {",
       "let internalFormat = internal_format;",
       translate(vk_src, "vkGetFormatFromOpenGLInternalFormat"), "}", "",
       "/// Port of `vkGetFormatSize`.",
       "pub fn vk_get_format_size(format: VkFormat) -> FormatSize {",
       "let mut fs = FormatSize::default();",
       translate(vk_src, "vkGetFormatSize"), "fs", "}", ""]
out = "\n".join(vkr).replace("match type ", "match r#type ")
Path("src/vk_format.rs").write_text(out)
print("wrote src/gl_format.rs, src/vk_format.rs;", len(defines), "GL constants")
