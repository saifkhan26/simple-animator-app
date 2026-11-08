# `src/ui/expr.rs` — arithmetic in number fields

> **Source:** [src/ui/expr.rs](../../../src/ui/expr.rs) · **Module:** `crate::ui::expr` · **Size:** 221 lines · Line links as of commit `67a7cb9`

## What this file is for

Lets numeric text fields take small expressions: the timeline's frame box accepts `22/2` or `8+12` instead of a
bare number. A **leading operator** is relative to the field's current value: `+12` jumps 12 frames forward,
`*2` doubles it.

It's a tiny **recursive-descent parser** — the same technique you'd use to write a calculator in JS:

```text
expr    := term (('+' | '-') term)*
term    := unary (('*' | '/') unary)*
unary   := '-' unary | primary
primary := number | '(' expr ')'
```

Each grammar rule is one method; higher-precedence rules are called from lower ones, so `2+3*4` parses as
`2 + (3*4)`.

**Example.**
```rust
eval("22/2", 0.0)    // Some(11.0)
eval("+12", 30.0)    // Some(42.0)  — relative to the current 30
eval("+12*2", 30.0)  // Some(54.0)  — 30 + 12*2, multiplication binds tighter
eval("abc", 30.0)    // None        — leave the field unchanged
```

## How it connects

- **Used by:** [ui/shell.rs](shell.md) (frame number box and other numeric fields).

## Rust concepts in this file

**A parser struct over a char slice.** `Parser<'a> { s: &'a [char], i: usize }` walks a borrowed array of characters
with an index, like `const s = [...input]; let i = 0;` in JS. The lifetime says it borrows the characters.
More: [Lifetimes](../../rust-primer.md#lifetimes)

**`Option` chaining with `?`.** Each parse method returns `Option<f64>`; `let v = self.term(seed)?;` stops at the
first syntax error. More: [Option and Result](../../rust-primer.md#option-and-result)

## Types

### `struct Parser`
[src/ui/expr.rs:41](../../../src/ui/expr.rs#L41)

The expression parser: the input as characters and a cursor position.

| Field | Type | Meaning |
|---|---|---|
| `s` | `&'a [char]` | The input characters. |
| `i` | `usize` | Current position. |

## Free functions

### `eval`
[src/ui/expr.rs:25](../../../src/ui/expr.rs#L25)

```rust
pub fn eval(input: &str, current: f64) -> Option<f64>
```

Evaluate `input`. `current` is the field's present value, used as the left operand when the input starts with an operator. `None` = not a valid expression; the caller should leave the value alone.

## `impl Parser<'_>`

### `Parser::peek`
[src/ui/expr.rs:47](../../../src/ui/expr.rs#L47)

```rust
fn peek(&self) -> Option<char>
```

The next character without consuming it.

### `Parser::bump`
[src/ui/expr.rs:51](../../../src/ui/expr.rs#L51)

```rust
fn bump(&mut self)
```

Consumes one character.

### `Parser::skip_ws`
[src/ui/expr.rs:55](../../../src/ui/expr.rs#L55)

```rust
fn skip_ws(&mut self)
```

Skips spaces.

### `Parser::expr`
[src/ui/expr.rs:65](../../../src/ui/expr.rs#L65)

```rust
fn expr(&mut self, seed: Option<f64>) -> Option<f64>
```

`seed` stands in for the first operand when the input opened with an operator. It is threaded down to `Parser::term` so that a leading `*` or `/` binds at the right precedence: `+12*2` is `current + 24`, while `*2` is `current * 2`.

### `Parser::term`
[src/ui/expr.rs:83](../../../src/ui/expr.rs#L83)

```rust
fn term(&mut self, seed: Option<f64>) -> Option<f64>
```

Parses `unary (('*' | '/') unary)*`, starting from `seed` when the input opened with `*` or `/`.

### `Parser::unary`
[src/ui/expr.rs:107](../../../src/ui/expr.rs#L107)

```rust
fn unary(&mut self) -> Option<f64>
```

Parses an optional unary minus, then a primary.

### `Parser::primary`
[src/ui/expr.rs:122](../../../src/ui/expr.rs#L122)

```rust
fn primary(&mut self) -> Option<f64>
```

A number or a parenthesised expression.

### `Parser::number`
[src/ui/expr.rs:140](../../../src/ui/expr.rs#L140)

```rust
fn number(&mut self) -> Option<f64>
```

Reads digits and an optional decimal point into a number.

## Tests

### `mod tests`

Run with `cargo test ui::expr::tests` · 8 tests

| Test | What it checks |
|---|---|
| [`evaluates_a_full_expression_against_the_current_frame`](../../../src/ui/expr.rs#L161) | Evaluates a full expression against the current frame. |
| [`a_leading_operator_is_relative_to_the_current_value`](../../../src/ui/expr.rs#L169) | A leading operator is relative to the current value. |
| [`a_relative_expression_keeps_normal_precedence`](../../../src/ui/expr.rs#L177) | A relative expression keeps normal precedence. |
| [`absolute_input_ignores_the_current_value`](../../../src/ui/expr.rs#L184) | Absolute input ignores the current value. |
| [`precedence_parens_and_whitespace`](../../../src/ui/expr.rs#L190) | Precedence parens and whitespace. |
| [`decimals_are_kept_unrounded_for_the_caller`](../../../src/ui/expr.rs#L199) | Decimals are kept unrounded for the caller. |
| [`sizes_scale_from_the_current_value_or_from_scratch`](../../../src/ui/expr.rs#L206) | The layer canvas size fields: triple a 4K width, either way round. |
| [`junk_is_rejected_so_the_field_keeps_its_value`](../../../src/ui/expr.rs#L213) | Junk is rejected so the field keeps its value. |
