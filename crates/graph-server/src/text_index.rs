//! Substring search over the graph's files: their path, their name, and their
//! content lines. One [`TextIndex`] is built at load from every `File` entity
//! the caller can produce a path and text for; `/search` (`handlers.rs`)
//! queries it. See `ui/CONTRACT.md` for the wire shape this feeds.
//!
//! Three kinds share one mechanism: a doc list plus a trigram postings map
//! (`Postings`), one per kind. A query either names a kind with a
//! `file:`/`path:`/`content:` prefix or searches all three.

use std::collections::HashMap;

use entity_graph::{EntityGraph, EntityId, EntityKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Path,
    Content,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::File => "file",
            Kind::Path => "path",
            Kind::Content => "content",
        }
    }
}

/// One match. `path` is the file's wire path regardless of which kind
/// matched (so a content hit can show its file alongside the line); `text`
/// is the thing the needle was found in (name / path / line) and `start`,
/// `end` locate the match within it, as UTF-16 code units.
pub struct Hit {
    pub kind: Kind,
    pub file: EntityId,
    pub path: String,
    pub line: Option<usize>,
    pub text: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Copy)]
pub struct Limits {
    pub file: usize,
    pub path: usize,
    pub content: usize,
}

#[derive(Default)]
pub struct More {
    pub file: usize,
    pub path: usize,
    pub content: usize,
}

pub struct SearchResult {
    pub kind: Option<Kind>,
    pub needle: String,
    pub hits: Vec<Hit>,
    pub more: More,
}

struct FileDoc {
    id: EntityId,
    path: String,
    name: String,
}

/// One content-postings document: a single line of a single file.
struct LineDoc {
    file_idx: u32,
    line: u32,
    text: String,
}

/// A doc list plus the trigram -> doc-index map used to shortlist candidates
/// for needles of 3+ chars. `texts` are case-folded so search is
/// case-insensitive without touching every candidate at query time.
struct Postings {
    texts: Vec<String>,
    grams: HashMap<[char; 3], Vec<u32>>,
}

impl Postings {
    fn build(texts: Vec<String>) -> Self {
        let mut grams: HashMap<[char; 3], Vec<u32>> = HashMap::new();
        for (i, text) in texts.iter().enumerate() {
            let i = i as u32;
            let chars: Vec<char> = text.chars().collect();
            if chars.len() < 3 {
                continue;
            }
            for w in chars.windows(3) {
                let list = grams.entry([w[0], w[1], w[2]]).or_default();
                // Docs are inserted in ascending order, and a doc can repeat
                // a trigram many times (e.g. "aaaa"); only the first
                // occurrence per doc should land in the list.
                if list.last() != Some(&i) {
                    list.push(i);
                }
            }
        }
        Postings { texts, grams }
    }

    /// `(doc, start_char)` of the first occurrence in each matching doc,
    /// ascending by doc. `needle_folded` must already be case-folded.
    fn find(&self, needle_folded: &str) -> Vec<(u32, usize)> {
        let needle_chars: Vec<char> = needle_folded.chars().collect();
        if needle_chars.len() < 3 {
            return self
                .texts
                .iter()
                .enumerate()
                .filter_map(|(i, t)| t.find(needle_folded).map(|b| (i as u32, char_count(t, b))))
                .collect();
        }

        let mut trigrams: Vec<[char; 3]> =
            needle_chars.windows(3).map(|w| [w[0], w[1], w[2]]).collect();
        trigrams.dedup();
        let mut lists: Vec<&Vec<u32>> = Vec::with_capacity(trigrams.len());
        for g in &trigrams {
            match self.grams.get(g) {
                Some(l) => lists.push(l),
                // A trigram of the needle appears in no doc at all, so the
                // needle cannot occur anywhere either.
                None => return Vec::new(),
            }
        }
        lists.sort_by_key(|l| l.len());
        let mut candidates = lists[0].clone();
        for l in &lists[1..] {
            if candidates.is_empty() {
                break;
            }
            candidates = intersect_sorted(&candidates, l);
        }

        // Trigram co-occurrence only means the needle's 3-grams are all
        // present somewhere in the doc, not that they line up in order; a
        // real substring search on each candidate is what actually decides.
        candidates
            .into_iter()
            .filter_map(|doc| {
                let t = &self.texts[doc as usize];
                t.find(needle_folded).map(|b| (doc, char_count(t, b)))
            })
            .collect()
    }
}

fn intersect_sorted(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                out.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    out
}

fn char_count(text: &str, byte_offset: usize) -> usize {
    text[..byte_offset].chars().count()
}

