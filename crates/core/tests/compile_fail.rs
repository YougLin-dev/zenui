#[test]
fn the_named_contracts_do_not_compile_for_the_named_reason() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/*.rs");
}
