use std::cell::Cell;

use cssparser::{ParseError, Parser, Token};
use lightningcss::traits::Parse as _;
use lightningcss::values::number::CSSNumber;

const INDEX: u8 = 1;
const COUNT: u8 = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SiblingValues {
    pub index: Option<u32>,
    pub count: Option<u32>,
}

thread_local! {
    static VALUES: Cell<(u32, u32, u8)> = const { Cell::new((1, 1, 0)) };
}

pub(crate) struct Scope((u32, u32, u8));

impl Scope {
    pub(crate) fn enter(index: u32, count: u32) -> Self {
        Self(VALUES.with(|values| values.replace((index, count, 0))))
    }

    pub(crate) fn read() -> SiblingValues {
        let (index, count, reads) = VALUES.with(Cell::get);
        SiblingValues {
            index: (reads & INDEX != 0).then_some(index),
            count: (reads & COUNT != 0).then_some(count),
        }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        VALUES.with(|values| values.set(self.0));
    }
}

fn plain_function_name(text: &str) -> bool {
    text.as_bytes()
        .windows(b"sibling-index(".len())
        .any(|part| {
            part.eq_ignore_ascii_case(b"sibling-index(")
                || part.eq_ignore_ascii_case(b"sibling-count(")
        })
}

pub(crate) fn contains_function(text: &str) -> bool {
    plain_function_name(text)
        || (text.contains('\\')
            && rewrite(text, 1, 1, false).map_or(true, |(value, _)| value.is_some()))
}

pub(crate) fn resolve(text: &str) -> Result<Option<String>, ()> {
    let (index, count, reads) = VALUES.with(Cell::get);
    let (value, used) = rewrite(text, index, count, false)?;
    VALUES.with(|v| v.set((index, count, reads | used)));
    Ok(value)
}

pub(crate) fn probe(text: &str) -> Result<Option<String>, ()> {
    rewrite(text, 1, 1, false).map(|(value, _)| value)
}

pub(crate) fn integer_math(text: &str) -> Option<String> {
    rewrite(text, 1, 1, true).ok()?.0
}

fn rewrite(
    text: &str,
    index: u32,
    count: u32,
    integer_math: bool,
) -> Result<(Option<String>, u8), ()> {
    if !integer_math && !plain_function_name(text) && !text.contains('\\') {
        return Ok((None, 0));
    }
    let mut parser = Parser::new(text);
    let mut edits = Vec::new();
    let used =
        collect_edits(&mut parser, index, count, integer_math, &mut edits).map_err(|_| ())?;
    if edits.is_empty() {
        return Ok((None, used));
    }
    let mut result = String::with_capacity(text.len());
    let mut last = 0;
    for (start, end, replacement) in edits {
        result.push_str(&text[last..start]);
        result.push_str(&replacement);
        last = end;
    }
    result.push_str(&text[last..]);
    Ok((Some(result), used))
}

fn collect_edits(
    parser: &mut Parser<'_>,
    index: u32,
    count: u32,
    integer_math: bool,
    edits: &mut Vec<(usize, usize, String)>,
) -> Result<u8, ParseError<()>> {
    let mut reads = 0;
    loop {
        let start = parser.position();
        let token = match parser.next_including_whitespace_and_comments() {
            Ok(token) => token,
            Err(_) => return Ok(reads),
        };
        match token {
            Token::Function(name)
                if name.eq_ignore_ascii_case("sibling-index")
                    || name.eq_ignore_ascii_case("sibling-count") =>
            {
                let (value, bit) = if name.eq_ignore_ascii_case("sibling-index") {
                    (index, INDEX)
                } else {
                    (count, COUNT)
                };
                parser.parse_nested_block(|p| Ok(p.expect_exhausted()?))?;
                edits.push((
                    start.byte_index(),
                    parser.position().byte_index(),
                    value.to_string(),
                ));
                reads |= bit;
            }
            Token::Function(name)
                if integer_math
                    && ["calc", "min", "max", "clamp"]
                        .iter()
                        .any(|function| name.eq_ignore_ascii_case(function)) =>
            {
                parser.parse_nested_block(|p| {
                    while p.next_including_whitespace_and_comments().is_ok() {}
                    Ok(())
                })?;
                let raw = parser.slice_from(start);
                if let Ok(value) = CSSNumber::parse_string(raw) {
                    let integer = (f64::from(value) + 0.5).floor() as i32;
                    edits.push((
                        start.byte_index(),
                        parser.position().byte_index(),
                        integer.to_string(),
                    ));
                }
            }
            Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock => {
                reads |= parser
                    .parse_nested_block(|p| collect_edits(p, index, count, integer_math, edits))?;
            }
            _ => {}
        }
    }
}
