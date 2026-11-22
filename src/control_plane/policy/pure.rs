use std::{cmp::Ordering, collections::BTreeMap};

use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Fault>;
#[derive(Debug)]
enum Fault {
    Fallback,
    Python(String),
}
fn error<T>(message: impl Into<String>) -> Result<T> {
    Err(Fault::Python(message.into()))
}

#[derive(Clone)]
enum Py {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Seq(bool, Vec<Py>), // true for tuple
    Dict(Vec<(String, Py)>),
}

impl Py {
    fn from_json(value: &Value, budget: &mut usize) -> Result<Self> {
        spend(budget, 1)?;
        Ok(match value {
            Value::Null => Self::None,
            Value::Bool(v) => Self::Bool(*v),
            Value::String(v) => {
                spend(budget, v.len() / 8)?;
                Self::Str(v.clone())
            }
            Value::Number(v) if v.is_f64() => Self::Float(v.as_f64().ok_or(Fault::Fallback)?),
            Value::Number(v) => Self::Int(v.as_i64().ok_or(Fault::Fallback)?),
            Value::Array(v) => Self::Seq(
                false,
                v.iter()
                    .map(|v| Self::from_json(v, budget))
                    .collect::<Result<_>>()?,
            ),
            Value::Object(v) => Self::Dict(
                v.iter()
                    .map(|(k, v)| Ok((k.clone(), Self::from_json(v, budget)?)))
                    .collect::<Result<_>>()?,
            ),
        })
    }

    fn into_json(self, depth: usize) -> Result<Value> {
        if depth > 32 {
            return error("ValueError: result nesting exceeds 32");
        }
        Ok(match self {
            Self::None => Value::Null,
            Self::Bool(v) => json!(v),
            Self::Int(v) => json!(v),
            Self::Float(v) if v.is_finite() => json!(v),
            Self::Str(v) => json!(v),
            Self::Seq(false, v) => Value::Array(
                v.into_iter()
                    .map(|v| v.into_json(depth + 1))
                    .collect::<Result<_>>()?,
            ),
            Self::Dict(v) => Value::Object(
                v.into_iter()
                    .map(|(k, v)| Ok((k, v.into_json(depth + 1)?)))
                    .collect::<Result<_>>()?,
            ),
            _ => {
                return error(
                    "TypeError: result must contain only JSON values with bounded integers",
                );
            }
        })
    }

    fn weight(&self) -> usize {
        match self {
            Self::Str(v) => 1 + v.len() / 8,
            Self::Seq(_, v) => 1 + v.iter().map(Self::weight).sum::<usize>(),
            Self::Dict(v) => {
                1 + v
                    .iter()
                    .map(|(k, v)| k.len() / 8 + v.weight())
                    .sum::<usize>()
            }
            _ => 1,
        }
    }

    fn truth(&self) -> bool {
        match self {
            Self::None => false,
            Self::Bool(v) => *v,
            Self::Int(v) => *v != 0,
            Self::Float(v) => *v != 0.0,
            Self::Str(v) => !v.is_empty(),
            Self::Seq(_, v) => !v.is_empty(),
            Self::Dict(v) => !v.is_empty(),
        }
    }

    fn int(&self) -> Option<i64> {
        match self {
            Self::Bool(v) => Some(i64::from(*v)),
            Self::Int(v) => Some(*v),
            _ => None,
        }
    }

    fn float(&self) -> Result<f64> {
        if let Self::Float(v) = self {
            return Ok(*v);
        }
        if let Some(v) = self.int() {
            if v.unsigned_abs() > (1 << 53) {
                return Err(Fault::Fallback);
            }
            return Ok(v as f64);
        }
        error("TypeError: expected a number")
    }

    fn string(&self) -> Result<&str> {
        if let Self::Str(v) = self {
            Ok(v)
        } else {
            error("TypeError: expected a string")
        }
    }

    fn len(&self) -> Result<usize> {
        Ok(match self {
            Self::Str(v) => v.chars().count(),
            Self::Seq(_, v) => v.len(),
            Self::Dict(v) => v.len(),
            _ => return error("TypeError: object has no len()"),
        })
    }

