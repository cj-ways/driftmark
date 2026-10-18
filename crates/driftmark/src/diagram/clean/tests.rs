//! Tests of the cleaner's parts. `tests/diagrams.rs` runs the hostile
//! fixtures through the whole of it.

use super::*;

fn cleaned(svg: &str) -> String {
    clean(svg, "dm1").unwrap()
}

#[test]
fn entities_resolve_to_their_characters() {
    assert_eq!(
        cleaned("<svg><text>&lt;&gt;&amp;&quot;&apos;&nbsp;&#65;&#x42;</text></svg>"),
        "<svg aria-label=\"Diagram\" role=\"img\"><text>&lt;&gt;&amp;&quot;'\u{a0}AB</text></svg>"
    );
    assert!(clean("<svg><text>&copy;</text></svg>", "dm1").is_err());
    assert!(clean("<svg><text>&#0;</text></svg>", "dm1").is_err());
}

#[test]
fn only_one_svg_root_is_accepted() {
    assert!(clean("", "dm1").is_err());
    assert!(clean("<!-- nothing -->", "dm1").is_err());
    assert!(clean("<g/>", "dm1").is_err());
    assert!(clean("<svg/><svg/>", "dm1").is_err());
    assert!(clean("<svg><g></svg>", "dm1").is_err());
    assert_eq!(
        cleaned("<?xml version=\"1.0\"?><svg/>"),
        "<svg aria-label=\"Diagram\" role=\"img\"/>"
    );
}

#[test]
fn cdata_and_comments() {
    assert_eq!(
        cleaned("<svg><text><![CDATA[a < b]]><!-- x --></text></svg>"),
        "<svg aria-label=\"Diagram\" role=\"img\"><text>a &lt; b</text></svg>"
    );
}

#[test]
fn the_root_gets_its_natural_size_and_a_name() {
    assert_eq!(
        cleaned(
            "<svg width=\"100%\" height=\"9\" viewBox=\"4 4 632.5 187\" aria-roledescription=\"er\"/>"
        ),
        "<svg aria-roledescription=\"er\" viewBox=\"4 4 632.5 187\" width=\"633\" height=\"187\" aria-label=\"Entity relationship diagram\" role=\"img\"/>"
    );
    assert_eq!(natural_size("0,0,10,5"), Some(("10".into(), "5".into())));
    assert_eq!(natural_size("0 0 0 5"), None);
    assert_eq!(natural_size("0 0 10"), None);
    assert_eq!(natural_size("a b c d"), None);
    assert_eq!(natural_size("0 0 inf 5"), None);
    assert_eq!(
        cleaned("<svg aria-label=\"Mine\" role=\"graphics-document\"/>"),
        "<svg aria-label=\"Mine\" role=\"graphics-document\"/>"
    );
}

#[test]
fn diagram_names_read_as_words() {
    assert_eq!(diagram_name(""), "Diagram");
    assert_eq!(diagram_name("flowchart-v2"), "Flowchart diagram");
    assert_eq!(diagram_name("sequence"), "Sequence diagram");
    assert_eq!(diagram_name("class"), "Class diagram");
    assert_eq!(diagram_name("stateDiagram"), "State diagram");
    assert_eq!(diagram_name("state"), "State diagram");
    assert_eq!(diagram_name("gantt"), "Gantt diagram");
    assert_eq!(diagram_name("pie chart"), "Pie diagram");
    assert_eq!(diagram_name("--"), "Diagram");
}

#[test]
fn style_declarations_split_and_drop_important() {
    assert_eq!(
        declarations("fill: red !important; ; Stroke :blue;nothing; x:"),
        vec![
            ("fill".to_owned(), "red".to_owned()),
            ("stroke".to_owned(), "blue".to_owned())
        ]
    );
}

#[test]
fn class_names_keep_safe_characters_only() {
    assert_eq!(class_name("node-1_a"), Some("node-1_a".to_owned()));
    assert_eq!(class_name("a.b"), None);
    assert_eq!(class_name(""), None);
}

#[test]
fn values_keep_only_inner_references() {
    let cleaner = Cleaner {
        prefix: "dm1",
        out: String::new(),
        open: Vec::new(),
        skipping: 0,
        root_seen: true,
    };
    assert_eq!(
        cleaner.value("url(#a) url( '#dm1-b' )"),
        Some("url(#dm1-a) url(#dm1-b)".to_owned())
    );
    assert_eq!(cleaner.value("URL(#x)"), Some("url(#dm1-x)".to_owned()));
    assert_eq!(cleaner.value("url(x.svg#a)"), None);
    assert_eq!(cleaner.value("url(#a"), None);
    assert_eq!(cleaner.value("red; x"), None);
    assert_eq!(cleaner.value("a\\62 c"), None);
    assert_eq!(cleaner.value("expression(1)"), None);
    assert_eq!(cleaner.value("javascript:1"), None);
    assert_eq!(cleaner.value("10 20"), Some("10 20".to_owned()));
    assert_eq!(cleaner.id("dm1-x"), "dm1-x");
    assert_eq!(cleaner.id("a b\"c"), "dm1-a_b_c");
    assert_eq!(
        cleaner.address("mailto:a@b.c", true),
        Some("mailto:a@b.c".to_owned())
    );
    assert_eq!(cleaner.address("mailto:a@b.c", false), None);
    assert_eq!(cleaner.address("https://a.example/x y", true), None);
    assert_eq!(cleaner.address("ftp://a.example", true), None);
    assert_eq!(cleaner.address("noscheme", true), None);
}

#[test]
fn style_becomes_attributes_or_label_classes() {
    assert_eq!(
        cleaned(
            "<svg><rect style=\"fill:#f00;cursor:pointer;stroke-width: 2px\" fill=\"blue\"/></svg>"
        ),
        "<svg aria-label=\"Diagram\" role=\"img\"><rect fill=\"#f00\" stroke-width=\"2px\"/></svg>"
    );
    assert_eq!(
        cleaned(
            "<svg><foreignObject><div class=\"x x\" style=\"white-space: normal; text-align: right; color: red\" id=\"d\"><br/><span/></div></foreignObject></svg>"
        ),
        "<svg aria-label=\"Diagram\" role=\"img\"><foreignObject><div class=\"x dm-wrap dm-end\"><br><span></span></div></foreignObject></svg>"
    );
    assert_eq!(
        cleaned("<svg><foreignObject><div><br></br></div></foreignObject></svg>"),
        "<svg aria-label=\"Diagram\" role=\"img\"><foreignObject><div><br></div></foreignObject></svg>"
    );
}

#[test]
fn errors_read_as_sentences() {
    let error = clean("<g/>", "dm1").unwrap_err();
    assert_eq!(error.to_string(), "the drawing's root is <g>, not <svg>");
    let attribute = clean("<svg a=\"1\" a=\"2\"/>", "dm1").unwrap_err();
    assert!(
        attribute
            .to_string()
            .starts_with("an attribute is not well-formed"),
        "{attribute}"
    );
    let value = clean("<svg aria-label=\"&bogus;\"/>", "dm1").unwrap_err();
    assert!(
        value.to_string().starts_with("an attribute value is wrong"),
        "{value}"
    );
}
