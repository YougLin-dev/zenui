// crates/core/tests/compile_fail.rs
//
// `ADR-0021` decision 1: "this must fail to compile *because of* error code
// X" is carried by trybuild UI tests and a checked-in `.stderr` snapshot, which
// matches the diagnostic — code, span and message — word for word.
//
// `compile_fail` doctests were carried here before and could not do that: they
// check only "does not compile", so a typo keeps them green. The cases live in
// `tests/ui/` as crate sources, where trybuild compiles them.

#[test]
fn the_named_contracts_do_not_compile_for_the_named_reason() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/*.rs");
}
