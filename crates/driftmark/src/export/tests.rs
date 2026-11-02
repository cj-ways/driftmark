//! Tests of the export's own helpers; `tests/export.rs` runs the command on
//! real repositories.

use super::*;

#[test]
fn languages_come_from_the_code_blocks() {
    let html = "<pre data-k=\"1\" data-language=\"sh\"><code class=\"language-sh\">x</code></pre><pre data-k=\"2\" data-language=\"c&amp;\"></pre><pre data-k=\"3\" data-language=\"sh\"></pre>";
    assert_eq!(languages(html), ["sh", "c&"]);
    assert_eq!(languages("<p>no code</p>"), Vec::<String>::new());
}

#[test]
fn sizes_read_as_people_write_them() {
    assert_eq!(size_words(10_000_000), "10 MB");
    assert_eq!(size_words(500_000), "500 KB");
    assert_eq!(size_words(1_234), "1234 bytes");
}

#[test]
fn a_branch_range_is_named_after_its_branch() {
    assert_eq!(range_heading("auto"), "This checkout");
    assert_eq!(range_heading("main...feature/x"), "feature/x");
    assert_eq!(range_heading("main..."), "main...");
    assert_eq!(range_heading("a..b"), "a..b");
}
