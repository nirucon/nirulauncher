//! The calculator row: plain arithmetic typed in the search field.
//!
//! A small recursive-descent parser over `f64`. It accepts numbers (with `.`
//! or `,` as the decimal mark), `+ - * / × ÷ ^`, parentheses, `N%` and
//! `N% of M`, and nothing else: no names, functions or variables, so a query
//! can never become anything but a number. Input length, nesting depth and the
//! size of the result are bounded.

/// Longer queries are search text, not arithmetic.
pub const MAX_INPUT: usize = 128;
/// Parentheses and unary signs deeper than this are rejected.
const MAX_DEPTH: usize = 32;
/// Results beyond this magnitude are rejected rather than shown rounded.
const MAX_MAGNITUDE: f64 = 1e15;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Token {
    Number(f64),
    Plus,
    Minus,
    Times,
    Divide,
    Power,
    Percent,
    Of,
    Open,
    Close,
}

fn tokens(input: &str) -> Option<Vec<Token>> {
    let mut out = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        match ch {
            ' ' | '\t' => i += 1,
            '0'..='9' | '.' | ',' => {
                let start = i;
                while i < chars.len()
                    && (chars[i].is_ascii_digit() || matches!(chars[i], '.' | ','))
                {
                    i += 1;
                }
                let text: String = chars[start..i]
                    .iter()
                    .map(|&ch| if ch == ',' { '.' } else { ch })
                    .collect();
                if text.matches('.').count() > 1 || text == "." {
                    return None;
                }
                out.push(Token::Number(text.parse().ok()?));
            }
            '+' => {
                out.push(Token::Plus);
                i += 1;
            }
            '-' | '−' => {
                out.push(Token::Minus);
                i += 1;
            }
            '*' | '×' | '·' => {
                out.push(Token::Times);
                i += 1;
            }
            '/' | '÷' => {
                out.push(Token::Divide);
                i += 1;
            }
            '^' => {
                out.push(Token::Power);
                i += 1;
            }
            '%' => {
                out.push(Token::Percent);
                i += 1;
            }
            '(' => {
                out.push(Token::Open);
                i += 1;
            }
            ')' => {
                out.push(Token::Close);
                i += 1;
            }
            'o' | 'O' if chars.get(i + 1).is_some_and(|ch| matches!(ch, 'f' | 'F')) => {
                out.push(Token::Of);
                i += 2;
            }
            _ => return None,
        }
    }
    Some(out)
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
    depth: usize,
    /// Whether an operator was used: a lone number is not a calculation.
    operated: bool,
}

impl Parser {
    fn peek(&self) -> Option<Token> {
        self.tokens.get(self.at).copied()
    }

