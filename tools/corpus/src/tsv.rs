// Tab-separated tables with a header row. Tabs, line breaks and backslashes
// inside a value are escaped, so every row stays on one line.

use std::path::Path;

pub fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

pub fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Table {
    pub header: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new(header: &[&str]) -> Table {
        Table {
            header: header.iter().map(|h| h.to_string()).collect(),
            rows: Vec::new(),
        }
    }

    pub fn push(&mut self, row: Vec<String>) {
        debug_assert_eq!(row.len(), self.header.len());
        self.rows.push(row);
    }

    pub fn column(&self, name: &str) -> Option<usize> {
        self.header.iter().position(|h| h == name)
    }

    // The value of `name` in `row`, or "" when the column or the cell is absent.
    pub fn get<'a>(&self, row: &'a [String], name: &str) -> &'a str {
        self.column(name)
            .and_then(|i| row.get(i))
            .map(String::as_str)
            .unwrap_or("")
    }

    pub fn to_text(&self) -> String {
        let mut text = String::new();
        for line in std::iter::once(&self.header).chain(self.rows.iter()) {
            let cells: Vec<String> = line.iter().map(|cell| escape(cell)).collect();
            text.push_str(&cells.join("\t"));
            text.push('\n');
        }
        text
    }

    pub fn parse(text: &str) -> Table {
        let mut lines = text.lines().filter(|line| !line.is_empty());
        let split = |line: &str| line.split('\t').map(unescape).collect::<Vec<String>>();
        let header = lines.next().map(split).unwrap_or_default();
        let rows = lines.map(split).collect();
        Table { header, rows }
    }

    pub fn write(&self, path: &Path) -> Result<(), String> {
        std::fs::write(path, self.to_text())
            .map_err(|e| format!("cannot write {}: {}", path.display(), e))
    }

    pub fn read(path: &Path) -> Result<Table, String> {
        std::fs::read_to_string(path)
            .map(|text| Table::parse(&text))
            .map_err(|e| format!("cannot read {}: {}", path.display(), e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_with_separators_round_trip() {
        let mut table = Table::new(&["doc", "message"]);
        table.push(vec!["a\tb.docx".into(), "line 1\nline 2\r\nC:\\path\\x".into()]);
        table.push(vec!["".into(), "\\t is not a tab".into()]);
        let text = table.to_text();
        assert_eq!(text.lines().count(), 3, "{}", text);
        assert_eq!(Table::parse(&text), table);
    }

    #[test]
    fn missing_cells_and_columns_read_as_empty() {
        let table = Table::parse("a\tb\n1\n");
        assert_eq!(table.get(&table.rows[0], "a"), "1");
        assert_eq!(table.get(&table.rows[0], "b"), "");
        assert_eq!(table.get(&table.rows[0], "zzz"), "");
    }
}
