//! Arithmetic for numeric text fields.
//!
//! Lets the timeline's frame box take `22/2` or `8+12` instead of a bare
//! number. A *leading* operator is relative to the value already in the field:
//! `+12` jumps twelve frames on, `/2` halves the frame number. That form exists
//! because egui selects the whole value when the box is clicked, so typing
//! replaces it — `+12` is the ergonomic spelling of `8+12`.
//!
//! Grammar (whitespace-insensitive):
//!
//! ```text
//! expr    := term (('+' | '-') term)*
//! term    := unary (('*' | '/' | '%') unary)*
//! unary   := ('+' | '-')* primary
//! primary := number | '(' expr ')'
//! ```
//!
//! Anything else — trailing garbage, an unbalanced paren, division by zero —
//! is rejected rather than guessed at, which leaves the field's current value
//! untouched.

/// Evaluate `input`. `current` is the field's present value, used as the left
/// operand when the input starts with an operator. `None` = not a valid
/// expression; the caller should leave the value alone.
pub fn eval(input: &str, current: f64) -> Option<f64> {
    let chars: Vec<char> = input.chars().collect();
    let mut p = Parser { s: &chars, i: 0 };
    p.skip_ws();
    // A leading operator means "apply this to what's already here". Note this
    // makes `-3` a relative step back rather than the literal -3; frame numbers
    // are never negative, so nothing is lost.
    let seed = matches!(p.peek(), Some('+' | '-' | '*' | '/' | '%')).then_some(current);
    let v = p.expr(seed)?;
    p.skip_ws();
    if p.i != p.s.len() {
        return None;
    }
    v.is_finite().then_some(v)
}

struct Parser<'a> {
    s: &'a [char],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn bump(&mut self) {
        self.i += 1;
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.bump();
        }
    }

    /// `seed` stands in for the first operand when the input opened with an
    /// operator. It is threaded down to [`Parser::term`] so that a leading
    /// `*` or `/` binds at the right precedence: `+12*2` is `current + 24`,
    /// while `*2` is `current * 2`.
    fn expr(&mut self, seed: Option<f64>) -> Option<f64> {
        let mut v = self.term(seed)?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some('+') => {
                    self.bump();
                    v += self.term(None)?;
                }
                Some('-') => {
                    self.bump();
                    v -= self.term(None)?;
                }
                _ => return Some(v),
            }
        }
    }

    fn term(&mut self, seed: Option<f64>) -> Option<f64> {
        let mut v = match seed {
            Some(s) => s,
            None => self.unary()?,
        };
        loop {
            self.skip_ws();
            let op = match self.peek() {
                Some(c @ ('*' | '/' | '%')) => c,
                _ => return Some(v),
            };
            self.bump();
            let rhs = self.unary()?;
            if op != '*' && rhs == 0.0 {
                return None;
            }
            v = match op {
                '*' => v * rhs,
                '/' => v / rhs,
                _ => v % rhs,
            };
        }
    }

    fn unary(&mut self) -> Option<f64> {
        self.skip_ws();
        match self.peek() {
            Some('-') => {
                self.bump();
                Some(-self.unary()?)
            }
            Some('+') => {
                self.bump();
                self.unary()
            }
            _ => self.primary(),
        }
    }

    fn primary(&mut self) -> Option<f64> {
        self.skip_ws();
        match self.peek() {
            Some('(') => {
                self.bump();
                let v = self.expr(None)?;
                self.skip_ws();
                if self.peek() != Some(')') {
                    return None;
                }
                self.bump();
                Some(v)
            }
            Some(c) if c.is_ascii_digit() || c == '.' => self.number(),
            _ => None,
        }
    }

    fn number(&mut self) -> Option<f64> {
        let start = self.i;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.bump();
        }
        if self.peek() == Some('.') {
            self.bump();
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.bump();
            }
        }
        let text: String = self.s[start..self.i].iter().collect();
        text.parse::<f64>().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::eval;

    #[test]
    fn evaluates_a_full_expression_against_the_current_frame() {
        // The two cases this exists for: half of frame 22, and 12 frames on
        // from frame 8.
        assert_eq!(eval("22/2", 22.0), Some(11.0));
        assert_eq!(eval("8+12", 8.0), Some(20.0));
    }

    #[test]
    fn a_leading_operator_is_relative_to_the_current_value() {
        assert_eq!(eval("+12", 8.0), Some(20.0));
        assert_eq!(eval("-3", 8.0), Some(5.0));
        assert_eq!(eval("/2", 22.0), Some(11.0));
        assert_eq!(eval("*2", 6.0), Some(12.0));
    }

    #[test]
    fn a_relative_expression_keeps_normal_precedence() {
        // current + (12 * 2), not (current + 12) * 2.
        assert_eq!(eval("+12*2", 8.0), Some(32.0));
        assert_eq!(eval("*2+4", 8.0), Some(20.0));
    }

    #[test]
    fn absolute_input_ignores_the_current_value() {
        assert_eq!(eval("7", 99.0), Some(7.0));
        assert_eq!(eval("0", 99.0), Some(0.0));
    }

    #[test]
    fn precedence_parens_and_whitespace() {
        assert_eq!(eval("2+3*4", 0.0), Some(14.0));
        assert_eq!(eval("(2+3)*4", 0.0), Some(20.0));
        assert_eq!(eval("  8 + 12 ", 0.0), Some(20.0));
        assert_eq!(eval("-(4+1)", 0.0), Some(-5.0));
        assert_eq!(eval("10%4", 0.0), Some(2.0));
    }

    #[test]
    fn decimals_are_kept_unrounded_for_the_caller() {
        assert_eq!(eval("8.5*2", 0.0), Some(17.0));
        assert_eq!(eval("22/4", 0.0), Some(5.5));
    }

    #[test]
    fn junk_is_rejected_so_the_field_keeps_its_value() {
        for bad in [
            "", "   ", "8+", "8 12", "(8+1", "8+1)", "abc", "8/0", "5%0", "()", "+",
        ] {
            assert_eq!(eval(bad, 8.0), None, "{bad:?} should not parse");
        }
    }
}
