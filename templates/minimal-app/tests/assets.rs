//! 固定字体资产：模板目录随源码携带字体，缺失路径给出带路径的诊断。

use minimal_app::{FONT_FILE, load_text_assets, load_text_assets_from};

#[test]
fn the_template_ships_the_font_next_to_its_sources() {
    // 测试的工作目录是包根；loader 的第二个候选正是包根下的 assets/。
    let assets = load_text_assets().expect("the template ships its font");
    drop(assets);
    assert!(std::path::Path::new("assets").join(FONT_FILE).is_file());
}

#[test]
fn a_missing_font_reports_the_checked_path() {
    let error =
        load_text_assets_from("assets/does-not-exist.otf").expect_err("a missing font is an error");
    let message = error.to_string();
    assert!(
        message.contains("assets/does-not-exist.otf"),
        "the diagnostic names the checked path: {message}"
    );
    assert!(
        message.contains("assets/"),
        "the diagnostic names the expected directory: {message}"
    );
}