    fn iter(self) -> Result<Vec<Self>> {
        match self {
            Self::Str(v) => Ok(v.chars().map(|c| Self::Str(c.to_string())).collect()),
            Self::Seq(_, v) => Ok(v),
            Self::Dict(v) => Ok(v.into_iter().map(|(k, _)| Self::Str(k)).collect()),
            _ => error("TypeError: object is not iterable"),
        }
    }

    fn lookup(&self, key: &Self) -> Result<Option<&Self>> {
        let Self::Dict(values) = self else {
            return error("TypeError: expected a dict");
        };
        match key {
            Self::Str(k) => Ok(values.iter().find(|(name, _)| name == k).map(|(_, v)| v)),
            Self::Seq(false, _) | Self::Dict(_) => error("TypeError: unhashable key"),
            Self::Seq(true, _) => Err(Fault::Fallback),
            _ => Ok(None),
        }
    }
}

fn spend(budget: &mut usize, amount: usize) -> Result<()> {
    *budget = budget.checked_sub(amount).ok_or(Fault::Fallback)?;
    Ok(())
}

fn numeric(v: &Py) -> bool {
    matches!(v, Py::Int(_) | Py::Bool(_) | Py::Float(_))
}
fn equal(a: &Py, b: &Py) -> Result<bool> {
    Ok(match (a, b) {
        (Py::None, Py::None) => true,
        (Py::Str(a), Py::Str(b)) => a == b,
        (Py::Seq(at, a), Py::Seq(bt, b)) if at == bt && a.len() == b.len() => {
            for (a, b) in a.iter().zip(b) {
                if !equal(a, b)? {
                    return Ok(false);
                }
            }
            true
        }
        (Py::Dict(a), Py::Dict(b)) if a.len() == b.len() => {
            for (key, a) in a {
                if !b
                    .iter()
                    .find(|(k, _)| k == key)
                    .map(|(_, b)| equal(a, b))
                    .transpose()?
                    .unwrap_or(false)
                {
                    return Ok(false);
                }
            }
            true
        }
        _ if numeric(a) && numeric(b) => numeric_order(a, b)? == Ordering::Equal,
        _ => false,
    })
}
fn numeric_order(a: &Py, b: &Py) -> Result<Ordering> {
    if let (Some(a), Some(b)) = (a.int(), b.int()) {
        return Ok(a.cmp(&b));
    }
    a.float()?.partial_cmp(&b.float()?).ok_or(Fault::Fallback)
}
fn order(a: &Py, b: &Py) -> Result<Ordering> {
    match (a, b) {
        (Py::Str(a), Py::Str(b)) => Ok(a.cmp(b)),
        (Py::Seq(at, a), Py::Seq(bt, b)) if at == bt => {
            for (a, b) in a.iter().zip(b) {
                if !equal(a, b)? {
                    return order(a, b);
                }
            }
            Ok(a.len().cmp(&b.len()))
        }
        _ if numeric(a) && numeric(b) => numeric_order(a, b),
        _ => error("TypeError: values cannot be ordered"),
    }
}
fn compare(op: &str, a: &Py, b: &Py) -> Result<bool> {
    Ok(match op {
        "Eq" => equal(a, b)?,
        "NotEq" => !equal(a, b)?,
        "Lt" => order(a, b)?.is_lt(),
        "LtE" => !order(a, b)?.is_gt(),
        "Gt" => order(a, b)?.is_gt(),
        "GtE" => !order(a, b)?.is_lt(),
        "Is" | "IsNot" => {
            let same = match (a, b) {
                (Py::None, Py::None) => true,
                (Py::Bool(a), Py::Bool(b)) => a == b,
                (Py::None | Py::Bool(_), _) | (_, Py::None | Py::Bool(_)) => false,
                _ => return Err(Fault::Fallback),
            };
            same == (op == "Is")
        }
        "In" | "NotIn" => {
            let found = match b {
                Py::Str(b) => b.contains(a.string()?),
                Py::Dict(_) => b.lookup(a)?.is_some(),
                Py::Seq(_, b) => {
                    let mut found = false;
                    for b in b {
                        if equal(a, b)? {
                            found = true;
                            break;
                        }
                    }
                    found
                }
                _ => return error("TypeError: object is not iterable"),
            };
            found == (op == "In")
        }
        _ => return Err(Fault::Fallback),
    })
}

