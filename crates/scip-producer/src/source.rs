use scip::types::PositionEncoding;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Span {
    pub start: Pos,
    pub end: Pos,
}

impl Span {
    /// SCIP ranges are `[line, start_col, end_col]` or
    /// `[start_line, start_col, end_line, end_col]`, end exclusive.
    pub fn from_scip(range: &[i32]) -> Option<Span> {
        let n = |i: usize| usize::try_from(range[i]).ok();
        match range.len() {
            3 => Some(Span {
                start: Pos {
                    line: n(0)?,
                    col: n(1)?,
                },
                end: Pos {
                    line: n(0)?,
                    col: n(2)?,
                },
            }),
            4 => Some(Span {
                start: Pos {
                    line: n(0)?,
                    col: n(1)?,
                },
                end: Pos {
                    line: n(2)?,
                    col: n(3)?,
                },
            }),
            _ => None,
        }
    }

    pub fn contains(&self, other: &Span) -> bool {
        self.start <= other.start && other.end <= self.end
    }

    pub fn contains_pos(&self, pos: Pos) -> bool {
        self.start <= pos && pos < self.end
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColumnUnit {
    Utf8,
    Utf16,
    Utf32,
}

impl ColumnUnit {
    /// Indexers written before `position_encoding` existed leave it
    /// unspecified; scip-typescript is the notable one and it counts UTF-16
    /// code units (JavaScript strings), everything else we know of counts bytes.
    pub fn resolve(encoding: PositionEncoding, tool_name: &str) -> ColumnUnit {
        match encoding {
            PositionEncoding::UTF8CodeUnitOffsetFromLineStart => ColumnUnit::Utf8,
            PositionEncoding::UTF16CodeUnitOffsetFromLineStart => ColumnUnit::Utf16,
            PositionEncoding::UTF32CodeUnitOffsetFromLineStart => ColumnUnit::Utf32,
            PositionEncoding::UnspecifiedPositionEncoding if tool_name == "scip-typescript" => {
                ColumnUnit::Utf16
            }
            PositionEncoding::UnspecifiedPositionEncoding => ColumnUnit::Utf8,
        }
    }
}

pub struct SourceFile {
    text: String,
    line_starts: Vec<usize>,
    unit: ColumnUnit,
    import_lines: Vec<bool>,
}

impl SourceFile {
    pub fn new(text: String, unit: ColumnUnit) -> SourceFile {
        let mut line_starts = vec![0];
        line_starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        let import_lines = mark_import_lines(&text);
        SourceFile {
            text,
            line_starts,
            unit,
            import_lines,
        }
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn line_count(&self) -> usize {
        self.text.lines().count()
    }

    pub fn is_import_line(&self, line: usize) -> bool {
        self.import_lines.get(line).copied().unwrap_or(false)
    }

    pub fn byte_offset(&self, pos: Pos) -> usize {
        let Some(&start) = self.line_starts.get(pos.line) else {
            return self.text.len();
        };
        let end = self
            .line_starts
            .get(pos.line + 1)
            .copied()
            .unwrap_or(self.text.len());
        let line = &self.text[start..end];
        let within = match self.unit {
            ColumnUnit::Utf8 => pos.col.min(line.len()),
            ColumnUnit::Utf16 => advance(line, pos.col, char::len_utf16),
            ColumnUnit::Utf32 => advance(line, pos.col, |_| 1),
        };
        start + within
    }
}

fn advance(line: &str, units: usize, width: impl Fn(char) -> usize) -> usize {
    let mut seen = 0;
    for (byte, ch) in line.char_indices() {
        if seen >= units {
            return byte;
        }
        seen += width(ch);
    }
    line.len()
}

/// No indexer we have met sets the `Import` symbol role, so import edges are
/// recognised textually: `use`/`import`/`from` statements, plus the lines of
/// a Go `import ( ... )` block.
fn mark_import_lines(text: &str) -> Vec<bool> {
    let mut in_go_block = false;
    text.lines()
        .map(|raw| {
            let line = strip_visibility(raw.trim_start());
            if in_go_block {
                if line.starts_with(')') {
                    in_go_block = false;
                }
                return true;
            }
            if line.starts_with("import (") {
                in_go_block = true;
                return true;
            }
            ["use", "import", "from"]
                .iter()
                .any(|kw| starts_with_word(line, kw))
        })
        .collect()
}

fn strip_visibility(line: &str) -> &str {
    for prefix in ["pub", "export"] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let rest = match rest.strip_prefix('(') {
                Some(after) => after.split_once(')').map(|(_, r)| r).unwrap_or(""),
                None => rest,
            };
            if rest.starts_with(char::is_whitespace) {
                return rest.trim_start();
            }
        }
    }
    line
}

fn starts_with_word(line: &str, word: &str) -> bool {
    line.strip_prefix(word)
        .is_some_and(|rest| rest.starts_with(char::is_whitespace))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_columns_land_after_multibyte_chars() {
        // "🚀" is 4 bytes, 2 UTF-16 units, 1 char.
        let text = "a🚀b\nc".to_string();
        let utf16 = SourceFile::new(text.clone(), ColumnUnit::Utf16);
        assert_eq!(utf16.byte_offset(Pos { line: 0, col: 3 }), 5);
        assert_eq!(utf16.byte_offset(Pos { line: 1, col: 0 }), 7);
        let utf32 = SourceFile::new(text.clone(), ColumnUnit::Utf32);
        assert_eq!(utf32.byte_offset(Pos { line: 0, col: 2 }), 5);
        let utf8 = SourceFile::new(text, ColumnUnit::Utf8);
        assert_eq!(utf8.byte_offset(Pos { line: 0, col: 5 }), 5);
        assert_eq!(utf8.byte_offset(Pos { line: 9, col: 0 }), 8);
    }

    #[test]
    fn import_lines_cover_rust_ts_and_go_forms() {
        let text = "pub(crate) use a::b;\nimport { x } from \"./y\";\nimport (\n\t\"fmt\"\n)\nfn used() {}\nlet from_here = 1;\n";
        let src = SourceFile::new(text.to_string(), ColumnUnit::Utf8);
        let got: Vec<bool> = (0..7).map(|l| src.is_import_line(l)).collect();
        assert_eq!(got, [true, true, true, true, true, false, false]);
    }
}
