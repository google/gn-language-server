// Copyright 2025 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use itertools::Itertools;
use pest::{
    iterators::Pair,
    pratt_parser::{Assoc, Op, PrattParser},
    Parser,
};

use crate::parser::*;

#[derive(pest_derive::Parser)]
#[grammar = "parser/gn.pest"]
struct GnParser;

fn rule_to_binary_op(rule: Rule) -> BinaryOp {
    match rule {
        Rule::add => BinaryOp::Add,
        Rule::sub => BinaryOp::Sub,
        Rule::ge => BinaryOp::Ge,
        Rule::gt => BinaryOp::Gt,
        Rule::le => BinaryOp::Le,
        Rule::lt => BinaryOp::Lt,
        Rule::eq => BinaryOp::Eq,
        Rule::ne => BinaryOp::Ne,
        Rule::and => BinaryOp::And,
        Rule::or => BinaryOp::Or,
        _ => unreachable!(),
    }
}

fn convert_identifier(pair: Pair<Rule>) -> Identifier {
    assert!(matches!(pair.as_rule(), Rule::identifier));
    Identifier {
        name: pair.as_str(),
        span: pair.as_span(),
    }
}

fn convert_integer(pair: Pair<Rule>) -> IntegerLiteral {
    assert!(matches!(pair.as_rule(), Rule::integer));
    IntegerLiteral {
        value: pair.as_str().parse().unwrap(),
        span: pair.as_span(),
    }
}

fn convert_string(pair: Pair<Rule>) -> StringLiteral {
    assert!(matches!(pair.as_rule(), Rule::string));
    let span = pair.as_span();
    let pair = pair.into_inner().exactly_one().unwrap();
    assert!(matches!(pair.as_rule(), Rule::string_content));
    let raw_value = pair.as_str();
    let embedded_exprs = pair
        .into_inner()
        .map(|pair| match pair.as_rule() {
            Rule::embedded_expr => convert_expr(pair.into_inner().exactly_one().unwrap()),
            Rule::embedded_identifier => Expr::Primary(Box::new(PrimaryExpr::Identifier(
                Box::new(convert_identifier(
                    pair.into_inner()
                        .exactly_one()
                        .unwrap()
                        .into_inner()
                        .exactly_one()
                        .unwrap(),
                )),
            ))),
            _ => unreachable!(),
        })
        .collect();
    StringLiteral {
        raw_value,
        embedded_exprs,
        span,
    }
}

fn convert_list(pair: Pair<Rule>) -> ListLiteral {
    assert!(matches!(pair.as_rule(), Rule::list));
    let span = pair.as_span();
    let pair = pair.into_inner().exactly_one().unwrap();
    let values = convert_expr_list(pair);
    ListLiteral { values, span }
}

fn convert_array_access(pair: Pair<Rule>) -> ArrayAccess {
    assert!(matches!(pair.as_rule(), Rule::array_access));
    let span = pair.as_span();
    let (array_pair, index_pair) = pair.into_inner().collect_tuple().unwrap();
    let array = convert_identifier(array_pair);
    let index = convert_expr(index_pair);
    ArrayAccess {
        array,
        index: Box::new(index),
        span,
    }
}

fn convert_scope_access(pair: Pair<Rule>) -> ScopeAccess {
    assert!(matches!(pair.as_rule(), Rule::scope_access));
    let span = pair.as_span();
    let (scope_pair, member_pair) = pair.into_inner().collect_tuple().unwrap();
    let scope = convert_identifier(scope_pair);
    let member = convert_identifier(member_pair);
    ScopeAccess {
        scope,
        member,
        span,
    }
}

fn convert_block(pair: Pair<Rule>) -> Block {
    assert!(matches!(pair.as_rule(), Rule::block));
    let span = pair.as_span();
    let mut comments = Comments::default();
    let statements = pair
        .into_inner()
        .filter_map(|pair| match pair.as_rule() {
            Rule::statement => Some(convert_statement(pair, std::mem::take(&mut comments))),
            Rule::error => Some(Statement::Error(Box::new(
                ErrorStatement::UnknownStatement(Box::new(UnknownStatement {
                    text: pair.as_str(),
                    span: pair.as_span(),
                })),
            ))),
            Rule::comment => {
                comments
                    .lines
                    .push(pair.into_inner().exactly_one().unwrap().as_str());
                None
            }
            _ => unreachable!(),
        })
        .collect();
    Block { statements, span }
}

