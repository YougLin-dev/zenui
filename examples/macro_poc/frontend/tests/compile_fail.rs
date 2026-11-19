#[test]
fn unsupported_macro_inputs_emit_native_diagnostics() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/*.rs");
}
