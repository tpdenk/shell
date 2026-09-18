//! Adds the keys a config file lacks, copying them from the defaults' YAML.
//!
//! The file text is spliced, not regenerated, so comments, key order, and
//! formatting written by the user survive. Missing keys are appended at the
//! end of the block mapping they belong to, at that mapping's indentation.
//! Flow mappings (`{a: 1}`) are left alone.

use std::ops::Range;

use anyhow::{Context, Result, bail};
use serde_saphyr::granit_parser::{Event, Marker, Parser, ScalarStyle, StructureStyle};

/// The extended text, or `None` when `text` already has every key of
/// `defaults`. An empty `text` becomes `defaults`.
pub(crate) fn extend(text: &str, defaults: &str) -> Result<Option<String>> {
    let Some(wanted) = parse(defaults).context("parsing defaults")? else {
        bail!("defaults do not serialize to a mapping");
    };
    let Some(present) = parse(text)? else {
        return Ok(text.trim().is_empty().then(|| defaults.to_owned()));
    };

    let mut insertions = Vec::new();
    missing(&present, &wanted, defaults, &mut insertions);
    if insertions.is_empty() {
        return Ok(None);
    }
    // Highest offset first keeps lower offsets valid. A nested mapping is
    // recorded before its parent, and both share an offset when the nested
    // mapping holds the last line. Ascending stable sort plus reverse
    // iteration inserts the parent's lines first, so the nested lines end
    // up above them, inside their mapping.
    insertions.sort_by_key(|(offset, _)| *offset);
    let mut out = text.to_owned();
    for (offset, lines) in insertions.iter().rev() {
        out.insert_str(*offset, lines);
    }
    Ok(Some(out))
}

/// A block mapping in the source text.
struct Mapping {
    /// Column of its keys.
    indent: usize,
    /// Byte offset of the end of the last line holding one of its values,
    /// where new entries go.
    end: usize,
    entries: Vec<Entry>,
}

struct Entry {
    key: String,
    /// Bytes of the `key: value` source, up to the next entry.
    span: Range<usize>,
    /// The value when it is a block mapping.
    mapping: Option<Mapping>,
}

/// Records, per mapping in `present`, the entries of `wanted` it lacks. Each
/// insertion is `(byte offset, lines)`, the lines re-indented for `present`
/// and each led by a newline.
fn missing(present: &Mapping, wanted: &Mapping, wanted_text: &str, out: &mut Vec<(usize, String)>) {
    let mut lines = String::new();
    for entry in &wanted.entries {
        match present.entries.iter().find(|e| e.key == entry.key) {
            None => {
                for line in wanted_text[entry.span.clone()].trim_end().lines() {
                    lines.push('\n');
                    let line = strip_indent(line, wanted.indent);
                    if !line.is_empty() {
                        lines.extend(std::iter::repeat_n(' ', present.indent));
                        lines.push_str(line);
                    }
                }
            }
            Some(existing) => {
                if let (Some(present), Some(wanted)) = (&existing.mapping, &entry.mapping) {
                    missing(present, wanted, wanted_text, out);
                }
            }
        }
    }
    if !lines.is_empty() {
        out.push((present.end, lines));
    }
}

/// Removes up to `indent` leading spaces.
fn strip_indent(line: &str, indent: usize) -> &str {
    let spaces = line.bytes().take(indent).take_while(|b| *b == b' ').count();
    &line[spaces..]
}

/// Byte offset of the newline ending the line that contains `offset`, or the
/// text length when that line is the last one. An `offset` just past a
/// newline belongs to the line before it.
fn line_end(text: &str, offset: usize) -> usize {
    let offset = offset.min(text.len());
    let offset = match offset.checked_sub(1) {
        Some(before) if text.as_bytes()[before] == b'\n' => before,
        _ => offset,
    };
    text[offset..].find('\n').map_or(text.len(), |i| offset + i)
}

fn byte(marker: Marker) -> usize {
    marker
        .byte_offset()
        .expect("str input tracks byte offsets")
}

/// An open collection while parsing.
enum Frame {
    Mapping {
        /// Block style with scalar keys only. Anything else cannot be
        /// extended by splicing lines.
        extendable: bool,
        indent: usize,
        entries: Vec<Entry>,
        /// A key was read and its value comes next. Holds the byte offset
        /// past the key.
        key_end: Option<usize>,
    },
    Sequence,
}

