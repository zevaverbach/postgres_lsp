use std::sync::RwLock;

use base_db::{Change, StatementChange, StatementRef};
use dashmap::DashMap;
use tree_sitter::{InputEdit, Tree};

pub struct TreeSitterParser {
    db: DashMap<StatementRef, Tree>,

    parser: RwLock<tree_sitter::Parser>,
}

impl TreeSitterParser {
    pub fn new() -> TreeSitterParser {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(tree_sitter_sql::language())
            .expect("Error loading sql language");

        TreeSitterParser {
            db: DashMap::new(),
            parser: RwLock::new(parser),
        }
    }

    pub fn process_changes(&self, changes: &Vec<StatementChange>) {
        for c in changes {
            if c.change.is_none() {
                // statement was removed
                self.db.remove(&c.statement);
                continue;
            }

            if c.change.as_ref().unwrap().range.is_none() {
                // statement was added
                let mut guard = self.parser.write().expect("Error reading parser");
                // todo handle error
                let tree = guard.parse(&c.statement.text, None).unwrap();
                drop(guard);
                self.db.insert(c.statement.clone(), tree);
                continue;
            }

            // statement was changed
            self.db.entry(c.statement.clone()).and_modify(|tree| {
                let edit = edit_from_change(
                    &c.statement.text.as_str(),
                    usize::from(c.change.as_ref().unwrap().range.unwrap().start()),
                    usize::from(c.change.as_ref().unwrap().range.unwrap().end()),
                    c.change.as_ref().unwrap().text.as_str(),
                );

                tree.edit(&edit);

                let new_text = apply_text_change(&c.statement.text, c.change.as_ref().unwrap());

                let mut guard = self.parser.write().expect("Error reading parser");
                // todo handle error
                *tree = guard.parse(new_text, Some(tree)).unwrap();
                drop(guard);
            });
        }
    }
}

// i wont pretend to know whats going on here but it seems to work
pub fn edit_from_change(
    text: &str,
    start_char: usize,
    end_char: usize,
    replacement_text: &str,
) -> InputEdit {
    let mut start_byte = 0;
    let mut end_byte = 0;
    let mut chars_counted = 0;

    let mut line = 0;
    let mut current_line_char_start = 0; // Track start of the current line in characters
    let mut column_start = 0;
    let mut column_end = 0;

    for (idx, c) in text.char_indices() {
        if chars_counted == start_char {
            start_byte = idx;
            column_start = chars_counted - current_line_char_start;
        }
        if chars_counted == end_char {
            end_byte = idx;
            // Calculate column_end based on replacement_text
            let replacement_lines: Vec<&str> = replacement_text.split('\n').collect();
            if replacement_lines.len() > 1 {
                // If replacement text spans multiple lines, adjust line and column_end accordingly
                line += replacement_lines.len() - 1;
                column_end = replacement_lines.last().unwrap().chars().count();
            } else {
                // Single line replacement, adjust column_end based on replacement text length
                column_end = column_start + replacement_text.chars().count();
            }
            break; // Found both start and end
        }
        if c == '\n' {
            line += 1;
            current_line_char_start = chars_counted + 1; // Next character starts a new line
        }
        chars_counted += 1;
    }

    // Adjust end_byte based on the byte length of the replacement text
    if start_byte != end_byte {
        // Ensure there's a range to replace
        end_byte = start_byte + replacement_text.len();
    } else if chars_counted < text.chars().count() && end_char == chars_counted {
        // For insertions at the end of text
        end_byte += replacement_text.len();
    }

    let start_point = tree_sitter::Point::new(line, column_start);
    let end_point = tree_sitter::Point::new(line, column_end);

    // Calculate the new end byte after the insertion
    let new_end_byte = start_byte + replacement_text.len();

    // Calculate the new end position
    let new_lines = replacement_text.matches('\n').count(); // Count how many new lines are in the inserted text
    let last_line_length = replacement_text
        .lines()
        .last()
        .unwrap_or("")
        .chars()
        .count(); // Length of the last line in the insertion

    let new_end_position = if new_lines > 0 {
        // If there are new lines, the row is offset by the number of new lines, and the column is the length of the last line
        tree_sitter::Point::new(start_point.row + new_lines, last_line_length)
    } else {
        // If there are no new lines, the row remains the same, and the column is offset by the length of the insertion
        tree_sitter::Point::new(start_point.row, start_point.column + last_line_length)
    };

    InputEdit {
        start_byte,
        old_end_byte: end_byte,
        new_end_byte,
        start_position: start_point,
        old_end_position: end_point,
        new_end_position,
    }
}

pub fn apply_text_change(text: &String, change: &Change) -> String {
    if change.range.is_none() {
        return change.text.clone();
    }

    let range = change.range.unwrap();
    let start = usize::from(range.start());
    let end = usize::from(range.end());

    let mut new_text = String::new();
    new_text.push_str(&text[..start]);
    new_text.push_str(&change.text);
    new_text.push_str(&text[end..]);

    new_text
}
