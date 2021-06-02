use crate::chunk::*;
use crate::scanner::*;
use crate::string;
use crate::value::*;

#[derive(Copy, Clone, PartialOrd, PartialEq)]
#[repr(u8)]
enum Precedence {
  None,
  Assignment, // =
  Or,         // or
  And,        // and
  Equality,   // == !=
  Comparison, // < > <= >=
  Term,       // + -
  Factor,     // * /
  Unary,      // ! -
  Call,       // . ()
  Primary,
}

impl Precedence {
  fn higher(&self) -> Precedence {
    match self {
      Precedence::None => Precedence::Assignment,
      Precedence::Assignment => Precedence::Or,
      Precedence::Or => Precedence::And,
      Precedence::And => Precedence::Equality,
      Precedence::Equality => Precedence::Comparison,
      Precedence::Comparison => Precedence::Term,
      Precedence::Term => Precedence::Factor,
      Precedence::Factor => Precedence::Unary,
      Precedence::Unary => Precedence::Call,
      _ => Precedence::Primary,
    }
  }
}

enum ErrorLocation {
  Current,
  Previous,
}

type ParseFn<'a> = fn(&mut Compiler<'a>, bool);
type ParseRule<'a> = (Option<ParseFn<'a>>, Option<ParseFn<'a>>, Precedence);

struct Parser<'a> {
  previous: Option<Token<'a>>,
  current: Option<Token<'a>>,
  had_error: bool,
  panic_mode: bool,
}

pub struct Compiler<'a> {
  parser: Parser<'a>,
  scanner: Scanner<'a>,
  chunk: &'a mut Chunk,
}

