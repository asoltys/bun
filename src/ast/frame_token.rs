//! The tokens JSC reports a stack frame at that the tree has no position for. The printer maps one where it starts a line, and finds it in the source text here.

use crate::{ExprData, Loc};
use bun_core::strings;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FrameToken {
    /// The `.`, `?.` or `[` in front of the name or the index of a member.
    Member,
    /// The end of the `?.` in front of the arguments of an optional call.
    OptionalCall,
    /// The end of the callee of a call, in front of its `(`.
    EndOfCallee,
    /// A token whose position the printer knows.
    Known,
}

impl FrameToken {
    /// The token in `source`, looking back from `anchor` over blanks and `(`. None where anything else stands between, the token is not behind `first`, or a line comment may hold it.
    pub fn find(self, source: &[u8], anchor: Loc, first: Loc) -> Option<Loc> {
        if self == FrameToken::Known {
            return Some(anchor);
        }
        let from = usize::try_from(anchor.start).ok()?;
        if from > source.len() || anchor.start <= first.start {
            return None;
        }
        let mut at = from;
        let mut skipped_parenthesis = false;
        loop {
            let before = &source[..at];
            at -= match strings::js_whitespace_len_back(before) {
                0 if before.ends_with(b"(") => {
                    skipped_parenthesis = true;
                    1
                }
                0 => break,
                len => len,
            };
        }
        let before = &source[..at];
        let start = match self {
            // `a["b"]` may be printed `a.b`, and the other way around.
            FrameToken::Member => match before {
                [.., b'?', b'.'] => at - 2,
                [.., b'.' | b'['] => at - 1,
                _ => return None,
            },
            FrameToken::OptionalCall => match before {
                [.., b'?', b'.'] => at,
                _ => return None,
            },
            FrameToken::EndOfCallee => match before {
                [.., b')' | b'}' | b']' | b'`'] if skipped_parenthesis => at,
                _ => return None,
            },
            FrameToken::Known => return Some(anchor),
        };
        if may_be_in_line_comment(source, start, from) {
            return None;
        }
        let start = i32::try_from(start).ok()?;
        (start > first.start).then_some(Loc { start })
    }

    /// `find` where a source map has the token: where `behind` and the token are on another line than `in_front`, the last mapping in front. The printer applies the same rule by line number.
    pub fn find_on_new_line(
        self,
        text: &[u8],
        behind: Loc,
        first: Loc,
        in_front: Loc,
    ) -> Option<Loc> {
        if are_on_one_line(text, behind, in_front) {
            return None;
        }
        let found = self.find(text, behind, first)?;
        (!are_on_one_line(text, found, in_front)).then_some(found)
    }
}

/// Whether `token` is on a line above `anchor` that has a `//` in front of the token or begins with `-->`. A token on the line of `anchor` is in no such comment.
fn may_be_in_line_comment(source: &[u8], token: usize, anchor: usize) -> bool {
    if !strings::contains_js_line_break(&source[token..anchor]) {
        return false;
    }
    let before = &source[..token];
    let mut line = &before[strings::last_index_of_any(before, b"\r\n").map_or(0, |at| at + 1)..];
    while let Some((at, len)) = strings::find_js_line_break(line) {
        line = &line[at + len..];
    }
    strings::contains(line, b"//") || strings::trim_js_whitespace_start(line).starts_with(b"-->")
}

/// Whether no line of `source` ends between the two positions: a source map has both on one line.
pub fn are_on_one_line(source: &[u8], a: Loc, b: Loc) -> bool {
    let (Ok(a), Ok(b)) = (usize::try_from(a.start), usize::try_from(b.start)) else {
        return true;
    };
    let (from, to) = (a.min(b).min(source.len()), a.max(b).min(source.len()));
    !strings::contains_js_line_break(&source[from..to])
}

/// Whether the printer writes `data` as a name. JSC reports a name that is not defined where the name ends: a token mapped right behind it would take that frame.
#[inline]
pub fn is_identifier_kind(data: &ExprData) -> bool {
    matches!(
        data,
        ExprData::EIdentifier(_)
            | ExprData::EImportIdentifier(_)
            | ExprData::ECommonjsExportIdentifier(_)
            | ExprData::ERequireCallTarget
            | ExprData::ERequireResolveCallTarget
    )
}

