use std::collections::BTreeMap;

use crate::{BinaryOp, Error, Expr, Graph, MAX_TEXT_BYTES, Result, Scalar, ScalarType, Scope};

pub(crate) fn variable_type(
    graph: &Graph,
    scope: Scope,
    name: &str,
    path: &str,
) -> Result<ScalarType> {
    let kind = match scope {
        Scope::Parameter => graph.parameters.get(name).copied(),
        Scope::Local => graph.locals.get(name).map(Scalar::kind),
        Scope::Shared => graph.shared.get(name).copied(),
    };
    kind.ok_or_else(|| {
        Error::new(
            "reference",
            path,
            format!("undeclared {scope:?} variable {name}"),
        )
    })
}

pub(crate) fn check(expr: &Expr, graph: &Graph, depth: usize, path: &str) -> Result<ScalarType> {
    if depth > 48 {
        return Err(Error::new("limit", path, "expression nesting exceeds 48"));
    }
    match expr {
        Expr::Literal { value } => {
            check_scalar(value, path)?;
            Ok(value.kind())
        }
        Expr::Read { scope, name } => variable_type(graph, *scope, name, path),
        Expr::Not { value } => {
            expect(
                check(value, graph, depth + 1, path)?,
                ScalarType::Bool,
                path,
            )?;
            Ok(ScalarType::Bool)
        }
        Expr::Binary { op, left, right } => {
            let left = check(left, graph, depth + 1, path)?;
            let right = check(right, graph, depth + 1, path)?;
            expect(right, left, path)?;
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
    if let Scalar::Text(v) = value {
        check_text(v, path)?;
    }
    Ok(())
}

pub(crate) fn check_text(value: &str, path: &str) -> Result<()> {
    if value.len() > MAX_TEXT_BYTES {
        Err(Error::new("limit", path, "text exceeds 64 KiB"))
    } else {
        Ok(())
    }
}

pub(crate) fn template_variables(
    text: &str,
    path: &str,
) -> Result<Vec<(usize, usize, Scope, String)>> {
    let mut matches = Vec::new();
    let mut offset = 0;
    while let Some(start) = text[offset..].find("{{") {
        let start = offset + start;
        let end = text[start + 2..]
            .find("}}")
            .map(|i| start + 2 + i + 2)
            .ok_or_else(|| Error::new("template", path, "unclosed {{variable}}"))?;
        let field = text[start + 2..end - 2].trim();
        let (scope, name) = field.split_once('.').ok_or_else(|| {
            Error::new(
                "template",
                path,
                "use {{parameter.name}}, {{local.name}} or {{shared.name}}",
            )
        })?;
        let scope = match scope {
            "parameter" => Scope::Parameter,
            "local" => Scope::Local,
            "shared" => Scope::Shared,
            _ => {
                return Err(Error::new(
                    "template",
                    path,
                    format!("unknown variable scope {scope}"),
                ));
            }
        };
        if !crate::compile::valid_name(name) {
            return Err(Error::new("template", path, "invalid variable name"));
        }
        matches.push((start, end, scope, name.into()));
        offset = end;
    }
    Ok(matches)
}

pub(crate) fn check_template(text: &str, graph: &Graph, path: &str) -> Result<()> {
    check_text(text, path)?;
    for (_, _, scope, name) in template_variables(text, path)? {
        variable_type(graph, scope, &name, path)?;
    }
    Ok(())
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
                            BinaryOp::Add | BinaryOp::Sub => {
                                let result = if *op == BinaryOp::Add {
                                    a.checked_add(b)
                                } else {
                                    a.checked_sub(b)
                                };
                                result.map(Scalar::Int).ok_or_else(|| {
                                    Error::new(
                                        "overflow",
                                        "expression",
                                        "integer arithmetic overflow",
                                    )
                                })
                            }
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
            Scalar::Bool(v) => Ok(v),
            _ => Err(Error::new("state", "condition", "expected checked boolean")),
        }
    }

    pub fn render(&self, text: &str) -> Result<String> {
        let mut output = String::new();
        let mut offset = 0;
        for (start, end, scope, name) in template_variables(text, "content")? {
            output.push_str(&text[offset..start]);
            output.push_str(&self.read(scope, &name)?.display());
            offset = end;
            check_text(&output, "rendered_content")?;
        }
        output.push_str(&text[offset..]);
        check_text(&output, "rendered_content")?;
        Ok(output)
    }
}
