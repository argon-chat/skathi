//! Interaction states inside `:is()` / `:where()`. The cascade buckets a rule by the states it
//! needs, which a list of alternatives cannot say, so a group holding a state is split into one
//! rule per argument: `:where(a, b):where(:hover)` becomes `:where(a):where(:hover)` and
//! `:where(b):where(:hover)`. A one-argument group is the same as its argument, so its states are
//! the element's own.

const GROUPS: [&str; 6] = ["is", "where", "matches", "any", "-webkit-any", "-moz-any"];
const STATES: [&str; 5] = ["hover", "active", "focus", "focus-visible", "focus-within"];
const MAX_VARIANTS: usize = 64;

/// The selectors `text` splits into, or `None` when no group holds a state.
pub(crate) fn expand(text: &str) -> Option<Vec<String>> {
    let mut done = Vec::new();
    let mut work = vec![text.to_owned()];
    let mut split = false;
    while let Some(sel) = work.pop() {
        match split_first(&sel) {
            Some(parts) => {
                split = true;
                if done.len() + work.len() + parts.len() > MAX_VARIANTS {
                    return None;
                }
                work.extend(parts);
            }
            None => done.push(sel),
        }
    }
    split.then(|| {
        done.reverse();
        done
    })
}

/// Splits the first group with two or more arguments, one of which holds a state.
fn split_first(text: &str) -> Option<Vec<String>> {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' | b'\'' => i = skip_string(b, i),
            b'[' => i = skip_to(b, i, b'[', b']'),
            b':' => {
                let start = i + 1;
                let end = ident_end(b, start);
                let name = &text[start..end];
                if b.get(end) != Some(&b'(') {
                    i = end.max(i + 1);
                    continue;
                }
                let close = skip_to(b, end, b'(', b')');
                if !GROUPS.iter().any(|g| g.eq_ignore_ascii_case(name)) {
                    i = close;
                    continue;
                }
                let inner = &text[end + 1..close - 1];
                let args = top_level_args(inner);
                if args.len() > 1 && args.iter().any(|a| holds_state(a)) {
                    let (head, tail) = (&text[..end + 1], &text[close - 1..]);
                    return Some(
                        args.iter()
                            .map(|a| format!("{head}{}{tail}", a.trim()))
                            .collect(),
                    );
                }
                i = end + 1;
            }
            _ => i += 1,
        }
    }
    None
}

/// Whether `text` names a state outside `:not()`.
fn holds_state(text: &str) -> bool {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' | b'\'' => i = skip_string(b, i),
            b'[' => i = skip_to(b, i, b'[', b']'),
            b':' => {
                let start = i + 1;
                let end = ident_end(b, start);
                let name = &text[start..end];
                if b.get(end) == Some(&b'(') && name.eq_ignore_ascii_case("not") {
                    i = skip_to(b, end, b'(', b')');
                    continue;
                }
                if b.get(end) != Some(&b'(') && STATES.iter().any(|s| s.eq_ignore_ascii_case(name))
                {
                    return true;
                }
                i = end.max(i + 1);
            }
            _ => i += 1,
        }
    }
    false
}

fn top_level_args(inner: &str) -> Vec<&str> {
    let b = inner.as_bytes();
    let (mut args, mut from, mut i) = (Vec::new(), 0, 0);
    while i < b.len() {
        match b[i] {
            b'"' | b'\'' => i = skip_string(b, i),
            b'[' => i = skip_to(b, i, b'[', b']'),
            b'(' => i = skip_to(b, i, b'(', b')'),
            b',' => {
                args.push(&inner[from..i]);
                i += 1;
                from = i;
            }
            _ => i += 1,
        }
    }
    args.push(&inner[from..]);
    args
}

fn ident_end(b: &[u8], mut i: usize) -> usize {
    while i < b.len()
        && (b[i].is_ascii_alphanumeric() || b[i] == b'-' || b[i] == b'_' || b[i] >= 0x80)
    {
        i += 1;
    }
    i
}

/// The index past the bracket that closes the one at `open`.
fn skip_to(b: &[u8], open: usize, l: u8, r: u8) -> usize {
    let (mut depth, mut i) = (0usize, open);
    while i < b.len() {
        match b[i] {
            b'"' | b'\'' => {
                i = skip_string(b, i);
                continue;
            }
            c if c == l => depth += 1,
            c if c == r => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    b.len()
}

fn skip_string(b: &[u8], open: usize) -> usize {
    let q = b[open];
    let mut i = open + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            c if c == q => return i + 1,
            _ => i += 1,
        }
    }
    b.len()
}

#[cfg(test)]
mod tests {
    use super::expand;

    #[test]
    fn a_group_with_a_state_splits_per_argument() {
        assert_eq!(
            expand(":where(button:hover, input[type=\"a,b\"])").unwrap(),
            [":where(button:hover)", ":where(input[type=\"a,b\"])"]
        );
        assert_eq!(
            expand("a:is(:hover, :focus-visible) b").unwrap(),
            ["a:is(:hover) b", "a:is(:focus-visible) b"]
        );
    }

    #[test]
    fn groups_without_a_state_and_negated_states_stay_whole() {
        assert_eq!(expand(":where(a, b):hover"), None);
        assert_eq!(expand(":is(a:not(:hover), b)"), None);
        assert_eq!(expand(":where(a:active-view-transition, b)"), None);
    }
}