fn convert_primary(pair: Pair<Rule>) -> PrimaryExpr {
    match pair.as_rule() {
        Rule::identifier => PrimaryExpr::Identifier(Box::new(convert_identifier(pair))),
        Rule::integer => PrimaryExpr::Integer(Box::new(convert_integer(pair))),
        Rule::string => PrimaryExpr::String(Box::new(convert_string(pair))),
        Rule::call => PrimaryExpr::Call(Box::new(convert_call(pair, Comments::default()))),
        Rule::array_access => PrimaryExpr::ArrayAccess(Box::new(convert_array_access(pair))),
        Rule::scope_access => PrimaryExpr::ScopeAccess(Box::new(convert_scope_access(pair))),
        Rule::block => PrimaryExpr::Block(Box::new(convert_block(pair))),
        Rule::paren_expr => PrimaryExpr::ParenExpr(Box::new(convert_paren_expr(pair))),
        Rule::list => PrimaryExpr::List(Box::new(convert_list(pair))),
        _ => unreachable!(),
    }
}

fn convert_expr(pair: Pair<Rule>) -> Expr {
    assert!(matches!(pair.as_rule(), Rule::expr));
    let pairs = pair.into_inner();

    // TODO: Cache PrattParser
    let pratt_parser = PrattParser::new()
        .op(Op::prefix(Rule::not))
        .op(Op::infix(Rule::add, Assoc::Left) | Op::infix(Rule::sub, Assoc::Left))
        .op(Op::infix(Rule::ge, Assoc::Left)
            | Op::infix(Rule::gt, Assoc::Left)
            | Op::infix(Rule::le, Assoc::Left)
            | Op::infix(Rule::lt, Assoc::Left))
        .op(Op::infix(Rule::eq, Assoc::Left) | Op::infix(Rule::ne, Assoc::Left))
        .op(Op::infix(Rule::and, Assoc::Left))
        .op(Op::infix(Rule::or, Assoc::Left));
    pratt_parser
        .map_primary(|pair| Expr::Primary(Box::new(convert_primary(pair))))
        .map_prefix(|op, rhs| {
            let span = Span::new(
                op.as_span().get_input(),
                op.as_span().start(),
                rhs.span().end(),
            )
            .unwrap();
            match op.as_rule() {
                Rule::not => Expr::Unary(Box::new(UnaryExpr {
                    op: UnaryOp::Not,
                    expr: Box::new(rhs),
                    span,
                })),
                _ => unreachable!(),
            }
        })
        .map_infix(|lhs, op, rhs| {
            let span =
                Span::new(lhs.span().get_input(), lhs.span().start(), rhs.span().end()).unwrap();
            Expr::Binary(Box::new(BinaryExpr {
                lhs: Box::new(lhs),
                op: rule_to_binary_op(op.as_rule()),
                rhs: Box::new(rhs),
                span,
            }))
        })
        .parse(pairs)
}

fn convert_paren_expr(pair: Pair<Rule>) -> ParenExpr {
    assert!(matches!(pair.as_rule(), Rule::paren_expr));
    let span = pair.as_span();
    let expr = convert_expr(pair.into_inner().exactly_one().unwrap());
    ParenExpr {
        expr: Box::new(expr),
        span,
    }
}

fn convert_expr_list(pair: Pair<Rule>) -> Vec<Expr> {
    assert!(matches!(pair.as_rule(), Rule::expr_list));
    let mut iter = pair.into_inner();
    let Some(first_pair) = iter.next() else {
        return Vec::new();
    };
    let mut exprs = vec![convert_expr(first_pair)];
    iter.tuples().for_each(|(comma_pair, expr_pair)| {
        if comma_pair.as_str().is_empty() {
            let pos = exprs.last().unwrap().span().end_pos();
            exprs.push(Expr::Primary(Box::new(PrimaryExpr::Error(Box::new(
                ErrorPrimaryExpr::MissingComma(Box::new(MissingComma {
                    span: pos.span(&pos),
                })),
            )))));
        }
        exprs.push(convert_expr(expr_pair));
    });
    exprs
}

fn convert_lvalue(pair: Pair<Rule>) -> LValue {
    assert!(matches!(pair.as_rule(), Rule::lvalue));
    let pair = pair.into_inner().exactly_one().unwrap();
    match pair.as_rule() {
        Rule::identifier => LValue::Identifier(Box::new(convert_identifier(pair))),
        Rule::array_access => LValue::ArrayAccess(Box::new(convert_array_access(pair))),
        Rule::scope_access => LValue::ScopeAccess(Box::new(convert_scope_access(pair))),
        _ => unreachable!(),
    }
}

