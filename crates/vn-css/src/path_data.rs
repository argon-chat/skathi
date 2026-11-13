//! SVG path data (the `d` grammar) for CSS `path()`, into gpui's absolute `VnPathVerb`s. Numbers
//! take SVG's compact forms (`1.5.5` is two numbers; an arc's flags need no separator). A string
//! that breaks the grammar is an error that invalidates the declaration, as in Chromium, rather
//! than drawing the part before the error as SVG `d` would.

use gpui::VnPathVerb;

pub(crate) fn parse(d: &str) -> Result<Vec<VnPathVerb>, String> {
    let mut p = Parser { s: d.as_bytes(), i: 0 };
    let mut out = Vec::new();
    let mut current = [0f32; 2];
    let mut start = [0f32; 2];
    let mut last_cubic: Option<[f32; 2]> = None;
    let mut last_quad: Option<[f32; 2]> = None;
    p.skip_separators();
    if p.done() {
        return Ok(out);
    }
    let mut command = match p.peek() {
        Some(b'M' | b'm') => p.next_command().expect("peeked"),
        _ => return Err(format!("path data must start with `M` or `m`: {d:?}")),
    };
    let mut first_of_command = true;
    loop {
        let relative = command.is_ascii_lowercase();
        let base = if relative { current } else { [0., 0.] };
        let at = |x: f32, y: f32| [base[0] + x, base[1] + y];
        match command.to_ascii_uppercase() {
            b'M' => {
                let (x, y) = (p.number()?, p.number()?);
                let pt = at(x, y);
                if first_of_command {
                    out.push(VnPathVerb::MoveTo(pt));
                    start = pt;
                } else {
                    out.push(VnPathVerb::LineTo(pt));
                }
                current = pt;
                last_cubic = None;
                last_quad = None;
            }
            b'L' => {
                let (x, y) = (p.number()?, p.number()?);
                current = at(x, y);
                out.push(VnPathVerb::LineTo(current));
                last_cubic = None;
                last_quad = None;
            }
            b'H' => {
                let x = p.number()?;
                current = [if relative { current[0] + x } else { x }, current[1]];
                out.push(VnPathVerb::LineTo(current));
                last_cubic = None;
                last_quad = None;
            }
            b'V' => {
                let y = p.number()?;
                current = [current[0], if relative { current[1] + y } else { y }];
                out.push(VnPathVerb::LineTo(current));
                last_cubic = None;
                last_quad = None;
            }
            b'C' => {
                let c1 = at(p.number()?, p.number()?);
                let c2 = at(p.number()?, p.number()?);
                let to = at(p.number()?, p.number()?);
                out.push(VnPathVerb::CubicTo(c1, c2, to));
                current = to;
                last_cubic = Some(c2);
                last_quad = None;
            }
            b'S' => {
                let c1 = match last_cubic {
                    Some(c) => [2. * current[0] - c[0], 2. * current[1] - c[1]],
                    None => current,
                };
                let c2 = at(p.number()?, p.number()?);
                let to = at(p.number()?, p.number()?);
                out.push(VnPathVerb::CubicTo(c1, c2, to));
                current = to;
                last_cubic = Some(c2);
                last_quad = None;
            }
            b'Q' => {
                let c = at(p.number()?, p.number()?);
                let to = at(p.number()?, p.number()?);
                out.push(VnPathVerb::QuadTo(c, to));
                current = to;
                last_quad = Some(c);
                last_cubic = None;
            }
            b'T' => {
                let c = match last_quad {
                    Some(c) => [2. * current[0] - c[0], 2. * current[1] - c[1]],
                    None => current,
                };
                let to = at(p.number()?, p.number()?);
                out.push(VnPathVerb::QuadTo(c, to));
                current = to;
                last_quad = Some(c);
                last_cubic = None;
            }
            b'A' => {
                let rx = p.number()?;
                let ry = p.number()?;
                let rotation = p.number()?;
                let large_arc = p.flag()?;
                let sweep = p.flag()?;
                let to = at(p.number()?, p.number()?);
                VnPathVerb::push_arc(&mut out, current, [rx, ry], rotation, large_arc, sweep, to);
                current = to;
                last_cubic = None;
                last_quad = None;
            }
            b'Z' => {
                out.push(VnPathVerb::Close);
                current = start;
                last_cubic = None;
                last_quad = None;
            }
            other => return Err(format!("`{}` is not a path command", other as char)),
        }
        p.skip_separators();
        if p.done() {
            return Ok(out);
        }
        match p.peek() {
            Some(c) if c.is_ascii_alphabetic() => {
                command = p.next_command().ok_or_else(|| format!("`{}` is not a path command", c as char))?;
                first_of_command = true;
            }
            _ if command.eq_ignore_ascii_case(&b'Z') => {
                return Err(format!("a number after `Z` in {d:?}"));
            }
            _ => first_of_command = false,
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn done(&self) -> bool {
        self.i >= self.s.len()
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')) {
            self.i += 1;
        }
    }

    fn skip_separators(&mut self) {
        self.skip_whitespace();
        if self.peek() == Some(b',') {
            self.i += 1;
            self.skip_whitespace();
        }
    }

    fn next_command(&mut self) -> Option<u8> {
        let c = self.peek()?;
        if b"MmLlHhVvCcSsQqTtAaZz".contains(&c) {
            self.i += 1;
            Some(c)
        } else {
            None
        }
    }

    fn number(&mut self) -> Result<f32, String> {
        self.skip_whitespace();
        let begin = self.i;
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.i += 1;
        }
        let mut digits = 0;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.i += 1;
            digits += 1;
        }
        if self.peek() == Some(b'.') {
            self.i += 1;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            return Err(self.expected("a number", begin));
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let mark = self.i;
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if matches!(self.peek(), Some(b'0'..=b'9')) {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.i += 1;
                }
            } else {
                // Not an exponent after all (an `e` cannot start a command, so this is the error
                // the next read reports).
                self.i = mark;
            }
        }
        let text = std::str::from_utf8(&self.s[begin..self.i]).map_err(|e| e.to_string())?;
        let value: f32 = text.parse().map_err(|_| self.expected("a number", begin))?;
        if !value.is_finite() {
            return Err(self.expected("a finite number", begin));
        }
        self.skip_separators();
        Ok(value)
    }

    fn flag(&mut self) -> Result<bool, String> {
        self.skip_whitespace();
        let flag = match self.peek() {
            Some(b'0') => false,
            Some(b'1') => true,
            _ => return Err(self.expected("an arc flag, 0 or 1", self.i)),
        };
        self.i += 1;
        self.skip_separators();
        Ok(flag)
    }

    fn expected(&self, what: &str, at: usize) -> String {
        let rest = String::from_utf8_lossy(&self.s[at.min(self.s.len())..]);
        let rest: String = rest.chars().take(16).collect();
        format!("expected {what} at {rest:?}")
    }
}

