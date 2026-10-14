use super::Diagnostic;
use qspquest_core::expression::*;
#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f64),
    Name(String),
    Text(String),
    Op(String),
    End,
}
struct Parser {
    tokens: Vec<(Token, usize)>,
    index: usize,
    line: usize,
    column: usize,
    depth: usize,
    nodes: usize,
}
impl Parser {
    fn fail(&self, message: &str) -> Diagnostic {
        Diagnostic::new(
            self.line,
            self.column + self.tokens[self.index].1,
            "expression_syntax",
            message,
            "Use a closed arithmetic, comparison, boolean, or function expression.",
        )
    }
    fn peek(&self, op: &str) -> bool {
        matches!(&self.tokens[self.index].0,Token::Op(s) if s==op)
    }
    fn eat(&mut self, op: &str) -> bool {
        if self.peek(op) {
            self.index += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, op: &str) -> Result<(), Diagnostic> {
        if self.eat(op) {
            Ok(())
        } else {
            Err(self.fail(&format!("Expected '{op}'")))
        }
    }
    fn node(&mut self, e: Expr) -> Result<Expr, Diagnostic> {
        self.nodes += 1;
        if self.nodes > 256 {
            Err(self.fail("Expression exceeds 256 nodes"))
        } else {
            Ok(e)
        }
    }
    fn expression(&mut self) -> Result<Expr, Diagnostic> {
        self.depth += 1;
        if self.depth > 128 {
            return Err(self.fail("Expression parser nesting exceeds 128"));
        }
        let condition = self.boolean(false)?;
        let result = if self.eat("?") {
            let yes = self.expression()?;
            self.expect(":")?;
            let no = self.expression()?;
            self.node(Expr::conditional(condition, yes, no))?
        } else {
            condition
        };
        self.depth -= 1;
        Ok(result)
    }
    fn boolean(&mut self, and: bool) -> Result<Expr, Diagnostic> {
        let mut args = vec![if and {
            self.compare()?
        } else {
            self.boolean(true)?
        }];
        while self.eat(if and { "&&" } else { "||" }) {
            args.push(if and {
                self.compare()?
            } else {
                self.boolean(true)?
            });
        }
        if args.len() == 1 {
            Ok(args.pop().unwrap())
        } else {
            self.node(Expr::Boolean {
                operator: if and { Boolean::And } else { Boolean::Or },
                arguments: args,
            })
        }
    }
    fn compare(&mut self) -> Result<Expr, Diagnostic> {
        let a = self.arithmetic(0)?;
        let op = if self.eat("<") {
            Some(Compare::Lt)
        } else if self.eat("<=") {
            Some(Compare::Le)
        } else if self.eat(">") {
            Some(Compare::Gt)
        } else if self.eat(">=") {
            Some(Compare::Ge)
        } else if self.eat("==") {
            Some(Compare::Eq)
        } else if self.eat("!=") {
            Some(Compare::Ne)
        } else {
            None
        };
        if let Some(operator) = op {
            let b = self.arithmetic(0)?;
            self.node(Expr::Compare {
                operator,
                arguments: Box::new([a, b]),
            })
        } else {
            Ok(a)
        }
    }
    fn arithmetic(&mut self, min: u8) -> Result<Expr, Diagnostic> {
        self.depth += 1;
        if self.depth > 128 {
            return Err(self.fail("Expression parser nesting exceeds 128"));
        }
        let mut left = if self.eat("+") {
            let a = self.arithmetic(5)?;
            self.node(Expr::Unary {
                operator: Unary::Positive,
                arguments: Box::new([a]),
            })?
        } else if self.eat("-") {
            let literal_sign = matches!(self.tokens[self.index].0, Token::Number(_));
            let a = self.arithmetic(5)?;
            match a {
                Expr::Literal {
                    value: Literal::Number(v),
                } if literal_sign => Expr::number(-v),
                other => self.node(Expr::Unary {
                    operator: Unary::Negative,
                    arguments: Box::new([other]),
                })?,
            }
        } else if self.eat("!") {
            let a = self.arithmetic(5)?;
            self.node(Expr::Unary {
                operator: Unary::Not,
                arguments: Box::new([a]),
            })?
        } else {
            self.atom()?
        };
        loop {
            let (operator, l, r) = match &self.tokens[self.index].0 {
                Token::Op(op) => match op.as_str() {
                    "+" => (Binary::Add, 1, 2),
                    "-" => (Binary::Subtract, 1, 2),
                    "*" => (Binary::Multiply, 3, 4),
                    "/" => (Binary::Divide, 3, 4),
                    "%" => (Binary::Modulo, 3, 4),
                    "^" => (Binary::Power, 6, 6),
                    _ => break,
                },
                _ => break,
            };
            if l < min {
                break;
            }
            self.index += 1;
            let right = self.arithmetic(r)?;
            left = self.node(Expr::binary(operator, left, right))?;
        }
        self.depth -= 1;
        Ok(left)
    }
    fn atom(&mut self) -> Result<Expr, Diagnostic> {
        let token = self.tokens[self.index].0.clone();
        match token {
            Token::Number(value) => {
                self.index += 1;
                self.node(Expr::number(value))
            }
            Token::Name(name) => {
                self.index += 1;
                if name == "true" || name == "false" {
                    return self.node(Expr::Literal {
                        value: Literal::Boolean(name == "true"),
                    });
                }
                if !self.eat("(") {
                    return self.node(Expr::symbol(name));
                }
                if name == "quantity" {
                    let sign = if self.eat("-") {
                        -1.
                    } else {
                        self.eat("+");
                        1.
                    };
                    let Token::Number(value) = self.tokens[self.index].0.clone() else {
                        return Err(self.fail("quantity requires a finite numeric constant"));
                    };
                    self.index += 1;
                    self.expect(",")?;
                    let Token::Text(unit) = self.tokens[self.index].0.clone() else {
                        return Err(self.fail("quantity requires a quoted unit"));
                    };
                    qspquest_core::units::Unit::parse(&unit).map_err(|e| self.fail(&e.message))?;
                    self.index += 1;
                    self.expect(")")?;
                    return self.node(Expr::Quantity {
                        quantity: qspquest_core::model::Quantity {
                            value: sign * value,
                            unit,
                        },
                    });
                }
                let function = match name.as_str() {
                    "exp" => Function::Exp,
                    "log" => Function::Log,
                    "log10" => Function::Log10,
                    "sqrt" => Function::Sqrt,
                    "sin" => Function::Sin,
                    "cos" => Function::Cos,
                    "tan" => Function::Tan,
                    "tanh" => Function::Tanh,
                    "abs" => Function::Abs,
                    "min" => Function::Min,
                    "max" => Function::Max,
                    "floor" => Function::Floor,
                    "ceil" => Function::Ceil,
                    "hill" => Function::Hill,
                    "ifelse" => Function::Ifelse,
                    _ => return Err(self.fail("Unknown function")),
                };
                let mut arguments = vec![];
                if !self.eat(")") {
                    loop {
                        arguments.push(self.expression()?);
                        if self.eat(")") {
                            break;
                        }
                        self.expect(",")?;
                    }
                }
                self.node(Expr::call(function, arguments))
            }
            Token::Op(ref op) if op == "(" => {
                self.index += 1;
                let e = self.expression()?;
                self.expect(")")?;
                Ok(e)
            }
            _ => Err(self.fail("Expected a number, symbol, function, or parenthesized expression")),
        }
    }
}
pub fn parse(source: &str, line: usize, column: usize) -> Result<Expr, Diagnostic> {
    let fail = |offset, message: &str| {
        Diagnostic::new(
            line,
            column + offset,
            "expression_syntax",
            message,
            "Use ASCII identifiers and supported expression operators.",
        )
    };
    if source.len() > 2000 {
        return Err(fail(0, "Expression exceeds 2000 bytes"));
    }
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut tokens = vec![];
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        let token = if bytes[i].is_ascii_digit()
            || (bytes[i] == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            while bytes.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
            if bytes.get(i) == Some(&b'.') {
                i += 1;
                while bytes.get(i).is_some_and(u8::is_ascii_digit) {
                    i += 1;
                }
            }
            if bytes.get(i).is_some_and(|c| *c == b'e' || *c == b'E') {
                i += 1;
                if bytes.get(i).is_some_and(|c| *c == b'+' || *c == b'-') {
                    i += 1;
                }
                while bytes.get(i).is_some_and(u8::is_ascii_digit) {
                    i += 1;
                }
            }
            let value = source[start..i]
                .parse::<f64>()
                .map_err(|_| fail(start, "Invalid numeric literal"))?;
            if !value.is_finite() {
                return Err(fail(start, "Numeric literal must be finite"));
            }
            Token::Number(value)
        } else if bytes[i] == b'"' {
            i += 1;
            let mut escaped = false;
            while i < bytes.len() {
                let byte = bytes[i];
                i += 1;
                if byte == b'"' && !escaped {
                    break;
                }
                escaped = byte == b'\\' && !escaped;
            }
            let value = serde_json::from_str::<String>(&source[start..i])
                .map_err(|_| fail(start, "Invalid quoted unit"))?;
            Token::Text(value)
        } else if bytes[i].is_ascii_lowercase() || bytes[i] == b'_' {
            i += 1;
            while bytes
                .get(i)
                .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
            {
                i += 1;
            }
            Token::Name(source[start..i].into())
        } else {
            let len = if i + 1 < bytes.len()
                && matches!(
                    &bytes[i..i + 2],
                    b"<=" | b">=" | b"==" | b"!=" | b"&&" | b"||"
                ) {
                2
            } else {
                1
            };
            if !bytes[i].is_ascii() || !b"+-*/%^!<>=&|(),?:".contains(&bytes[i]) {
                return Err(fail(start, "Unsupported character"));
            }
            i += len;
            Token::Op(source[start..i].into())
        };
        tokens.push((token, start));
    }
    tokens.push((Token::End, source.len()));
    let mut parser = Parser {
        tokens,
        index: 0,
        line,
        column,
        depth: 0,
        nodes: 0,
    };
    let expr = parser.expression()?;
    if parser.tokens[parser.index].0 != Token::End {
        return Err(parser.fail("Unexpected token after expression"));
    }
    fn depth(expr: &Expr) -> usize {
        1 + expr.children().iter().map(depth).max().unwrap_or(0)
    }
    if depth(&expr) > 32 {
        return Err(fail(0, "Expression tree exceeds depth 32"));
    }
    Ok(expr)
}
pub fn format(expr: &Expr) -> String {
    match expr {
        Expr::Quantity { quantity } => format!(
            "quantity({}, {})",
            quantity.value,
            serde_json::to_string(&quantity.unit).expect("unit string serialization")
        ),
        Expr::Literal {
            value: Literal::Number(v),
        } => {
            if *v < 0.0 || v.is_sign_negative() {
                format!("(-{})", -v)
            } else {
                v.to_string()
            }
        }
        Expr::Literal {
            value: Literal::Boolean(v),
        } => v.to_string(),
        Expr::Symbol { name } => name.clone(),
        Expr::Unary {
            operator,
            arguments,
        } => format!(
            "({}({}))",
            match operator {
                Unary::Positive => "+",
                Unary::Negative => "-",
                Unary::Not => "!",
            },
            format(&arguments[0])
        ),
        Expr::Binary {
            operator,
            arguments,
        } => format!(
            "({} {} {})",
            format(&arguments[0]),
            match operator {
                Binary::Add => "+",
                Binary::Subtract => "-",
                Binary::Multiply => "*",
                Binary::Divide => "/",
                Binary::Power => "^",
                Binary::Modulo => "%",
            },
            format(&arguments[1])
        ),
        Expr::Compare {
            operator,
            arguments,
        } => format!(
            "({} {} {})",
            format(&arguments[0]),
            match operator {
                Compare::Lt => "<",
                Compare::Le => "<=",
                Compare::Gt => ">",
                Compare::Ge => ">=",
                Compare::Eq => "==",
                Compare::Ne => "!=",
            },
            format(&arguments[1])
        ),
        Expr::Boolean {
            operator,
            arguments,
        } => format!(
            "({})",
            arguments
                .iter()
                .map(format)
                .collect::<Vec<_>>()
                .join(match operator {
                    Boolean::And => " && ",
                    Boolean::Or => " || ",
                })
        ),
        Expr::Call { name, arguments } => format!(
            "{}({})",
            match name {
                Function::Exp => "exp",
                Function::Log => "log",
                Function::Log10 => "log10",
                Function::Sqrt => "sqrt",
                Function::Sin => "sin",
                Function::Cos => "cos",
                Function::Tan => "tan",
                Function::Tanh => "tanh",
                Function::Abs => "abs",
                Function::Min => "min",
                Function::Max => "max",
                Function::Floor => "floor",
                Function::Ceil => "ceil",
                Function::Hill => "hill",
                Function::Ifelse => "ifelse",
            },
            arguments.iter().map(format).collect::<Vec<_>>().join(", ")
        ),
        Expr::Conditional { arguments } => format!(
            "({} ? {} : {})",
            format(&arguments[0]),
            format(&arguments[1]),
            format(&arguments[2])
        ),
    }
}