    fn take(&mut self, token: Token) -> bool {
        if self.peek() == Some(token) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn enter(&mut self) -> Option<()> {
        self.depth += 1;
        (self.depth <= MAX_DEPTH).then_some(())
    }

    fn expr(&mut self) -> Option<f64> {
        let mut value = self.term()?;
        loop {
            if self.take(Token::Plus) {
                self.operated = true;
                value += self.term()?;
            } else if self.take(Token::Minus) {
                self.operated = true;
                value -= self.term()?;
            } else {
                return Some(value);
            }
        }
    }

    fn term(&mut self) -> Option<f64> {
        let mut value = self.unary()?;
        loop {
            if self.take(Token::Times) {
                self.operated = true;
                value *= self.unary()?;
            } else if self.take(Token::Divide) {
                self.operated = true;
                let divisor = self.unary()?;
                if divisor == 0.0 {
                    return None;
                }
                value /= divisor;
            } else {
                return Some(value);
            }
        }
    }

    /// A sign binds looser than `^`: `-2^2` is `-(2^2)`.
    fn unary(&mut self) -> Option<f64> {
        if self.take(Token::Minus) {
            self.enter()?;
            let value = -self.unary()?;
            self.depth -= 1;
            return Some(value);
        }
        if self.take(Token::Plus) {
            self.enter()?;
            let value = self.unary()?;
            self.depth -= 1;
            return Some(value);
        }
        self.power()
    }

    /// `^` is right associative and its exponent may carry a sign.
    fn power(&mut self) -> Option<f64> {
        let base = self.percent()?;
        if self.take(Token::Power) {
            self.operated = true;
            self.enter()?;
            let exponent = self.unary()?;
            self.depth -= 1;
            return Some(base.powf(exponent));
        }
        Some(base)
    }

    fn percent(&mut self) -> Option<f64> {
        let value = self.primary()?;
        if self.take(Token::Percent) {
            self.operated = true;
            let share = value / 100.0;
            if self.take(Token::Of) {
                return Some(share * self.unary()?);
            }
            return Some(share);
        }
        Some(value)
    }

    fn primary(&mut self) -> Option<f64> {
        match self.peek()? {
            Token::Number(value) => {
                self.at += 1;
                Some(value)
            }
            Token::Open => {
                self.at += 1;
                self.enter()?;
                let value = self.expr()?;
                self.depth -= 1;
                self.take(Token::Close).then_some(value)
            }
            _ => None,
        }
    }
}

/// The value of an arithmetic query, or `None` when the query is not one.
pub fn evaluate(input: &str) -> Option<f64> {
    let input = input.trim();
    if input.is_empty() || input.chars().count() > MAX_INPUT {
        return None;
    }
    let tokens = tokens(input)?;
    let mut parser = Parser {
        tokens,
        at: 0,
        depth: 0,
        operated: false,
    };
    let value = parser.expr()?;
    if parser.at != parser.tokens.len() || !parser.operated {
        return None;
    }
    (value.is_finite() && value.abs() <= MAX_MAGNITUDE).then_some(if value == 0.0 {
        0.0
    } else {
        value
    })
}

/// A result as it is shown and copied: up to ten decimals, no trailing zeros.
pub fn format(value: f64) -> String {
    let text = format!("{value:.10}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".into()
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calc(input: &str) -> Option<String> {
        evaluate(input).map(format)
    }

    #[test]
    fn arithmetic_follows_precedence() {
        assert_eq!(calc("2+2*3").as_deref(), Some("8"));
        assert_eq!(calc("(2+2)*3").as_deref(), Some("12"));
        assert_eq!(calc("(1920/1080)").as_deref(), Some("1.7777777778"));
        assert_eq!(
            calc("2^3^2").as_deref(),
            Some("512"),
            "power is right associative"
        );
        assert_eq!(calc("-2^2").as_deref(), Some("-4"));
        assert_eq!(calc("10 - 4 - 3").as_deref(), Some("3"));
        assert_eq!(calc("7 ÷ 2 × 4").as_deref(), Some("14"));
        assert_eq!(calc("1,5*2").as_deref(), Some("3"), "decimal comma");
        assert_eq!(calc("0.1+0.2").as_deref(), Some("0.3"));
    }

    #[test]
    fn percentages() {
        assert_eq!(calc("15% of 240").as_deref(), Some("36"));
        assert_eq!(calc("15 % OF 240").as_deref(), Some("36"));
        assert_eq!(calc("50%").as_deref(), Some("0.5"));
        assert_eq!(calc("200 * 15%").as_deref(), Some("30"));
    }

    #[test]
    fn anything_else_is_not_a_calculation() {
        for input in [
            "",
            "42",
            "(5)",
            "-3",
            "firefox",
            "2+",
            "2**3",
            "1.2.3",
            "sqrt(4)",
            "2+2; rm -rf ~",
            "$(id)",
            "1e5+1",
            "0x10+1",
            "1/0",
            "(1+2",
            "1+2)",
            "of 5",
        ] {
            assert_eq!(evaluate(input), None, "{input:?}");
        }
    }

    #[test]
    fn length_depth_and_overflow_are_bounded() {
        let long = format!("1{}", "+1".repeat(MAX_INPUT));
        assert_eq!(evaluate(&long), None, "too long");
        let fits = format!("1{}", "+1".repeat(50));
        assert_eq!(calc(&fits).as_deref(), Some("51"));
        let deep = format!("{}1+1{}", "(".repeat(40), ")".repeat(40));
        assert_eq!(evaluate(&deep), None, "too deep");
        let unary = format!("{}1+1", "-".repeat(60));
        assert_eq!(evaluate(&unary), None, "too many signs");
        assert_eq!(evaluate("10^400"), None, "infinite");
        assert_eq!(evaluate("10^16"), None, "beyond the shown precision");
        assert_eq!(calc("10^15").as_deref(), Some("1000000000000000"));
        assert_eq!(evaluate("99999999999999999999*9"), None);
    }
}