#[cfg(test)]
mod tests {
    use super::parse;
    use gpui::VnPathVerb::{self, *};

    fn ok(d: &str) -> Vec<VnPathVerb> {
        parse(d).unwrap_or_else(|e| panic!("{d}: {e}"))
    }

    #[test]
    fn absolute_and_relative_lines_close_and_repeat() {
        assert_eq!(
            ok("M10 20 L30 40 H50 V60 Z"),
            vec![MoveTo([10., 20.]), LineTo([30., 40.]), LineTo([50., 40.]), LineTo([50., 60.]), Close]
        );
        assert_eq!(
            ok("m10,20 10,0 0,10 z l5 5 h-5 v-5"),
            vec![
                MoveTo([10., 20.]),
                LineTo([20., 20.]),
                LineTo([20., 30.]),
                Close,
                LineTo([15., 25.]),
                LineTo([10., 25.]),
                LineTo([10., 20.]),
            ]
        );
        assert_eq!(ok("M0 0 L1 1 2 2"), vec![MoveTo([0., 0.]), LineTo([1., 1.]), LineTo([2., 2.])]);
        assert_eq!(ok(""), vec![]);
        assert_eq!(ok("  \n "), vec![]);
    }

    #[test]
    fn numbers_take_the_compact_forms() {
        assert_eq!(ok("M.5.5-1-2"), vec![MoveTo([0.5, 0.5]), LineTo([-1., -2.])]);
        assert_eq!(ok("M1e1 2E-1L+3 -4.25e+0"), vec![MoveTo([10., 0.2]), LineTo([3., -4.25])]);
        assert_eq!(ok("M0 0 H12.150600000000001"), vec![MoveTo([0., 0.]), LineTo([12.1506, 0.])]);
    }

    #[test]
    fn curves_reflect_and_relative_curves_add_the_current_point() {
        assert_eq!(
            ok("M0 0 C10 0 20 10 20 20 S30 40 40 40"),
            vec![
                MoveTo([0., 0.]),
                CubicTo([10., 0.], [20., 10.], [20., 20.]),
                CubicTo([20., 30.], [30., 40.], [40., 40.]),
            ]
        );
        assert_eq!(
            ok("M10 10 q10 0 10 10 t10 10"),
            vec![MoveTo([10., 10.]), QuadTo([20., 10.], [20., 20.]), QuadTo([20., 30.], [30., 30.])]
        );
        assert_eq!(
            ok("M0 0 L10 0 S20 10 20 20 T30 30"),
            vec![
                MoveTo([0., 0.]),
                LineTo([10., 0.]),
                CubicTo([10., 0.], [20., 10.], [20., 20.]),
                QuadTo([20., 20.], [30., 30.]),
            ]
        );
    }

    #[test]
    fn arcs_become_cubics_and_flags_need_no_separator() {
        let v = ok("M0 50 a50 50 0 0110 0");
        assert!(v.len() >= 2, "{v:?}");
        assert_eq!(v.last().map(|verb| matches!(verb, CubicTo(_, _, [x, y]) if *x == 10. && *y == 50.)), Some(true));
        let half = ok("M0 50 A50 50 0 0 1 100 50");
        assert!(half[1..].iter().all(|verb| matches!(verb, CubicTo(..))));
        assert_eq!(ok("M0 0 A0 5 0 0 0 10 0"), vec![MoveTo([0., 0.]), LineTo([10., 0.])]);
    }

    #[test]
    fn the_overlay_notch_parses() {
        let d = "M0 0 C12.1506 0 22 9.8494 22 22 V42 Q22 64 44 64 H420 Q442 64 442 42 V22 \
                 C442 9.8494 451.8494 0 464 0 Z";
        let v = ok(d);
        assert_eq!(v.len(), 9);
        assert_eq!(v[0], MoveTo([0., 0.]));
        assert_eq!(v[2], LineTo([22., 42.]));
        assert_eq!(v[3], QuadTo([22., 64.], [44., 64.]));
        assert_eq!(v[8], Close);
    }

    #[test]
    fn broken_data_is_an_error() {
        for bad in ["L0 0", "M0", "M0 0 L1", "M0 0 X1 1", "M0 0 A1 1 0 2 0 1 1", "M0 0 Z 1", "M1e 2"] {
            assert!(parse(bad).is_err(), "{bad} parsed: {:?}", parse(bad));
        }
    }
}