// `char::to_lowercase` can yield more than one char for a handful of special
// casings; taking only the first keeps folding 1:1 on char count, so a char
// offset into the folded text is always the same offset into the original
// (byte offsets do not carry over the same way — `İ` folds to a shorter `i`).
fn fold(s: &str) -> String {
    s.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect()
}

/// The wire format slices JS strings, which count UTF-16 code units, not
/// chars; `text` is the original (unfolded) string the offset is measured
/// against.
fn utf16_span(text: &str, start_chars: usize, len_chars: usize) -> (usize, usize) {
    let start = text.chars().take(start_chars).map(char::len_utf16).sum();
    let end = text.chars().take(start_chars + len_chars).map(char::len_utf16).sum();
    (start, end)
}

fn split_lines(text: &str) -> impl Iterator<Item = &str> {
    let mut lines: Vec<&str> = text.split('\n').collect();
    // A trailing newline produces one empty trailing piece that is not a line.
    if lines.last() == Some(&"") {
        lines.pop();
    }
    lines.into_iter().map(|l| l.strip_suffix('\r').unwrap_or(l))
}

fn parse_query(q: &str) -> (Option<Kind>, &str) {
    for (prefix, kind) in [("file:", Kind::File), ("path:", Kind::Path), ("content:", Kind::Content)]
    {
        if let Some(rest) = strip_prefix_ci(q, prefix) {
            return (Some(kind), rest.trim_start());
        }
    }
    (None, q)
}

// The three prefixes are ASCII, so a byte-wise ASCII-case-insensitive
// compare is safe: it only slices at a boundary it just confirmed is ASCII.
fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let bytes = prefix.len();
    if s.len() >= bytes && s.as_bytes()[..bytes].eq_ignore_ascii_case(prefix.as_bytes()) {
        Some(&s[bytes..])
    } else {
        None
    }
}

fn truncate<T>(mut items: Vec<T>, limit: usize) -> (Vec<T>, usize) {
    if items.len() > limit {
        let more = items.len() - limit;
        items.truncate(limit);
        (items, more)
    } else {
        (items, 0)
    }
}

pub struct TextIndex {
    files: Vec<FileDoc>,
    file_postings: Postings,
    path_postings: Postings,
    content_postings: Postings,
    lines: Vec<LineDoc>,
}

impl TextIndex {
    /// One document set per kind over every File entity of `graph`. `read`
    /// returns the file's wire path and text, or `None` to leave it out
    /// (unreadable, too big, not UTF-8).
    pub fn build(graph: &EntityGraph, mut read: impl FnMut(EntityId) -> Option<(String, String)>) -> Self {
        let mut files = Vec::new();
        let mut file_names_folded = Vec::new();
        let mut paths_folded = Vec::new();
        let mut content_folded = Vec::new();
        let mut lines = Vec::new();

        for entity in &graph.entities {
            if entity.kind != EntityKind::File {
                continue;
            }
            let Some((mut path, text)) = read(entity.id) else { continue };
            // A single-file load has an empty relative path; the name is the
            // only thing left to match a `path:` query against.
            if path.is_empty() {
                path = entity.name.clone();
            }

            let file_idx = files.len() as u32;
            file_names_folded.push(fold(&entity.name));
            paths_folded.push(fold(&path));
            for (line_no, line) in split_lines(&text).enumerate() {
                content_folded.push(fold(line));
                lines.push(LineDoc { file_idx, line: line_no as u32, text: line.to_string() });
            }
            files.push(FileDoc { id: entity.id, path, name: entity.name.clone() });
        }

        TextIndex {
            files,
            file_postings: Postings::build(file_names_folded),
            path_postings: Postings::build(paths_folded),
            content_postings: Postings::build(content_folded),
            lines,
        }
    }

