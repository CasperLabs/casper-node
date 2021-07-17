//! Generates reactors with routing from concise definitions. See `README.md` for details.

#![doc(html_root_url = "https://docs.rs/casper-node-macros/1.3.0")]
#![doc(
    html_favicon_url = "https://raw.githubusercontent.com/CasperLabs/casper-node/master/images/CasperLabs_Logo_Favicon_RGB_50px.png",
    html_logo_url = "https://raw.githubusercontent.com/CasperLabs/casper-node/master/images/CasperLabs_Logo_Symbol_RGB.png",
    test(attr(forbid(warnings)))
)]
#![warn(missing_docs, trivial_casts, trivial_numeric_casts)]

mod gen;
mod parse;
mod rust_type;
mod util;

use proc_macro::TokenStream;
use syn::parse_macro_input;

use parse::ReactorDefinition;

/// Generates a new reactor implementation, along with types.
#[proc_macro]
pub fn reactor(input: TokenStream) -> TokenStream {
    let mut def = parse_macro_input!(input as ReactorDefinition);

    // Insert the control announcements.
    def.inject_control_announcements();

    let mut output: proc_macro2::TokenStream = Default::default();

    output.extend(gen::generate_reactor(&def));
    output.extend(gen::generate_reactor_types(&def));
    output.extend(gen::generate_reactor_impl(&def));

    output.into()
}