fn arithmetic(op: &str, a: Py, b: Py, budget: &mut usize) -> Result<Py> {
    if numeric(&a) && numeric(&b) {
        if let (Some(a), Some(b)) = (a.int(), b.int()) {
            return Ok(Py::Int(
                match op {
                    "Add" => a.checked_add(b),
                    "Sub" => a.checked_sub(b),
                    _ => a.checked_mul(b),
                }
                .ok_or(Fault::Fallback)?,
            ));
        }
        let (a, b) = (a.float()?, b.float()?);
        let result = match op {
            "Add" => a + b,
            "Sub" => a - b,
            _ => a * b,
        };
        return if result.is_finite() {
            Ok(Py::Float(result))
        } else {
            Err(Fault::Fallback)
        };
    }
    if op == "Add" {
        return match (a, b) {
            (Py::Str(mut a), Py::Str(b)) => {
                a.push_str(&b);
                Ok(Py::Str(a))
            }
            (Py::Seq(at, mut a), Py::Seq(bt, b)) if at == bt => {
                a.extend(b);
                Ok(Py::Seq(at, a))
            }
            _ => error("TypeError: unsupported operands for +"),
        };
    }
    if op == "Mult" {
        let (sequence, count) = if let Some(n) = b.int() {
            (a, n)
        } else if let Some(n) = a.int() {
            (b, n)
        } else {
            return error("TypeError: unsupported operands for *");
        };
        let count = usize::try_from(count.max(0)).map_err(|_| Fault::Fallback)?;
        if matches!(&sequence, Py::Str(s) if s.is_empty())
            || matches!(&sequence, Py::Seq(_,s) if s.is_empty())
        {
            return Ok(sequence);
        }
        spend(
            budget,
            sequence
                .weight()
                .checked_mul(count)
                .ok_or(Fault::Fallback)?,
        )?;
        return match sequence {
            Py::Str(s) => Ok(Py::Str(s.repeat(count))),
            Py::Seq(t, v) => Ok(Py::Seq(
                t,
                (0..count).flat_map(|_| v.iter().cloned()).collect(),
            )),
            _ => error("TypeError: unsupported operands for *"),
        };
    }
    error("TypeError: unsupported operands for -")
}