impl<'a> Compiler<'a> {
  fn get_rule(kind: TokenKind) -> ParseRule<'a> {
    match kind {
      TokenKind::LeftParen => (Some(Self::grouping), None, Precedence::None),
      TokenKind::Minus => (Some(Self::unary), Some(Self::binary), Precedence::Term),
      TokenKind::Plus => (None, Some(Self::binary), Precedence::Term),
      TokenKind::Slash => (None, Some(Self::binary), Precedence::Factor),
      TokenKind::Star => (None, Some(Self::binary), Precedence::Factor),
      TokenKind::Bang => (Some(Self::unary), None, Precedence::None),
      TokenKind::BangEqual => (None, Some(Self::binary), Precedence::Equality),
      TokenKind::EqualEqual => (None, Some(Self::binary), Precedence::Equality),
      TokenKind::Greater => (None, Some(Self::binary), Precedence::Comparison),
      TokenKind::GreaterEqual => (None, Some(Self::binary), Precedence::Comparison),
      TokenKind::Less => (None, Some(Self::binary), Precedence::Comparison),
      TokenKind::LessEqual => (None, Some(Self::binary), Precedence::Comparison),
      TokenKind::Identifier => (Some(Self::variable), None, Precedence::None),
      TokenKind::String => (Some(Self::string), None, Precedence::None),
      TokenKind::Number => (Some(Self::number), None, Precedence::None),
      TokenKind::False => (Some(Self::literal), None, Precedence::None),
      TokenKind::True => (Some(Self::literal), None, Precedence::None),
      TokenKind::Nil => (Some(Self::literal), None, Precedence::None),
      _ => (None, None, Precedence::None),
    }
  }

  pub fn new(scanner: Scanner<'a>, chunk: &'a mut Chunk) -> Compiler<'a> {
    Compiler {
      parser: Parser {
        previous: None,
        current: None,
        had_error: false,
        panic_mode: false,
      },
      scanner: scanner,
      chunk: chunk,
    }
  }

  fn current_kind(&self) -> TokenKind {
    self.parser.current.as_ref().unwrap().kind
  }

  fn previous_kind(&self) -> TokenKind {
    self.parser.previous.as_ref().unwrap().kind
  }

  fn error_at(&mut self, location: ErrorLocation, message: &str) {
    if self.parser.panic_mode {
      return;
    }

    let token = match location {
      ErrorLocation::Current => self.parser.current.as_ref().unwrap(),
      ErrorLocation::Previous => self.parser.previous.as_ref().unwrap(),
    };

    eprint!("[line {}] Error", token.line);

    if token.kind == TokenKind::EOF {
      eprint!(" at end");
    } else if token.kind != TokenKind::Error {
      eprint!(" at '{}'", token.lexeme);
    }

    eprintln!(": {}", message);
    drop(token);
    self.parser.panic_mode = true;
    self.parser.had_error = true;
  }

  fn error_at_current(&mut self, message: &str) {
    self.error_at(ErrorLocation::Current, message)
  }

  fn error(&mut self, message: &str) {
    self.error_at(ErrorLocation::Previous, message)
  }

  fn advance(&mut self) {
    self.parser.previous = std::mem::take(&mut self.parser.current);

    loop {
      let token = self.scanner.scan_token();
      self.parser.current = Some(token);
      let token = self.parser.current.as_ref().unwrap();
      if token.kind != TokenKind::Error {
        break;
      }

      self.error_at_current(&self.parser.current.as_ref().unwrap().lexeme);
    }
  }

  fn consume(&mut self, kind: TokenKind, message: &str) {
    if self.current_kind() == kind {
      self.advance();
      return;
    }

    self.error_at_current(message);
  }

  fn check(&self, kind: TokenKind) -> bool {
    self.current_kind() == kind
  }

  fn match_current(&mut self, kind: TokenKind) -> bool {
    if !self.check(kind) {
      return false;
    }

    self.advance();
    true
  }

  fn emit_byte(&mut self, byte: u8) {
    self
      .chunk
      .write(byte, self.parser.previous.as_ref().unwrap().line);
  }

  fn emit_bytes(&mut self, byte1: u8, byte2: u8) {
    self.emit_byte(byte1);
    self.emit_byte(byte2);
  }

  fn emit_return(&mut self) {
    self.emit_byte(Op::Return as u8);
  }

  fn make_constant(&mut self, value: Value) -> u8 {
    let constant = self.chunk.add_constant(value);
    if constant > std::u8::MAX {
      self.error("Too many constants in one chunk.");
      0
    } else {
      constant as u8
    }
  }

  fn emit_constant(&mut self, value: Value) {
    let constant = self.make_constant(value);
    self.emit_bytes(Op::Constant as u8, constant);
  }

  fn binary(&mut self, _can_assign: bool) {
    let operator_type = self.previous_kind();
    let rule = Self::get_rule(operator_type);
    let precedence = rule.2.higher();
    self.parse_precedence(precedence);

    match operator_type {
      TokenKind::BangEqual => self.emit_bytes(Op::Equal as u8, Op::Not as u8),
      TokenKind::EqualEqual => self.emit_byte(Op::Equal as u8),
      TokenKind::Greater => self.emit_byte(Op::Greater as u8),
      TokenKind::GreaterEqual => self.emit_bytes(Op::Less as u8, Op::Not as u8),
      TokenKind::Less => self.emit_byte(Op::Less as u8),
      TokenKind::LessEqual => self.emit_bytes(Op::Greater as u8, Op::Not as u8),
      TokenKind::Plus => self.emit_byte(Op::Add as u8),
      TokenKind::Minus => self.emit_byte(Op::Subtract as u8),
      TokenKind::Star => self.emit_byte(Op::Multiply as u8),
      TokenKind::Slash => self.emit_byte(Op::Divide as u8),
      _ => unreachable!(),
    }
  }

  fn literal(&mut self, _can_assign: bool) {
    match self.previous_kind() {
      TokenKind::False => self.emit_byte(Op::False as u8),
      TokenKind::Nil => self.emit_byte(Op::Nil as u8),
      TokenKind::True => self.emit_byte(Op::True as u8),
      _ => (),
    }
  }

  fn grouping(&mut self, _can_assign: bool) {
    self.expression();
    self.consume(TokenKind::RightParen, "Expect ')' after expression.")
  }

  fn string(&mut self, _can_assign: bool) {
    let string = String::from(self.parser.previous.as_ref().unwrap().lexeme);

    self.emit_constant(Value::String(string::Handle::from_str(
      &string[1..string.len() - 1],
    )))
  }

  fn named_variable(&mut self, name: &str, can_assign: bool) {
    let arg = self.identifier_constant(name);

    if can_assign && self.match_current(TokenKind::Equal) {
      self.expression();
      self.emit_bytes(Op::SetGlobal as u8, arg);
    } else {
      self.emit_bytes(Op::GetGlobal as u8, arg);
    }
  }

  fn variable(&mut self, can_assign: bool) {
    self.named_variable(self.parser.previous.as_ref().unwrap().lexeme, can_assign)
  }

  fn number(&mut self, _can_assign: bool) {
    let value: f64 = self
      .parser
      .previous
      .as_ref()
      .unwrap()
      .lexeme
      .parse()
      .expect("Failed to parse string into float");

    self.emit_constant(Value::Number(value));
  }

  fn unary(&mut self, _can_assign: bool) {
    let operator_type = self.previous_kind();

    self.parse_precedence(Precedence::Unary);

    match operator_type {
      TokenKind::Minus => self.emit_byte(Op::Negate as u8),
      TokenKind::Bang => self.emit_byte(Op::Not as u8),
      _ => unreachable!(),
    }
  }

  fn parse_precedence(&mut self, precedence: Precedence) {
    self.advance();
    let prefix_rule = Self::get_rule(self.previous_kind()).0;
    if prefix_rule.is_none() {
      self.error("Expect expression.");
      return;
    }

    let can_assign = precedence <= Precedence::Assignment;
    prefix_rule.unwrap()(self, can_assign);

    while precedence <= Self::get_rule(self.current_kind()).2 {
      self.advance();
      if let Some(infix_rule) = Self::get_rule(self.previous_kind()).1 {
        infix_rule(self, can_assign);
      }
    }

    if can_assign && self.match_current(TokenKind::Equal) {
      self.error("Invalid assignment target.");
    }
  }

  fn identifier_constant(&mut self, name: &str) -> u8 {
    self.make_constant(Value::String(string::Handle::from_str(name)))
  }

  fn parse_variable(&mut self, message: &str) -> u8 {
    self.consume(TokenKind::Identifier, message);
    return self.identifier_constant(self.parser.previous.as_ref().unwrap().lexeme);
  }

  fn define_variable(&mut self, global: u8) {
    self.emit_bytes(Op::DefineGlobal as u8, global)
  }

  fn expression(&mut self) {
    self.parse_precedence(Precedence::Assignment);
  }

  fn var_declaration(&mut self) {
    let global = self.parse_variable("Expect variable name.");

    if self.match_current(TokenKind::Equal) {
      self.expression();
    } else {
      self.emit_byte(Op::Nil as u8);
    }

    self.consume(
      TokenKind::Semicolon,
      "Expect ';' after variable declaration.",
    );
    self.define_variable(global);
  }

  fn expression_statement(&mut self) {
    self.expression();
    self.consume(TokenKind::Semicolon, "Expect ';' after expression.");
    self.emit_byte(Op::Pop as u8)
  }

  fn print_statement(&mut self) {
    self.expression();
    self.consume(TokenKind::Semicolon, "Expect ';' after value.");
    self.emit_byte(Op::Print as u8)
  }

  fn synchronize(&mut self) {
    loop {
      if self.previous_kind() == TokenKind::Semicolon {
        return;
      }
      match self.current_kind() {
        TokenKind::EOF => return,
        TokenKind::Fun => return,
        TokenKind::Var => return,
        TokenKind::For => return,
        TokenKind::If => return,
        TokenKind::While => return,
        TokenKind::Print => return,
        TokenKind::Return => return,
        _ => self.advance(),
      }
    }
  }

  fn declaration(&mut self) {
    if self.match_current(TokenKind::Var) {
      self.var_declaration();
    } else {
      self.statement();
    }

    if self.parser.panic_mode {
      self.synchronize();
    }
  }

  fn statement(&mut self) {
    if self.match_current(TokenKind::Print) {
      self.print_statement();
    } else {
      self.expression_statement();
    }
  }

  fn end_compiler(&mut self) {
    self.emit_return();
    {
      #![cfg(feature = "trace-execution")]
      if !self.parser.had_error {
        self.chunk.disassemble("code");
      }
    }
  }

  fn compile(&mut self) -> bool {
    self.advance();

    while !self.match_current(TokenKind::EOF) {
      self.declaration();
    }

    self.end_compiler();
    !self.parser.had_error
  }
}

pub fn compile(source: &String, chunk: &mut Chunk) -> bool {
  let mut compiler = Compiler::new(Scanner::new(source), chunk);
  compiler.compile()
}