/// Whether a callee ends in a name: a name, a member behind a dot or `super`. JSC reports its call at that name, not in front of the `(`.
#[inline]
pub fn callee_ends_in_name(callee: &ExprData) -> bool {
    is_identifier_kind(callee) || matches!(callee, ExprData::EDot(_) | ExprData::ESuper(_))
}

#[cfg(test)]
mod tests {
    use super::FrameToken::{EndOfCallee, Known, Member, OptionalCall};
    use super::*;

    /// Where the last `text` of `source` starts.
    fn position(source: &str, text: &str) -> Loc {
        let at = strings::last_index_of(source.as_bytes(), text.as_bytes());
        let at = at.expect("the text is in the source");
        Loc {
            start: i32::try_from(at).unwrap(),
        }
    }

    fn find(token: FrameToken, source: &str, anchor: &str, first: i32) -> Option<i32> {
        let anchor = position(source, anchor);
        let found = token.find(source.as_bytes(), anchor, Loc { start: first });
        found.map(|loc| loc.start)
    }

    fn find_on_new_line(token: FrameToken, text: &str, behind: &str) -> Option<i32> {
        let (behind, first) = (position(text, behind), Loc { start: 0 });
        let found = token.find_on_new_line(text.as_bytes(), behind, first, first);
        found.map(|loc| loc.start)
    }

    #[test]
    fn a_member_is_found_at_its_punctuator() {
        assert_eq!(find(Member, "a\n  .b", "b", 0), Some(4));
        assert_eq!(find(Member, "a\n  ?.b", "b", 0), Some(4));
        assert_eq!(find(Member, "a\n  [ (b)]", "b", 0), Some(4));
        assert_eq!(find(Member, "a?.[b]", "b", 0), Some(3));
        assert_eq!(find(Member, "a /* c */ b", "b", 0), None);
    }

    #[test]
    fn a_call_is_found_behind_its_callee() {
        assert_eq!(find(EndOfCallee, "(() => {\n})(1)", "1", 0), Some(11));
        assert_eq!(find(EndOfCallee, "f()\n  ((1))", "1", 0), Some(3));
        assert_eq!(find(EndOfCallee, "f(1)", "1", 0), None);
        assert_eq!(find(OptionalCall, "f\n  ?.(1)", "1", 0), Some(6));
        assert_eq!(find(OptionalCall, "f\n  ?. ( (1))", "1", 0), Some(6));
    }

    #[test]
    fn a_known_token_is_where_the_tree_has_it() {
        assert_eq!(find(Known, "[a]", "[", 0), Some(0));
    }

    #[test]
    fn blanks_that_are_not_ascii_are_stepped_over() {
        assert_eq!(find(Member, "a\n\u{A0}.\u{3000}b", "b", 0), Some(4));
        assert_eq!(find(Member, "a\u{2028}[\u{FEFF}b]", "b", 0), Some(4));
        assert_eq!(find(EndOfCallee, "f()\u{2029}\u{A0}(1)", "1", 0), Some(3));
    }

    #[test]
    fn what_was_written_elsewhere_is_not_found() {
        // The key of `o[KEY]` where `KEY` is printed as its value: the value stands in front of `o`.
        assert_eq!(find(Member, "const KEY = [1];\no[KEY]", "1", 17), None);
    }