fn space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}
fn method(name: &str, receiver: Py, args: Vec<Py>) -> Result<Py> {
    match (name, args.as_slice()) {
        ("get", [key] | [key, _]) => receiver.lookup(key).map(|v| {
            v.cloned()
                .unwrap_or_else(|| args.get(1).cloned().unwrap_or(Py::None))
        }),
        ("items", []) => match receiver {
            Py::Dict(v) => Ok(Py::Seq(
                false,
                v.into_iter()
                    .map(|(k, v)| Py::Seq(true, vec![Py::Str(k), v]))
                    .collect(),
            )),
            _ => error("AttributeError: object has no items method"),
        },
        ("lower", []) => {
            let s = receiver.string()?;
            if !s.is_ascii() {
                return Err(Fault::Fallback);
            }
            Ok(Py::Str(s.to_ascii_lowercase()))
        }
        ("strip", [] | [Py::None]) => {
            Ok(Py::Str(receiver.string()?.trim_matches(space).to_owned()))
        }
        ("strip", [chars]) => {
            let chars = chars.string()?;
            Ok(Py::Str(
                receiver
                    .string()?
                    .trim_matches(|c| chars.contains(c))
                    .to_owned(),
            ))
        }
        ("startswith" | "endswith", [prefix] | [prefix, _] | [prefix, _, _]) => {
            let s = receiver.string()?;
            let end = args.get(2).unwrap_or(&Py::None);
            let start = args.get(1).unwrap_or(&Py::None);
            let bound = |value: &Py, default| match value {
                Py::None => Ok(default),
                _ => value.int().ok_or_else(|| {
                    Fault::Python("TypeError: slice indices must be integers or None".into())
                }),
            };
            let len = s.chars().count() as i64;
            let normalize = |value: i64| {
                if value < 0 {
                    (len + value).max(0)
                } else {
                    value
                }
            };
            let (start, end) = (
                normalize(bound(start, 0)?),
                normalize(bound(end, len)?).min(len),
            );
            let window = if start <= end {
                Some(
                    s.chars()
                        .skip(start as usize)
                        .take((end - start) as usize)
                        .collect::<String>(),
                )
            } else {
                None
            };
            let check = |prefix: &Py| -> Result<bool> {
                let prefix = prefix.string()?;
                Ok(window.as_ref().is_some_and(|s| {
                    if name == "startswith" {
                        s.starts_with(prefix)
                    } else {
                        s.ends_with(prefix)
                    }
                }))
            };
            if let Py::Seq(true, choices) = prefix {
                for p in choices {
                    if check(p)? {
                        return Ok(Py::Bool(true));
                    }
                }
                Ok(Py::Bool(false))
            } else {
                Ok(Py::Bool(check(prefix)?))
            }
        }
        ("split", [] | [_] | [_, _]) => {
            let s = receiver.string()?;
            let max = if let Some(n) = args.get(1) {
                n.int().ok_or_else(|| {
                    Fault::Python("TypeError: split maxsplit must be an integer".into())
                })?
            } else {
                -1
            };
            let limit = if max < 0 {
                usize::MAX
            } else {
                max as usize + 1
            };
            let mut parts = Vec::new();
            if let Some(sep) = args.first().filter(|v| !matches!(v, Py::None)) {
                let sep = sep.string()?;
                if sep.is_empty() {
                    return error("ValueError: empty separator");
                }
                parts.extend(s.splitn(limit, sep).map(|s| Py::Str(s.into())));
            } else {
                let mut rest = s.trim_start_matches(space);
                while !rest.is_empty() {
                    let end = if parts.len() + 1 == limit {
                        rest.len()
                    } else {
                        rest.find(space).unwrap_or(rest.len())
                    };
                    parts.push(Py::Str(rest[..end].into()));
                    rest = rest[end..].trim_start_matches(space);
                }
            }
            Ok(Py::Seq(false, parts))
        }
        _ => Err(Fault::Fallback),
    }
}

struct Interpreter {
    budget: usize,
    vars: BTreeMap<String, Py>,
}
fn array(value: &Value) -> Result<&[Value]> {
    value.as_array().map(Vec::as_slice).ok_or(Fault::Fallback)
}
fn text(value: &Value) -> Result<&str> {
    value.as_str().ok_or(Fault::Fallback)
}

