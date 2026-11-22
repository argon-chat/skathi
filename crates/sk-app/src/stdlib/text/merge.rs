//! Three-way merge by lines, with git's conflict markers.

use super::diff::{self, Algorithm, Change, Options, Pieces, NEWLINE};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Style {
    Merge,
    Diff3,
    Zdiff3,
}

/// Line ranges, the end not included, and the line of the `<<<<<<<` marker in the result.
#[derive(Debug, PartialEq)]
pub(super) struct Conflict {
    pub(super) base: (usize, usize),
    pub(super) ours: (usize, usize),
    pub(super) theirs: (usize, usize),
    pub(super) at: usize,
}

pub(super) struct Merged {
    pub(super) text: Vec<u16>,
    pub(super) conflicts: Vec<Conflict>,
}

struct Writer<'e> {
    text: Vec<u16>,
    lines: usize,
    /// The last line written has no `\n`.
    open: bool,
    eol: &'e [u16],
}

impl Writer<'_> {
    fn lines(&mut self, side: &Pieces, (from, to): (usize, usize)) {
        for i in from..to {
            if self.open {
                self.text.push(NEWLINE);
            }
            self.text.extend_from_slice(side.piece(i));
            self.open = side.unterminated(i);
            if !self.open {
                self.text.push(NEWLINE);
            }
            self.lines += 1;
        }
    }

    fn marker(&mut self, mark: &str, label: &[u16]) {
        if self.open {
            self.text.extend_from_slice(self.eol);
            self.open = false;
        }
        self.text.extend(mark.encode_utf16());
        if !label.is_empty() {
            self.text.push(b' ' as u16);
            self.text.extend_from_slice(label);
        }
        self.text.extend_from_slice(self.eol);
        self.lines += 1;
    }
}

/// The range a side has for the base lines `start..end`, given the side's changes there, or
/// its offset when it has none.
fn side_range(changes: &[Change], (start, end): (usize, usize), delta: isize) -> (usize, usize) {
    match (changes.first(), changes.last()) {
        (Some(first), Some(last)) => (
            first.new.0 - (first.old.0 - start),
            last.new.1 + (end - last.old.1),
        ),
        _ => (
            (start as isize + delta) as usize,
            (end as isize + delta) as usize,
        ),
    }
}

fn delta_of(changes: &[Change]) -> isize {
    changes
        .iter()
        .map(|c| (c.new.1 - c.new.0) as isize - (c.old.1 - c.old.0) as isize)
        .sum()
}