/// The document's root when it is a block mapping.
fn parse(text: &str) -> Result<Option<Mapping>> {
    let mut parser = Parser::new_from_str(text);
    let mut stack: Vec<Frame> = Vec::new();
    // Byte offset just past the most recent value, so a mapping closing
    // knows where its last line is.
    let mut last_end = 0;
    let mut root = None;

    while let Some(result) = parser.next_event() {
        let (event, span) = result.context("parsing YAML")?;
        match event {
            Event::MappingStart(style, ..) => {
                begin_value(&mut stack);
                stack.push(Frame::Mapping {
                    extendable: style == StructureStyle::Block,
                    indent: span.start.col(),
                    entries: Vec::new(),
                    key_end: None,
                });
            }
            Event::SequenceStart(..) => {
                begin_value(&mut stack);
                stack.push(Frame::Sequence);
            }
            Event::Scalar(value, style, ..) => match stack.last_mut() {
                Some(Frame::Mapping {
                    key_end: key_end @ None,
                    entries,
                    ..
                }) => {
                    let start = byte(span.start);
                    if let Some(previous) = entries.last_mut() {
                        previous.span.end = start;
                    }
                    entries.push(Entry {
                        key: value.into_owned(),
                        span: start..start,
                        mapping: None,
                    });
                    *key_end = Some(byte(span.end));
                    last_end = byte(span.end);
                }
                Some(Frame::Mapping { key_end, .. }) => {
                    let key_end = key_end.take().expect("value follows a key");
                    // An empty plain value has no text of its own and its
                    // marker may already sit on the next line.
                    last_end = if value.is_empty() && style == ScalarStyle::Plain {
                        key_end
                    } else {
                        byte(span.end)
                    };
                }
                Some(Frame::Sequence) | None => last_end = byte(span.end),
            },
            Event::Alias(..) => {
                begin_value(&mut stack);
                end_value(&mut stack, None);
                last_end = byte(span.end);
            }
            Event::MappingEnd => {
                let Some(Frame::Mapping {
                    extendable,
                    indent,
                    mut entries,
                    ..
                }) = stack.pop()
                else {
                    bail!("mapping end without a mapping");
                };
                if let Some(last) = entries.last_mut() {
                    last.span.end = byte(span.start);
                }
                let mapping = extendable.then(|| Mapping {
                    indent,
                    end: line_end(text, last_end),
                    entries,
                });
                if stack.is_empty() {
                    root = mapping;
                    break;
                }
                end_value(&mut stack, mapping);
            }
            Event::SequenceEnd => {
                if stack.pop().is_none() {
                    bail!("sequence end without a sequence");
                }
                if stack.is_empty() {
                    break;
                }
                end_value(&mut stack, None);
            }
            _ => {}
        }
    }
    Ok(root)
}

/// A collection or alias is starting. Where a key is expected, that makes a
/// complex key, and the enclosing mapping cannot be extended.
fn begin_value(stack: &mut [Frame]) {
    if let Some(Frame::Mapping {
        key_end: None,
        extendable,
        ..
    }) = stack.last_mut()
    {
        *extendable = false;
    }
}

/// A nested node closed. When it was the value of an entry, attaches it.
fn end_value(stack: &mut [Frame], mapping: Option<Mapping>) {
    if let Some(Frame::Mapping {
        key_end, entries, ..
    }) = stack.last_mut()
        && key_end.take().is_some()
        && let Some(entry) = entries.last_mut()
    {
        entry.mapping = mapping;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULTS: &str = "height: 32\nnested:\n  enabled: true\n  spacing: 4.0\nterminal: xdg-terminal-exec\n";

    #[test]
    fn complete_file_is_untouched() {
        assert_eq!(extend(DEFAULTS, DEFAULTS).unwrap(), None);
    }

    #[test]
    fn empty_file_becomes_the_defaults() {
        assert_eq!(extend("", DEFAULTS).unwrap().as_deref(), Some(DEFAULTS));
    }

    #[test]
    fn missing_keys_are_appended_where_they_belong() {
        let text = "# mine\nnested:\n  spacing: 2.0 # tight\nheight: 40\n";
        let out = extend(text, DEFAULTS).unwrap().unwrap();
        assert_eq!(
            out,
            "# mine\nnested:\n  spacing: 2.0 # tight\n  enabled: true\nheight: 40\nterminal: xdg-terminal-exec\n"
        );
    }

    #[test]
    fn whole_missing_mapping_is_inserted_as_a_block() {
        let out = extend("height: 40\n", DEFAULTS).unwrap().unwrap();
        assert_eq!(
            out,
            "height: 40\nnested:\n  enabled: true\n  spacing: 4.0\nterminal: xdg-terminal-exec\n"
        );
    }

    #[test]
    fn nested_lines_precede_parent_lines_on_a_shared_last_line() {
        let out = extend("height: 40\nnested:\n  enabled: false", DEFAULTS).unwrap().unwrap();
        assert_eq!(
            out,
            "height: 40\nnested:\n  enabled: false\n  spacing: 4.0\nterminal: xdg-terminal-exec"
        );
    }

    #[test]
    fn deeper_user_indentation_is_followed() {
        let out = extend("nested:\n    enabled: false\nheight: 40\n", DEFAULTS).unwrap().unwrap();
        assert_eq!(
            out,
            "nested:\n    enabled: false\n    spacing: 4.0\nheight: 40\nterminal: xdg-terminal-exec\n"
        );
    }

    #[test]
    fn flow_mapping_is_left_alone() {
        let out = extend("height: 40\nnested: {enabled: false}\n", DEFAULTS).unwrap().unwrap();
        assert_eq!(
            out,
            "height: 40\nnested: {enabled: false}\nterminal: xdg-terminal-exec\n"
        );
    }

    #[test]
    fn empty_value_does_not_swallow_the_next_line() {
        let text = "nested:\n  enabled:\nheight: 40\n";
        let out = extend(text, DEFAULTS).unwrap().unwrap();
        assert_eq!(
            out,
            "nested:\n  enabled:\n  spacing: 4.0\nheight: 40\nterminal: xdg-terminal-exec\n"
        );
    }
}
