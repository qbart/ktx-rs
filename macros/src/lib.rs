//! Internal proc macros of the `ktx` crate.

use proc_macro::{Delimiter, Group, Ident, Punct, Spacing, Span, TokenStream, TokenTree};

/// `#[isa]` on an item expands to `isa_fn! { item }`. Each instruction set copy of the astcenc
/// compressor defines its own `isa_fn!` (adding its `#[target_feature]`), so one source file
/// compiles once per instruction set.
#[proc_macro_attribute]
pub fn isa(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return "compile_error!(\"#[isa] takes no arguments\");"
            .parse()
            .unwrap_or_default();
    }
    [
        TokenTree::Ident(Ident::new("isa_fn", Span::call_site())),
        TokenTree::Punct(Punct::new('!', Spacing::Alone)),
        TokenTree::Group(Group::new(Delimiter::Brace, item)),
    ]
    .into_iter()
    .collect()
}
