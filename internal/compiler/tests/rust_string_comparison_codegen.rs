// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use i_slint_compiler::diagnostics::BuildDiagnostics;
use i_slint_compiler::generator::OutputFormat;
use i_slint_compiler::parser::parse;
use i_slint_compiler::{CompilerConfiguration, compile_syntax_node};

fn generate(source: &str) -> String {
    let mut diagnostics = BuildDiagnostics::default();
    let syntax_node = parse(source.into(), None, &mut diagnostics);
    let configuration = CompilerConfiguration::new(OutputFormat::Rust);
    let (document, diagnostics, loader) =
        spin_on::spin_on(compile_syntax_node(syntax_node, diagnostics, configuration));
    assert!(!diagnostics.has_errors(), "{:?}", diagnostics.to_string_vec());
    i_slint_compiler::generator::rust::generate(&document, &loader.compiler_config)
        .expect("Rust code generation failed")
        .to_string()
}

#[test]
fn string_function_parameter_comparison_borrows_the_parameter() {
    let generated = generate(
        r#"
        export global Strings {
            public pure function equals(value: string) -> bool {
                return !(value != "first" && value != "second");
            }
        }
        export component Test inherits Rectangle {}
        "#,
    );

    assert!(generated.contains("args . 0 . as_str ()"));
    assert!(generated.contains("!= \"first\""));
    assert!(generated.contains("!= \"second\""));
    assert!(!generated.contains("SharedString :: from (\"first\")"));
    assert!(!generated.contains("SharedString :: from (\"second\")"));
}
