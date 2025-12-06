//! String and text manipulation utilities for fixture analysis.

/// Format a docstring by removing leading/trailing empty lines and dedenting.
pub(crate) fn format_docstring(docstring: String) -> String {
    let lines: Vec<&str> = docstring.lines().collect();

    if lines.is_empty() {
        return String::new();
    }

    // Find first and last non-empty lines
    let mut start = 0;
    let mut end = lines.len();

    while start < lines.len() && lines[start].trim().is_empty() {
        start += 1;
    }

    while end > start && lines[end - 1].trim().is_empty() {
        end -= 1;
    }

    if start >= end {
        return String::new();
    }

    let lines = &lines[start..end];

    // Find minimum indentation (excluding first line if it's not empty)
    let mut min_indent = usize::MAX;
    for (i, line) in lines.iter().enumerate() {
        if i == 0 && !line.trim().is_empty() {
            continue; // First line indentation doesn't count
        }

        if !line.trim().is_empty() {
            let indent = line.len() - line.trim_start().len();
            min_indent = min_indent.min(indent);
        }
    }

    if min_indent == usize::MAX {
        min_indent = 0;
    }

    // Dedent all lines
    let mut result = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if i == 0 {
            result.push(line.trim().to_string());
        } else if line.trim().is_empty() {
            result.push(String::new());
        } else {
            let dedented = if line.len() > min_indent {
                &line[min_indent..]
            } else {
                line.trim_start()
            };
            result.push(dedented.to_string());
        }
    }

    result.join("\n")
}

/// Extract the word at a given character position in a line.
/// Returns None if the position is not within a word.
pub(crate) fn extract_word_at_position(line: &str, character: usize) -> Option<String> {
    let char_indices: Vec<(usize, char)> = line.char_indices().collect();

    if character >= char_indices.len() {
        return None;
    }

    let (_byte_pos, c) = char_indices[character];

    if !c.is_alphanumeric() && c != '_' {
        return None;
    }

    // Find start of word
    let mut start_idx = character;
    while start_idx > 0 {
        let (_, prev_c) = char_indices[start_idx - 1];
        if !prev_c.is_alphanumeric() && prev_c != '_' {
            break;
        }
        start_idx -= 1;
    }

    // Find end of word
    let mut end_idx = character + 1;
    while end_idx < char_indices.len() {
        let (_, curr_c) = char_indices[end_idx];
        if !curr_c.is_alphanumeric() && curr_c != '_' {
            break;
        }
        end_idx += 1;
    }

    let start_byte = char_indices[start_idx].0;
    let end_byte = if end_idx < char_indices.len() {
        char_indices[end_idx].0
    } else {
        line.len()
    };

    Some(line[start_byte..end_byte].to_string())
}

/// Find the character position of a function name in a line of code.
/// Returns (start_char, end_char) positions.
pub(crate) fn find_function_name_position(
    content: &str,
    line: usize,
    func_name: &str,
) -> (usize, usize) {
    if let Some(line_content) = content.lines().nth(line.saturating_sub(1)) {
        // Look for "def function_name" pattern
        if let Some(def_pos) = line_content.find("def ") {
            let after_def = &line_content[def_pos + 4..];
            if let Some(name_pos) = after_def.find(func_name) {
                let start_char = def_pos + 4 + name_pos;
                let end_char = start_char + func_name.len();
                return (start_char, end_char);
            }
        }
        // Fallback: search for function name anywhere in line
        if let Some(pos) = line_content.find(func_name) {
            return (pos, pos + func_name.len());
        }
    }
    // Default fallback
    (0, func_name.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_docstring_simple() {
        let input = "Simple docstring".to_string();
        assert_eq!(format_docstring(input), "Simple docstring");
    }

    #[test]
    fn test_format_docstring_with_whitespace() {
        let input = "\n\n  Line 1\n  Line 2\n\n".to_string();
        assert_eq!(format_docstring(input), "Line 1\nLine 2");
    }

    #[test]
    fn test_format_docstring_indented() {
        let input = "First line\n    Indented line\n    Another indented".to_string();
        let result = format_docstring(input);
        assert_eq!(result, "First line\nIndented line\nAnother indented");
    }

    #[test]
    fn test_extract_word_at_position() {
        let line = "def my_function(arg1, arg2):";
        assert_eq!(extract_word_at_position(line, 0), Some("def".to_string()));
        assert_eq!(
            extract_word_at_position(line, 4),
            Some("my_function".to_string())
        );
        assert_eq!(extract_word_at_position(line, 16), Some("arg1".to_string()));
        assert_eq!(extract_word_at_position(line, 20), None); // comma
    }

    #[test]
    fn test_find_function_name_position() {
        let content = "def my_function():\n    pass";
        let (start, end) = find_function_name_position(content, 1, "my_function");
        assert_eq!(start, 4);
        assert_eq!(end, 15);
    }
}
