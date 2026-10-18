//! Tests of the drawer: merman with the shipped fonts, the cache, and a
//! drawer that draws nothing.

use std::fmt::Write as _;

use super::*;

const FLOWCHART: &str = "flowchart LR\n  A[Start here] --> B{Ready?}\n  B -->|yes| C[Done]";

#[test]
fn mermaid_is_drawn_clean_and_kept() {
    let drawer = Drawer::new();
    assert!(drawer.draws_mermaid());
    let svg = drawer
        .drawing(DiagramLanguage::Mermaid, FLOWCHART)
        .unwrap()
        .unwrap();
    assert!(svg.starts_with("<svg id=\"dm"), "{svg}");
    assert!(svg.contains("aria-label=\"Flowchart diagram\""), "{svg}");
    assert!(svg.contains("<p>Start here</p>"), "{svg}");
    assert!(!svg.contains("<style"), "the style block is removed");
    assert!(!svg.contains("style=\""), "no inline style is left");
    let again = drawer
        .drawing(DiagramLanguage::Mermaid, FLOWCHART)
        .unwrap()
        .unwrap();
    assert!(
        Arc::ptr_eq(&svg, &again),
        "the second drawing is the kept one"
    );
    let through = DiagramRender::draw(
        &drawer,
        &Diagram {
            language: DiagramLanguage::Mermaid,
            code: FLOWCHART,
            block: 3,
        },
    );
    assert_eq!(through, Ok(Some(svg.to_string())));
}

#[test]
fn a_diagram_merman_cannot_read_gives_its_message() {
    let drawer = Drawer::new();
    let error = drawer
        .drawing(DiagramLanguage::Mermaid, "flowchart LR\n  A --> ")
        .unwrap_err();
    assert_ne!(error, "");
    assert!(!error.contains('\n'), "{error}");
}

#[test]
fn large_er_diagrams_fit_the_measurement_budget() {
    // merman issue 207: 340 tables of 2 columns ran out of the default
    // budget of 4,096 text measurements.
    let mut code = String::from("erDiagram\n");
    for table in 0..360 {
        let _ = writeln!(code, "  T{table} {{\n    string name\n    uuid id PK\n  }}");
    }
    let drawer = Drawer::new();
    let drawn = drawer.drawing(DiagramLanguage::Mermaid, code.trim_end());
    assert!(matches!(drawn, Ok(Some(_))), "{drawn:?}");
}

#[test]
fn a_drawer_without_programs_leaves_diagrams_to_the_browser() {
    let drawer = Drawer::none();
    assert!(!drawer.draws_mermaid());
    assert!(!drawer.draws_d2());
    assert_eq!(
        drawer.drawing(DiagramLanguage::Mermaid, FLOWCHART),
        Ok(None)
    );
    assert_eq!(drawer.drawing(DiagramLanguage::D2, "a -> b"), Ok(None));
}

#[cfg(unix)]
#[test]
fn d2_drawings_are_cleaned_too() {
    use std::os::unix::fs::PermissionsExt;
    let folder = std::env::temp_dir().join(format!("driftmark-drawer-d2-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let program = folder.join("d2");
    std::fs::write(
        &program,
        "#!/bin/sh\ncat >/dev/null\nprintf '<svg viewBox=\"0 0 9 9\"><script>alert(1)</script><rect id=\"r\" width=\"1\" height=\"1\"/></svg>'\n",
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Wait until the new script can run (see `draw_retrying`).
    let _ = d2::tests::draw_retrying(&d2::D2::at(program.clone()), "a -> b");
    let mut drawer = Drawer::none();
    drawer.d2 = Some(d2::D2::at(program));
    assert!(drawer.draws_d2());
    let svg = drawer
        .drawing(DiagramLanguage::D2, "a -> b")
        .unwrap()
        .unwrap();
    assert!(!svg.contains("script"), "{svg}");
    assert!(svg.contains("<rect id=\"dm"), "{svg}");
    let broken = folder.join("broken");
    std::fs::write(
        &broken,
        "#!/bin/sh\ncat >/dev/null\nprintf '<html></html>'\n",
    )
    .unwrap();
    std::fs::set_permissions(&broken, std::fs::Permissions::from_mode(0o755)).unwrap();
    let _ = d2::tests::draw_retrying(&d2::D2::at(broken.clone()), "c -> d");
    drawer.d2 = Some(d2::D2::at(broken));
    let error = drawer.drawing(DiagramLanguage::D2, "c -> d").unwrap_err();
    assert!(
        error.starts_with("the drawing could not be made safe: "),
        "{error}"
    );
}