fn convert_assign_op(pair: Pair<Rule>) -> AssignOp {
    assert!(matches!(pair.as_rule(), Rule::assign_op));
    match pair.as_str() {
        "=" => AssignOp::Assign,
        "+=" => AssignOp::AddAssign,
        "-=" => AssignOp::SubAssign,
        _ => unreachable!(),
    }
}

fn convert_assignment<'i>(pair: Pair<'i, Rule>, comments: Comments<'i>) -> Assignment<'i> {
    assert!(matches!(pair.as_rule(), Rule::assignment));
    let span = pair.as_span();
    let (lvalue_pair, assign_op_pair, expr_pair) = pair.into_inner().collect_tuple().unwrap();
    let lvalue = convert_lvalue(lvalue_pair);
    let assign_op = convert_assign_op(assign_op_pair);
    let expr = convert_expr(expr_pair);
    Assignment {
        lvalue,
        op: assign_op,
        rvalue: Box::new(expr),
        comments,
        span,
    }
}

fn convert_call<'i>(pair: Pair<'i, Rule>, comments: Comments<'i>) -> Call<'i> {
    assert!(matches!(pair.as_rule(), Rule::call));
    let span = pair.as_span();
    let mut pairs = pair.into_inner();
    let function = convert_identifier(pairs.next().unwrap());
    let args = convert_expr_list(pairs.next().unwrap());
    let block = pairs.next().map(convert_block);
    Call {
        function,
        args,
        block,
        comments,
        span,
    }
}

fn convert_condition(pair: Pair<Rule>) -> Condition {
    assert!(matches!(pair.as_rule(), Rule::condition));
    let span = pair.as_span();
    let mut pairs = pair.into_inner();

    let condition = convert_expr(pairs.next().unwrap());
    let then_block = convert_block(pairs.next().unwrap());
    let else_block = match pairs.next() {
        Some(pair) if matches!(pair.as_rule(), Rule::condition) => {
            Some(Either::Left(Box::new(convert_condition(pair))))
        }
        Some(pair) if matches!(pair.as_rule(), Rule::block) => {
            Some(Either::Right(Box::new(convert_block(pair))))
        }
        Some(pair) => unreachable!("{:?}", pair),
        None => None,
    };

    Condition {
        condition: Box::new(condition),
        then_block,
        else_block,
        span,
    }
}

fn convert_statement<'i>(pair: Pair<'i, Rule>, comments: Comments<'i>) -> Statement<'i> {
    assert!(matches!(pair.as_rule(), Rule::statement));
    let pair = pair.into_inner().exactly_one().unwrap();
    match pair.as_rule() {
        Rule::assignment => Statement::Assignment(Box::new(convert_assignment(pair, comments))),
        Rule::call => Statement::Call(Box::new(convert_call(pair, comments))),
        Rule::condition => Statement::Condition(Box::new(convert_condition(pair))),
        _ => unreachable!(),
    }
}

fn convert_file(pair: Pair<Rule>) -> Block {
    assert!(matches!(pair.as_rule(), Rule::file));
    let span = pair.as_span();
    let mut comments = Comments::default();
    let statements = pair
        .into_inner()
        .filter_map(|pair| match pair.as_rule() {
            Rule::statement => Some(convert_statement(pair, std::mem::take(&mut comments))),
            Rule::error => Some(Statement::Error(Box::new(
                ErrorStatement::UnknownStatement(Box::new(UnknownStatement {
                    text: pair.as_str(),
                    span: pair.as_span(),
                })),
            ))),
            Rule::unmatched_brace => Some(Statement::Error(Box::new(
                ErrorStatement::UnmatchedBrace(Box::new(UnmatchedBrace {
                    span: pair.as_span(),
                })),
            ))),
            Rule::comment => {
                comments
                    .lines
                    .push(pair.into_inner().exactly_one().unwrap().as_str());
                None
            }
            Rule::EOI => None,
            _ => unreachable!(),
        })
        .collect();
    Block { statements, span }
}

pub fn parse(input: &str) -> Block<'_> {
    let file_pair = GnParser::parse(Rule::file, input)
        .unwrap()
        .exactly_one()
        .unwrap();
    convert_file(file_pair)
}