    #[test]
    fn a_token_in_a_line_comment_is_not_found() {
        let shapes = [
            (EndOfCallee, "thrower()\n// note)\n(1);", "1);"),
            (
                EndOfCallee,
                "(function () {\n})\n// call it (now)\n();",
                ");",
            ),
            (EndOfCallee, "(() => {\n})\n// see {braces}\n();", ");"),
            (EndOfCallee, "[thrower()][0]\n// see [1]\n(1);", "1);"),
            (
                EndOfCallee,
                "(() => {\n})()\n// (then call it)\n(1);",
                "1);",
            ),
            (EndOfCallee, "(0, thrower())\n// note)\n(1);", "1);"),
            (EndOfCallee, "String.raw`x`\n// a `quote`\n(1);", "1);"),
            (EndOfCallee, "thrower()\r\n// note)\r\n(1);", "1);"),
            (EndOfCallee, "thrower() // note)\u{2028}(1);", "1);"),
            (EndOfCallee, "thrower()\n  --> note)\n(1);", "1);"),
            (Member, "o.a.\n  // see the docs.\n  b;", "b;"),
            (Member, "o.a[\n  // the first [\n  0\n];", "0\n"),
            (
                Member,
                "(function () {\n})().\n  // explain it.\n  x;",
                "x;",
            ),
            (Member, "o?.\n  // or o?.\n  boom;", "boom"),
            (OptionalCall, "thrower()?.(\n  // maybe?.(\n  1);", "1);"),
        ];
        for (token, source, anchor) in shapes {
            assert_eq!(find(token, source, anchor, 0), None, "{source}");
        }
    }

    #[test]
    fn a_token_outside_a_comment_is_found() {
        assert_eq!(
            find(EndOfCallee, "(thrower()\n)\n  (1);", "1);", 0),
            Some(12)
        );
        assert_eq!(find(Member, "o\n  .a\n  .b;", "a\n", 0), Some(4));
        assert_eq!(find(Member, "o\n  .a\n  .b;", "b;", 0), Some(9));
        assert_eq!(find(EndOfCallee, "(() => {\n})();", ");", 0), Some(11));
        assert_eq!(
            find(Member, "o.a\n  // the dot is below.\n  .b;", "b;", 0),
            Some(29)
        );
        assert_eq!(
            find(EndOfCallee, "(f(\"http://x/\")\n) (1);", "1);", 0),
            Some(17)
        );
        assert_eq!(
            find(EndOfCallee, "x // y\u{2028}f()\n(1);", "1);", 0),
            Some(12)
        );
        // A `//` in a string on the line of the token reads as a comment: that token is given up.
        assert_eq!(
            find(EndOfCallee, "f(\"http://x/\")\n  (1);", "1);", 0),
            None
        );
    }

    #[test]
    fn a_token_is_found_only_where_it_starts_a_line() {
        assert_eq!(find_on_new_line(Member, "f().b", "b"), None);
        assert_eq!(find_on_new_line(Member, "f()\n  .b", "b"), Some(6));
        assert_eq!(find_on_new_line(Member, "f()\r\n  .b", "b"), Some(7));
        assert_eq!(find_on_new_line(Member, "f()\n.\nb", "b"), Some(4));
        // A lookup finds a token that ends the line of the mapping in front as it is.
        assert_eq!(find_on_new_line(Member, "f().\n  b", "b"), None);
        assert_eq!(find_on_new_line(Member, "f()\n  // see.\n  b", "b"), None);
        assert_eq!(find_on_new_line(OptionalCall, "f()\n  ?.(1)", "1"), Some(8));
        assert_eq!(find_on_new_line(EndOfCallee, "(f)\n  (1)", "1"), None);
        assert_eq!(find_on_new_line(EndOfCallee, "(f\n)\n  (1)", "1"), Some(4));
        assert_eq!(find_on_new_line(Known, "f`x`", "`x"), None);
        assert_eq!(find_on_new_line(Known, "f\n`x`", "`x"), Some(2));
    }

    #[test]
    fn lines() {
        let on_one_line = |source: &str, a: i32, b: i32| {
            are_on_one_line(source.as_bytes(), Loc { start: a }, Loc { start: b })
        };
        assert!(on_one_line("a.b", 0, 2));
        assert!(!on_one_line("a\n.b", 0, 2));
        assert!(!on_one_line("a\n.b", 2, 0));
        assert!(!on_one_line("a\r\n.b", 0, 3));
        assert!(!on_one_line("a\r.b", 0, 2));
        assert!(!on_one_line("a\u{2028}.b", 0, 4));
        assert!(on_one_line("a\n.b", 2, 3));
        assert!(on_one_line("a\n.b", -1, 3));
        assert!(on_one_line("a\n.b", 9, 12));
    }
}