impl Interpreter {
    fn eval(&mut self, instruction: &Value) -> Result<Py> {
        spend(&mut self.budget, 1)?;
        let [op, args @ ..] = array(instruction)? else {
            return Err(Fault::Fallback);
        };
        match (text(op)?, args) {
            ("const" | "float", [value]) => Py::from_json(value, &mut self.budget),
            ("var", [name]) => {
                let value = self
                    .vars
                    .get(text(name)?)
                    .ok_or_else(|| Fault::Python(format!("UnboundLocalError: {name}")))?;
                spend(&mut self.budget, value.weight())?;
                Ok(value.clone())
            }
            ("list" | "tuple", values) => Ok(Py::Seq(
                op == "tuple",
                values.iter().map(|v| self.eval(v)).collect::<Result<_>>()?,
            )),
            ("dict", entries) => {
                let mut result: Vec<(String, Py)> = Vec::new();
                for entry in entries {
                    let [key, value] = array(entry)? else {
                        return Err(Fault::Fallback);
                    };
                    let (key, value) = (text(key)?, self.eval(value)?);
                    if let Some((_, v)) = result.iter_mut().find(|(k, _)| k == key) {
                        *v = value;
                    } else {
                        result.push((key.into(), value));
                    }
                }
                Ok(Py::Dict(result))
            }
            ("if", [test, yes, no]) => {
                let branch = if self.eval(test)?.truth() { yes } else { no };
                self.eval(branch)
            }
            ("and" | "or", [first, rest @ ..]) => {
                let mut value = self.eval(first)?;
                for expression in rest {
                    if value.truth() == (op == "or") {
                        break;
                    }
                    value = self.eval(expression)?;
                }
                Ok(value)
            }
            ("compare", [first, comparisons @ ..]) => {
                let mut left = self.eval(first)?;
                for item in comparisons {
                    let [operator, right] = array(item)? else {
                        return Err(Fault::Fallback);
                    };
                    let right = self.eval(right)?;
                    if !compare(text(operator)?, &left, &right)? {
                        return Ok(Py::Bool(false));
                    }
                    left = right;
                }
                Ok(Py::Bool(true))
            }
            ("Not" | "bool" | "len" | "UAdd" | "USub" | "any" | "all", [arg]) => {
                let value = self.eval(arg)?;
                Ok(match text(op)? {
                    "Not" => Py::Bool(!value.truth()),
                    "bool" => Py::Bool(value.truth()),
                    "len" => Py::Int(value.len()? as i64),
                    "any" => Py::Bool(value.iter()?.iter().any(Py::truth)),
                    "all" => Py::Bool(value.iter()?.iter().all(Py::truth)),
                    "UAdd" | "USub" => {
                        if let Some(i) = value.int() {
                            Py::Int(if op == "USub" {
                                i.checked_neg().ok_or(Fault::Fallback)?
                            } else {
                                i
                            })
                        } else {
                            Py::Float(if op == "USub" {
                                -value.float()?
                            } else {
                                value.float()?
                            })
                        }
                    }
                    _ => unreachable!(),
                })
            }
            ("Add" | "Sub" | "Mult", [a, b]) => {
                let (a, b) = (self.eval(a)?, self.eval(b)?);
                arithmetic(text(op)?, a, b, &mut self.budget)
            }
            ("method", [name, obj, args @ ..]) => {
                let receiver = self.eval(obj)?;
                let name = text(name)?;
                let exists = match name {
                    "get" | "items" => matches!(receiver, Py::Dict(_)),
                    _ => matches!(receiver, Py::Str(_)),
                };
                if !exists {
                    return error(format!("AttributeError: object has no {name} method"));
                }
                let args = args
                    .iter()
                    .map(|arg| self.eval(arg))
                    .collect::<Result<_>>()?;
                method(name, receiver, args)
            }
            ("index", [obj, key]) => {
                let (obj, key) = (self.eval(obj)?, self.eval(key)?);
                if matches!(obj, Py::Dict(_)) {
                    return obj
                        .lookup(&key)?
                        .cloned()
                        .ok_or_else(|| Fault::Python("KeyError: missing dictionary key".into()));
                }
                if !matches!(obj, Py::Seq(..) | Py::Str(_)) {
                    return error("TypeError: object is not subscriptable");
                }
                let index = key
                    .int()
                    .ok_or_else(|| Fault::Python("TypeError: index must be an integer".into()))?;
                let len = obj.len()? as i64;
                let index = if index < 0 { len + index } else { index };
                if !(0..len).contains(&index) {
                    return error("IndexError: index out of range");
                }
                match obj {
                    Py::Seq(_, mut values) => Ok(values.swap_remove(index as usize)),
                    Py::Str(s) => Ok(Py::Str(
                        s.chars()
                            .nth(index as usize)
                            .ok_or(Fault::Fallback)?
                            .to_string(),
                    )),
                    _ => error("TypeError: object is not subscriptable"),
                }
            }
            ("slice", [obj, start, end]) => {
                let (obj, start, end) = (self.eval(obj)?, self.eval(start)?, self.eval(end)?);
                if !matches!(obj, Py::Seq(..) | Py::Str(_)) {
                    return error("TypeError: object cannot be sliced");
                }
                let len = obj.len()? as i64;
                let bound = |v: Py, default| -> Result<usize> {
                    if matches!(v, Py::None) {
                        return Ok(default);
                    }
                    let i = v.int().ok_or_else(|| {
                        Fault::Python("TypeError: slice indices must be integers or None".into())
                    })?;
                    Ok((if i < 0 { len + i } else { i }).clamp(0, len) as usize)
                };
                let start = bound(start, 0)?;
                let end = bound(end, len as usize)?.max(start);
                match obj {
                    Py::Str(s) => Ok(Py::Str(s.chars().skip(start).take(end - start).collect())),
                    Py::Seq(t, v) => Ok(Py::Seq(
                        t,
                        v.into_iter().skip(start).take(end - start).collect(),
                    )),
                    _ => unreachable!(),
                }
            }
            ("comprehension", [mode, target, iter, body, filters @ ..]) => {
                let items = self.eval(iter)?.iter()?;
                let names = if target.is_string() {
                    vec![text(target)?]
                } else {
                    array(target)?.iter().map(text).collect::<Result<_>>()?
                };
                let old: Vec<_> = names.iter().map(|name| self.vars.remove(*name)).collect();
                let result = self.comprehension(
                    text(mode)?,
                    target.is_string(),
                    &names,
                    items,
                    body,
                    filters,
                );
                for (name, value) in names.into_iter().zip(old) {
                    self.vars.remove(name);
                    if let Some(value) = value {
                        self.vars.insert(name.into(), value);
                    }
                }
                result
            }
            _ => Err(Fault::Fallback),
        }
    }

