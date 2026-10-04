use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Error, MAX_TEXT_BYTES, Result, Scalar, ScalarType};

pub const MAX_EXPRESSION_DEPTH: usize = 48;

#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Parameter,
    Local,
    Shared,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Variable {
    pub scope: Scope,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Assignment {
    pub target: Variable,
    pub value: Expr,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Expr {
    Literal {
        value: Scalar,
    },
    Read {
        scope: Scope,
        name: String,
    },
    Not {
        value: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BinaryOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    And,
    Or,
}

/// The variables a graph may read: its parameters, its locals and the shared variables it
/// declares.
pub(crate) struct Declarations<'a> {
    pub parameters: &'a BTreeMap<String, ScalarType>,
    pub locals: &'a BTreeMap<String, Scalar>,
    pub shared: &'a BTreeMap<String, ScalarType>,
}

impl Declarations<'_> {
    pub fn variable(&self, scope: Scope, name: &str, path: &str) -> Result<ScalarType> {
        let kind = match scope {
            Scope::Parameter => self.parameters.get(name).copied(),
            Scope::Local => self.locals.get(name).map(Scalar::kind),
            Scope::Shared => self.shared.get(name).copied(),
        };
        kind.ok_or_else(|| {
            Error::new(
                "reference",
                path,
                format!("undeclared {scope:?} variable {name}"),
            )
        })
    }

    pub fn check(&self, expr: &Expr, path: &str) -> Result<ScalarType> {
        self.check_at(expr, 0, path)
    }

    fn check_at(&self, expr: &Expr, depth: usize, path: &str) -> Result<ScalarType> {
        if depth > MAX_EXPRESSION_DEPTH {
            return Err(Error::new("limit", path, "expression nesting exceeds 48"));
        }
        match expr {
            Expr::Literal { value } => {
                check_scalar(value, path)?;
                Ok(value.kind())
            }
            Expr::Read { scope, name } => self.variable(*scope, name, path),
            Expr::Not { value } => {
                expect(
                    self.check_at(value, depth + 1, path)?,
                    ScalarType::Bool,
                    path,
                )?;
                Ok(ScalarType::Bool)
            }
            Expr::Binary { op, left, right } => {
                let left = self.check_at(left, depth + 1, path)?;
                let right = self.check_at(right, depth + 1, path)?;
                expect(right, left, path)?;
                // Only equality is defined for every type, including `ref`.
                match op {
                    BinaryOp::Eq | BinaryOp::Ne => Ok(ScalarType::Bool),
                    BinaryOp::And | BinaryOp::Or => {
                        expect(left, ScalarType::Bool, path)?;
                        Ok(ScalarType::Bool)
                    }
                    BinaryOp::Add | BinaryOp::Sub => {
                        expect(left, ScalarType::Int, path)?;
                        Ok(ScalarType::Int)
                    }
                    BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                        expect(left, ScalarType::Int, path)?;
                        Ok(ScalarType::Bool)
                    }
                }
            }
        }
    }

    pub fn check_assignments(&self, values: &[Assignment], path: &str) -> Result<()> {
        for assignment in values {
            if assignment.target.scope == Scope::Parameter {
                return Err(Error::new(
                    "readonly",
                    path,
                    "parameters are immutable bindings",
                ));
            }
            let expected = self.variable(assignment.target.scope, &assignment.target.name, path)?;
            expect(self.check(&assignment.value, path)?, expected, path)?;
        }
        Ok(())
    }
}

pub(crate) fn expect(actual: ScalarType, expected: ScalarType, path: &str) -> Result<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(Error::new(
            "type",
            path,
            format!("expected {expected:?}, got {actual:?}"),
        ))
    }
}

pub(crate) fn check_scalar(value: &Scalar, path: &str) -> Result<()> {
    match value {
        Scalar::Text(text) if text.len() > MAX_TEXT_BYTES => {
            Err(Error::new("limit", path, "text value exceeds 64 KiB"))
        }
        _ => Ok(()),
    }
}

pub(crate) struct Values<'a> {
    pub parameters: &'a BTreeMap<String, Scalar>,
    pub locals: &'a BTreeMap<String, Scalar>,
    pub shared: &'a BTreeMap<String, Scalar>,
}

impl Values<'_> {
    pub fn read(&self, scope: Scope, name: &str) -> Result<&Scalar> {
        let map = match scope {
            Scope::Parameter => self.parameters,
            Scope::Local => self.locals,
            Scope::Shared => self.shared,
        };
        map.get(name)
            .ok_or_else(|| Error::new("state", name, "missing checked variable"))
    }

    pub fn evaluate(&self, expr: &Expr) -> Result<Scalar> {
        match expr {
            Expr::Literal { value } => Ok(value.clone()),
            Expr::Read { scope, name } => self.read(*scope, name).cloned(),
            Expr::Not { value } => Ok(Scalar::Bool(!self.boolean(value)?)),
            Expr::Binary {
                op: BinaryOp::And,
                left,
                right,
            } => Ok(Scalar::Bool(self.boolean(left)? && self.boolean(right)?)),
            Expr::Binary {
                op: BinaryOp::Or,
                left,
                right,
            } => Ok(Scalar::Bool(self.boolean(left)? || self.boolean(right)?)),
            Expr::Binary { op, left, right } => {
                let left = self.evaluate(left)?;
                let right = self.evaluate(right)?;
                match op {
                    BinaryOp::Eq => Ok(Scalar::Bool(left == right)),
                    BinaryOp::Ne => Ok(Scalar::Bool(left != right)),
                    _ => {
                        let (Scalar::Int(a), Scalar::Int(b)) = (left, right) else {
                            return Err(Error::new(
                                "state",
                                "expression",
                                "expected checked integers",
                            ));
                        };
                        match op {
                            BinaryOp::Lt => Ok(Scalar::Bool(a < b)),
                            BinaryOp::Le => Ok(Scalar::Bool(a <= b)),
                            BinaryOp::Gt => Ok(Scalar::Bool(a > b)),
                            BinaryOp::Ge => Ok(Scalar::Bool(a >= b)),
                            BinaryOp::Add => a.checked_add(b).map(Scalar::Int).ok_or_else(overflow),
                            BinaryOp::Sub => a.checked_sub(b).map(Scalar::Int).ok_or_else(overflow),
                            _ => Err(Error::new(
                                "state",
                                "expression",
                                "invalid checked binary operation",
                            )),
                        }
                    }
                }
            }
        }
    }

    pub fn boolean(&self, expr: &Expr) -> Result<bool> {
        match self.evaluate(expr)? {
            Scalar::Bool(value) => Ok(value),
            _ => Err(Error::new("state", "condition", "expected checked boolean")),
        }
    }

    pub fn condition(&self, expr: Option<&Expr>) -> Result<bool> {
        expr.map_or(Ok(true), |value| self.boolean(value))
    }
}

fn overflow() -> Error {
    Error::new("overflow", "expression", "integer arithmetic overflow")
}