/// Merges the changes from `base` to `ours` and from `base` to `theirs`. Changes that overlap or
/// touch conflict unless they are the same, as in git. `labels` are base, ours and theirs.
pub(super) fn merge(
    base: &[u16],
    ours: &[u16],
    theirs: &[u16],
    style: Style,
    labels: [&[u16]; 3],
) -> Merged {
    let sides = [
        Pieces::lines(base),
        Pieces::lines(ours),
        Pieces::lines(theirs),
    ];
    let (ids, tokens) = diff::intern(&[&sides[0], &sides[1], &sides[2]], &Options::default());
    let changes = |side: usize| {
        diff::changes_of(&diff::diff(
            &ids[0],
            &ids[side],
            tokens,
            Algorithm::Histogram,
        ))
    };
    let (first, second) = (changes(1), changes(2));
    let crlf = sides
        .iter()
        .find(|s| s.len() > 0)
        .is_some_and(|s| s.piece(0).last() == Some(&(b'\r' as u16)));
    let eol: Vec<u16> = if crlf { vec![13, 10] } else { vec![10] };
    let mut out = Writer {
        text: Vec::with_capacity(ours.len().max(theirs.len())),
        lines: 0,
        open: false,
        eol: &eol,
    };
    let mut conflicts = Vec::new();
    let (mut i, mut j, mut done) = (0, 0, 0);
    let (mut d1, mut d2) = (0isize, 0isize);
    while i < first.len() || j < second.len() {
        let start = match (first.get(i), second.get(j)) {
            (Some(a), Some(b)) => a.old.0.min(b.old.0),
            (Some(a), None) => a.old.0,
            (None, Some(b)) => b.old.0,
            (None, None) => unreachable!(),
        };
        out.lines(&sides[0], (done, start));
        let (mut end, mut i2, mut j2) = (start, i, j);
        loop {
            let before = (i2, j2);
            while let Some(c) = first.get(i2).filter(|c| c.old.0 <= end) {
                end = end.max(c.old.1);
                i2 += 1;
            }
            while let Some(c) = second.get(j2).filter(|c| c.old.0 <= end) {
                end = end.max(c.old.1);
                j2 += 1;
            }
            if before == (i2, j2) {
                break;
            }
        }
        let ours_range = side_range(&first[i..i2], (start, end), d1);
        let theirs_range = side_range(&second[j..j2], (start, end), d2);
        d1 += delta_of(&first[i..i2]);
        d2 += delta_of(&second[j..j2]);
        let same = ids[1][ours_range.0..ours_range.1] == ids[2][theirs_range.0..theirs_range.1];
        if i == i2 {
            out.lines(&sides[2], theirs_range);
        } else if j == j2 || same {
            out.lines(&sides[1], ours_range);
        } else {
            let ((mut a1, mut b1), (mut a2, mut b2)) = (ours_range, theirs_range);
            if style != Style::Diff3 {
                while a1 < b1 && a2 < b2 && ids[1][a1] == ids[2][a2] {
                    a1 += 1;
                    a2 += 1;
                }
                while a1 < b1 && a2 < b2 && ids[1][b1 - 1] == ids[2][b2 - 1] {
                    b1 -= 1;
                    b2 -= 1;
                }
            }
            out.lines(&sides[1], (ours_range.0, a1));
            let at = out.lines;
            out.marker("<<<<<<<", labels[1]);
            out.lines(&sides[1], (a1, b1));
            if style != Style::Merge {
                out.marker("|||||||", labels[0]);
                out.lines(&sides[0], (start, end));
            }
            out.marker("=======", &[]);
            out.lines(&sides[2], (a2, b2));
            out.marker(">>>>>>>", labels[2]);
            out.lines(&sides[1], (b1, ours_range.1));
            conflicts.push(Conflict {
                base: (start, end),
                ours: (a1, b1),
                theirs: (a2, b2),
                at,
            });
        }
        (done, i, j) = (end, i2, j2);
    }
    out.lines(&sides[0], (done, sides[0].len()));
    Merged {
        text: out.text,
        conflicts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    fn run(base: &str, ours: &str, theirs: &str, style: Style) -> (String, Vec<Conflict>) {
        let labels = [u("base"), u("ours"), u("theirs")];
        let m = merge(
            &u(base),
            &u(ours),
            &u(theirs),
            style,
            [&labels[0], &labels[1], &labels[2]],
        );
        (String::from_utf16(&m.text).unwrap(), m.conflicts)
    }

    const BASE: &str = "one\ntwo\nthree\nfour\nfive\nsix\nseven\n";

    #[test]
    fn changes_apart_merge_cleanly() {
        let ours = BASE.replace("two", "2");
        let theirs = BASE
            .replace("six", "6")
            .replace("seven\n", "seven\neight\n");
        let (text, conflicts) = run(BASE, &ours, &theirs, Style::Merge);
        assert!(conflicts.is_empty());
        assert_eq!(text, "one\n2\nthree\nfour\nfive\n6\nseven\neight\n");
        let (same, conflicts) = run(BASE, &ours, &ours, Style::Merge);
        assert!(conflicts.is_empty());
        assert_eq!(same, ours);
        assert_eq!(run(BASE, BASE, &theirs, Style::Diff3).0, theirs);
        assert_eq!(run("", "", "", Style::Merge).0, "");
    }

    #[test]
    fn overlapping_changes_conflict_with_markers_and_positions() {
        let ours = BASE.replace("three", "3");
        let theirs = BASE.replace("three", "THREE");
        let (text, conflicts) = run(BASE, &ours, &theirs, Style::Merge);
        assert_eq!(
            text,
            "one\ntwo\n<<<<<<< ours\n3\n=======\nTHREE\n>>>>>>> theirs\nfour\nfive\nsix\nseven\n"
        );
        assert_eq!(
            conflicts,
            [Conflict {
                base: (2, 3),
                ours: (2, 3),
                theirs: (2, 3),
                at: 2
            }]
        );
        let (diff3, _) = run(BASE, &ours, &theirs, Style::Diff3);
        assert!(diff3
            .contains("<<<<<<< ours\n3\n||||||| base\nthree\n=======\nTHREE\n>>>>>>> theirs\n"));
    }

    #[test]
    fn touching_changes_conflict_as_in_git() {
        let ours = BASE.replace("two\n", "two\nafter two\n");
        let theirs = BASE.replace("three", "3");
        let (_, conflicts) = run(BASE, &ours, &theirs, Style::Merge);
        assert_eq!(conflicts.len(), 1);
    }

    #[test]
    fn common_lines_leave_the_conflict_unless_diff3_shows_it_whole() {
        let ours = BASE.replace("three\nfour", "same\nOURS\nend");
        let theirs = BASE.replace("three\nfour", "same\nTHEIRS\nend");
        let (text, conflicts) = run(BASE, &ours, &theirs, Style::Zdiff3);
        assert!(text.contains(
            "two\nsame\n<<<<<<< ours\nOURS\n||||||| base\nthree\nfour\n=======\nTHEIRS\n>>>>>>> theirs\nend\nfive\n"
        ));
        assert_eq!(
            (conflicts[0].ours, conflicts[0].theirs, conflicts[0].at),
            ((3, 4), (3, 4), 3)
        );
        let (diff3, conflicts) = run(BASE, &ours, &theirs, Style::Diff3);
        assert!(diff3.contains("<<<<<<< ours\nsame\nOURS\nend\n||||||| base\n"));
        assert_eq!(conflicts[0].ours, (2, 5));
    }

    #[test]
    fn several_conflicts_count_lines_in_the_result() {
        let ours = BASE.replace("two", "2a").replace("six", "6a");
        let theirs = BASE.replace("two", "2b").replace("six", "6b");
        let (text, conflicts) = run(BASE, &ours, &theirs, Style::Merge);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(conflicts.len(), 2);
        for c in &conflicts {
            assert_eq!(lines[c.at], "<<<<<<< ours");
        }
    }

    #[test]
    fn one_sided_and_identical_changes_never_conflict() {
        let mut seed = 0xdead_beef_u64;
        let mut next = |below: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % below
        };
        let words = ["a", "b", "c", "}", "", "x"];
        for _ in 0..2000 {
            let mut text = |n: u64| {
                let lines: Vec<&str> = (0..next(n)).map(|_| words[next(6) as usize]).collect();
                let mut t = lines.join("\n");
                if next(2) == 0 {
                    t.push('\n');
                }
                t
            };
            let (base, other) = (text(12), text(12));
            for style in [Style::Merge, Style::Diff3, Style::Zdiff3] {
                assert_eq!(run(&base, &other, &base, style), (other.clone(), vec![]));
                assert_eq!(run(&base, &base, &other, style), (other.clone(), vec![]));
                assert_eq!(run(&base, &other, &other, style), (other.clone(), vec![]));
            }
        }
    }

    #[test]
    fn a_missing_final_newline_and_crlf_are_kept() {
        let (text, conflicts) = run("a\nm\nb", "a\nm\nb\nc", "z\nm\nb", Style::Merge);
        assert!(conflicts.is_empty(), "{text:?}");
        assert_eq!(text, "z\nm\nb\nc");
        let (text, conflicts) = run("a\r\nb\r\n", "A\r\nb\r\n", "a!\r\nb\r\n", Style::Merge);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(
            text,
            "<<<<<<< ours\r\nA\r\n=======\r\na!\r\n>>>>>>> theirs\r\nb\r\n"
        );
        let (text, _) = run("x", "y", "z", Style::Merge);
        assert_eq!(text, "<<<<<<< ours\ny\n=======\nz\n>>>>>>> theirs\n");
    }
}