    fn comprehension(
        &mut self,
        mode: &str,
        single: bool,
        names: &[&str],
        items: Vec<Py>,
        body: &Value,
        filters: &[Value],
    ) -> Result<Py> {
        let mut values = Vec::new();
        'items: for item in items {
            spend(&mut self.budget, 1)?;
            if single {
                self.vars.insert(names[0].into(), item);
            } else {
                let elements = item.iter()?;
                if elements.len() != names.len() {
                    return error("ValueError: wrong number of values to unpack");
                }
                for (name, value) in names.iter().zip(elements) {
                    self.vars.insert((*name).into(), value);
                }
            }
            for test in filters {
                if !self.eval(test)?.truth() {
                    continue 'items;
                }
            }
            let value = self.eval(body)?;
            match mode {
                "list" => values.push(value),
                "any" if value.truth() => return Ok(Py::Bool(true)),
                "all" if !value.truth() => return Ok(Py::Bool(false)),
                _ => (),
            }
        }
        Ok(if mode == "list" {
            Py::Seq(false, values)
        } else {
            Py::Bool(mode == "all")
        })
    }

    fn block(&mut self, statements: &Value) -> Result<Option<Py>> {
        for statement in array(statements)? {
            spend(&mut self.budget, 1)?;
            match array(statement)? {
                [op, value] if op == "return" => return self.eval(value).map(Some),
                [op, name, value] if op == "assign" => {
                    let value = self.eval(value)?;
                    self.vars.insert(text(name)?.into(), value);
                }
                [op, test, yes, no] if op == "branch" => {
                    let branch = if self.eval(test)?.truth() { yes } else { no };
                    if let Some(value) = self.block(branch)? {
                        return Ok(Some(value));
                    }
                }
                _ => return Err(Fault::Fallback),
            }
        }
        Ok(None)
    }
}

pub(super) fn evaluate(
    program: &Value,
    data: &Value,
    config: &BTreeMap<String, Value>,
) -> Option<std::result::Result<Value, String>> {
    let run = || {
        let mut evaluator = Interpreter {
            budget: 8192,
            vars: BTreeMap::new(),
        };
        let data = Py::from_json(data, &mut evaluator.budget)?;
        let config = Py::Dict(
            config
                .iter()
                .map(|(key, v)| Ok((key.clone(), Py::from_json(v, &mut evaluator.budget)?)))
                .collect::<Result<_>>()?,
        );
        evaluator
            .vars
            .insert(text(&program["argument"])?.into(), data);
        evaluator.vars.insert("policy_config".into(), config);
        evaluator
            .block(&program["body"])?
            .unwrap_or(Py::None)
            .into_json(0)
    };
    match run() {
        Ok(value) => Some(Ok(value)),
        Err(Fault::Python(message)) => Some(Err(message)),
        Err(Fault::Fallback) => None,
    }
}