    pub fn search(&self, query: &str, limits: Limits) -> SearchResult {
        let query = query.trim();
        let (kind, needle) = parse_query(query);
        if needle.is_empty() {
            return SearchResult {
                kind,
                needle: needle.to_string(),
                hits: Vec::new(),
                more: More::default(),
            };
        }

        let folded = fold(needle);
        let needle_len = needle.chars().count();
        let want = |k: Kind| kind.is_none() || kind == Some(k);

        let mut file_matches =
            if want(Kind::File) { self.file_postings.find(&folded) } else { Vec::new() };
        let mut path_matches =
            if want(Kind::Path) { self.path_postings.find(&folded) } else { Vec::new() };
        let mut content_matches =
            if want(Kind::Content) { self.content_postings.find(&folded) } else { Vec::new() };

        file_matches.sort_by(|a, b| {
            let (fa, fb) = (&self.files[a.0 as usize], &self.files[b.0 as usize]);
            fa.name.chars().count().cmp(&fb.name.chars().count()).then_with(|| fa.path.cmp(&fb.path))
        });
        path_matches.sort_by(|a, b| {
            let (fa, fb) = (&self.files[a.0 as usize], &self.files[b.0 as usize]);
            fa.path.chars().count().cmp(&fb.path.chars().count()).then_with(|| fa.path.cmp(&fb.path))
        });
        content_matches.sort_by(|a, b| {
            let (la, lb) = (&self.lines[a.0 as usize], &self.lines[b.0 as usize]);
            let (fa, fb) = (&self.files[la.file_idx as usize], &self.files[lb.file_idx as usize]);
            fa.path.cmp(&fb.path).then_with(|| la.line.cmp(&lb.line))
        });

        if kind.is_none() {
            // A path hit inside the trailing filename segment of the path is
            // the same occurrence the file hit already shows; drop it. A hit
            // in a directory segment (e.g. "main/" in "main/src/main.rs" for
            // "main") is a different occurrence and is kept.
            path_matches.retain(|&(doc, start_chars)| {
                let f = &self.files[doc as usize];
                let name_chars = f.name.chars().count();
                let path_chars = f.path.chars().count();
                start_chars < path_chars.saturating_sub(name_chars)
            });
        }

        let (file_matches, file_more) = truncate(file_matches, limits.file);
        let (path_matches, path_more) = truncate(path_matches, limits.path);
        let (content_matches, content_more) = truncate(content_matches, limits.content);

        let mut hits = Vec::with_capacity(
            file_matches.len() + path_matches.len() + content_matches.len(),
        );
        hits.extend(file_matches.into_iter().map(|(doc, start)| {
            let f = &self.files[doc as usize];
            let (start, end) = utf16_span(&f.name, start, needle_len);
            Hit { kind: Kind::File, file: f.id, path: f.path.clone(), line: None, text: f.name.clone(), start, end }
        }));
        hits.extend(path_matches.into_iter().map(|(doc, start)| {
            let f = &self.files[doc as usize];
            let (start, end) = utf16_span(&f.path, start, needle_len);
            Hit { kind: Kind::Path, file: f.id, path: f.path.clone(), line: None, text: f.path.clone(), start, end }
        }));
        hits.extend(content_matches.into_iter().map(|(doc, start)| {
            let l = &self.lines[doc as usize];
            let f = &self.files[l.file_idx as usize];
            let (start, end) = utf16_span(&l.text, start, needle_len);
            Hit {
                kind: Kind::Content,
                file: f.id,
                path: f.path.clone(),
                line: Some(l.line as usize),
                text: l.text.clone(),
                start,
                end,
            }
        }));

        SearchResult {
            kind,
            needle: needle.to_string(),
            hits,
            more: More { file: file_more, path: path_more, content: content_more },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Postings edge cases below trigram intersection: co-occurrence without
    /// containment is rejected by verification, a sub-3-char needle takes
    /// the linear-scan path, a repeated trigram collapses to one posting,
    /// and folding keeps char offsets 1:1 even when it changes byte length
    /// (case folding) or the text holds an astral char (UTF-16 offsets).
    #[test]
    fn postings_edge_cases() {
        let raw = ["Größe", "abc bcd cde", "aaaa", "xy", "😀 Foo"];
        let folded: Vec<String> = raw.iter().map(|s| fold(s)).collect();
        let p = Postings::build(folded);

        // Every trigram of "abcde" occurs in "abc bcd cde", but the
        // substring itself does not.
        assert!(p.find(&fold("abcde")).is_empty());

        // A 2-char needle bypasses the postings and scans linearly.
        assert_eq!(p.find(&fold("xy")), vec![(3, 0)]);

        // "aaaa" repeats the trigram "aaa" three times; it appears once.
        assert_eq!(p.grams[&['a', 'a', 'a']], vec![2]);

        // "ß" does not fold to "ss", so a needle built from a differently
        // cased original still lands on the same char offset.
        assert_eq!(p.find(&fold("ÖßE")), vec![(0, 2)]);

        // A match after an astral char: "😀" is one char but two UTF-16
        // units, which the char-offset postings do not see and `utf16_span`
        // must account for.
        assert_eq!(p.find(&fold("foo")), vec![(4, 2)]);
        assert_eq!(utf16_span("😀 Foo", 2, 3), (3, 6));
    }
}
