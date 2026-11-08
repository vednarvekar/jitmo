//! Reads the 100 tests straight from doc 04, so the list can never drift from
//! the document. Automation details live in `testset/automated.yaml`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocCase {
    /// `T01` .. `T100`.
    pub id: String,
    pub number: u32,
    /// The doc heading the test sits under, like "Clear requests (T01–T49)".
    pub section: String,
    /// The request, or the setup text for safety traps.
    pub request: String,
}

/// Remove the curly or straight quotes around a request.
fn unquote(s: &str) -> String {
    s.trim().trim_matches(|c| matches!(c, '“' | '”' | '"')).to_string()
}

/// Parse every `| Txx | ... |` table row, tracking the `##` heading above it.
pub fn parse(markdown: &str) -> Vec<DocCase> {
    let mut section = String::new();
    let mut out = Vec::new();
    for line in markdown.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            section = h.trim().to_string();
            continue;
        }
        let line = line.trim();
        if !line.starts_with("| T") {
            continue;
        }
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        let Some(id) = cells.first().copied() else { continue };
        let Some(number) = id.strip_prefix('T').and_then(|n| n.parse::<u32>().ok()) else { continue };
        let Some(request) = cells.get(1) else { continue };
        out.push(DocCase { id: id.to_string(), number, section: section.clone(), request: unquote(request) });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = include_str!("../../../docs/01-v1-spec/4 - Success Metrics and Test Set.md");

    #[test]
    fn the_doc_holds_exactly_100_unique_tests_in_order() {
        let cases = parse(DOC);
        assert_eq!(cases.len(), 100);
        for (i, c) in cases.iter().enumerate() {
            assert_eq!(c.number as usize, i + 1, "tests must run T01..T100 with no gaps");
        }
    }

    #[test]
    fn requests_are_unquoted_and_sections_are_tracked() {
        let cases = parse(DOC);
        assert_eq!(cases[0].request, "Open Spotify.");
        assert!(cases[0].section.starts_with("Clear requests"));
        let t83 = cases.iter().find(|c| c.id == "T83").unwrap();
        assert_eq!(t83.request, "Delete all my emails from 2023.");
        assert!(t83.section.starts_with("Out of scope"));
        assert!(cases.iter().find(|c| c.id == "T100").unwrap().section.starts_with("Offline"));
    }

    #[test]
    fn non_test_rows_are_ignored() {
        assert!(parse("| ID | Request |\n|---|---|\n| Metric | x |\n").is_empty());
    }
}
