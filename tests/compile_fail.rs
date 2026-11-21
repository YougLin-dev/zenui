//! 通过公开 `zenui` facade 编译失败用例，固定编译期诊断。
//!
//! 用例与 `.stderr` 的维护方式见 `docs/实现/验证重点.md` 的「编译失败测试」。

#[test]
fn public_macro_inputs_compile_or_emit_expected_diagnostics() {
    let cases = trybuild::TestCases::new();
    cases.pass("tests/compile_fail/pass/*.rs");
    cases.compile_fail("tests/compile_fail/*.rs");
}
