//! Which of Prism's files an exported file carries.
//!
//! The server's page script loads Prism's core and the languages a page uses
//! on demand. An exported file can load nothing, so the program picks the
//! files when it writes it: the languages its reading pages use, with the
//! languages each builds on, in an order where every file comes after what
//! it needs. The two tables below are the page script's (`assets/page.js`,
//! `PRISM_NEEDS` and `PRISM_ALIASES`); a test keeps them equal.

use std::collections::BTreeMap;

/// Prism's language files that need another one first.
pub(crate) const NEEDS: &[(&str, &[&str])] = &[
    ("c", &["clike"]),
    ("cpp", &["c"]),
    ("csharp", &["clike"]),
    ("dart", &["clike"]),
    ("go", &["clike"]),
    ("gradle", &["clike"]),
    ("groovy", &["clike"]),
    ("java", &["clike"]),
    ("javascript", &["clike"]),
    ("json5", &["json"]),
    ("jsx", &["markup", "javascript"]),
    ("kotlin", &["clike"]),
    ("markdown", &["markup"]),
    ("protobuf", &["clike"]),
    ("ruby", &["clike"]),
    ("scala", &["java"]),
    ("shell-session", &["bash"]),
    ("solidity", &["clike"]),
    ("typescript", &["javascript"]),
    ("tsx", &["jsx", "typescript"]),
    ("xml-doc", &["markup"]),
];

/// Other names for a language, as code blocks write them.
pub(crate) const ALIASES: &[(&str, &str)] = &[
    ("sh", "bash"),
    ("shell", "bash"),
    ("zsh", "bash"),
    ("env", "bash"),
    ("dotenv", "bash"),
    ("cs", "csharp"),
    ("c#", "csharp"),
    ("dotnet", "csharp"),
    ("c++", "cpp"),
    ("cc", "cpp"),
    ("h", "c"),
    ("hpp", "cpp"),
    ("dockerfile", "docker"),
    ("hs", "haskell"),
    ("js", "javascript"),
    ("mjs", "javascript"),
    ("cjs", "javascript"),
    ("jsonc", "json"),
    ("webmanifest", "json"),
    ("html", "markup"),
    ("xml", "markup"),
    ("svg", "markup"),
    ("kt", "kotlin"),
    ("kts", "kotlin"),
    ("tex", "latex"),
    ("md", "markdown"),
    ("py", "python"),
    ("rb", "ruby"),
    ("rs", "rust"),
    ("golang", "go"),
    ("ts", "typescript"),
    ("yml", "yaml"),
    ("tf", "hcl"),
    ("terraform", "hcl"),
    ("alloy", "hcl"),
    ("ps1", "powershell"),
    ("pwsh", "powershell"),
    ("proto", "protobuf"),
    ("gql", "graphql"),
    ("sh-session", "shell-session"),
    ("shellsession", "shell-session"),
    ("sol", "solidity"),
];

/// The Prism name of a code block's language, in lower case as the page
/// script reads it, when Prism knows it: `has(name)` says whether the
/// program carries `prism-<name>.js`.
pub(crate) fn name(language: &str, has: impl Fn(&str) -> bool) -> Option<String> {
    let lower = language.to_ascii_lowercase();
    let name = ALIASES
        .iter()
        .find(|(alias, _)| *alias == lower)
        .map_or(lower.as_str(), |(_, name)| *name);
    has(name).then(|| name.to_owned())
}

/// The files to carry for these languages, as Prism names, each after the
/// languages it needs. `prism-core.js` comes first and is not listed.
pub(crate) fn files(languages: &[&str]) -> Vec<String> {
    let needs: BTreeMap<&str, &[&str]> = NEEDS.iter().copied().collect();
    let mut order: Vec<String> = Vec::new();
    let mut sorted = languages.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    for language in sorted {
        with_needs(language, &needs, &mut order);
    }
    order
}

fn with_needs(name: &str, needs: &BTreeMap<&str, &[&str]>, out: &mut Vec<String>) {
    for need in needs.get(name).copied().unwrap_or_default() {
        with_needs(need, needs, out);
    }
    if !out.iter().any(|done| done == name) {
        out.push(name.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text between `const <name> = {` and the next `};` in the page
    /// script.
    fn script_table(name: &str) -> String {
        let script = include_str!("../../assets/page.js");
        let start = script
            .find(&format!("const {name} = {{"))
            .unwrap_or_else(|| panic!("page.js has no {name}"));
        let rest = &script[start..];
        let end = rest.find("};").expect("the table ends");
        rest[rest.find('{').unwrap() + 1..end].to_owned()
    }

    #[test]
    fn the_tables_are_the_page_scripts() {
        let mut needs = Vec::new();
        for entry in script_table("PRISM_NEEDS").split("],") {
            let Some((key, list)) = entry.split_once(':') else {
                continue;
            };
            let key = key.trim().trim_matches('"').to_owned();
            let list: Vec<String> = list
                .trim()
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split(',')
                .map(|item| item.trim().trim_matches('"').to_owned())
                .filter(|item| !item.is_empty())
                .collect();
            needs.push((key, list));
        }
        let ours: Vec<(String, Vec<String>)> = NEEDS
            .iter()
            .map(|(key, list)| {
                (
                    (*key).to_owned(),
                    list.iter().map(|s| (*s).to_owned()).collect(),
                )
            })
            .collect();
        assert_eq!(needs, ours);
        let mut aliases = Vec::new();
        for entry in script_table("PRISM_ALIASES").split(',') {
            let Some((key, value)) = entry.split_once(':') else {
                continue;
            };
            aliases.push((
                key.trim().trim_matches('"').to_owned(),
                value.trim().trim_matches('"').to_owned(),
            ));
        }
        let ours: Vec<(String, String)> = ALIASES
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        assert_eq!(aliases, ours);
    }

    #[test]
    fn names_follow_the_aliases_and_what_is_carried() {
        let has = |name: &str| ["bash", "rust", "typescript", "toml"].contains(&name);
        let named = |language: &str| name(language, has);
        assert_eq!(named("sh").as_deref(), Some("bash"));
        assert_eq!(named("SH").as_deref(), Some("bash"));
        assert_eq!(named("rust").as_deref(), Some("rust"));
        assert_eq!(named("Rust").as_deref(), Some("rust"));
        assert_eq!(named("ts").as_deref(), Some("typescript"));
        assert_eq!(named("text"), None);
        assert_eq!(named("py"), None);
    }

    #[test]
    fn every_file_comes_after_what_it_needs() {
        assert_eq!(
            files(&["tsx"]),
            ["markup", "clike", "javascript", "jsx", "typescript", "tsx"]
        );
        assert_eq!(
            files(&["scala", "go", "go"]),
            ["clike", "go", "java", "scala"]
        );
        assert_eq!(files(&[]), Vec::<String>::new());
    }
}
