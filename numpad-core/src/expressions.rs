use crate::{
    functions::{
        get_built_in_function_def, get_built_in_function_id, get_function_def,
        is_built_in_function, BUILT_IN_FUNCTION_IDENTS,
    },
    heap::{Heap, HeapPointer, HeapValue, ListPointer},
    parser::Rule,
    values::{
        LambdaArg, LambdaDef,
        Value::{self, Bool, List, Number, Spread},
    },
};
use anyhow::{anyhow, Ok, Result};
use pest::{
    iterators::Pairs,
    pratt_parser::{Assoc, Op, PrattParser},
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    rc::Rc,
    sync::LazyLock,
};

static PRATT: LazyLock<PrattParser<Rule>> = LazyLock::new(|| {
    PrattParser::new()
        .op(Op::infix(Rule::and, Assoc::Left)
            | Op::infix(Rule::natural_and, Assoc::Left)
            | Op::infix(Rule::or, Assoc::Left)
            | Op::infix(Rule::natural_or, Assoc::Left)
            | Op::infix(Rule::with, Assoc::Left))
        .op(Op::infix(Rule::equal, Assoc::Left)
            | Op::infix(Rule::not_equal, Assoc::Left)
            | Op::infix(Rule::less, Assoc::Left)
            | Op::infix(Rule::less_eq, Assoc::Left)
            | Op::infix(Rule::greater, Assoc::Left)
            | Op::infix(Rule::greater_eq, Assoc::Left))
        .op(Op::infix(Rule::add, Assoc::Left) | Op::infix(Rule::subtract, Assoc::Left))
        .op(Op::infix(Rule::multiply, Assoc::Left)
            | Op::infix(Rule::divide, Assoc::Left)
            | Op::infix(Rule::modulo, Assoc::Left))
        .op(Op::infix(Rule::power, Assoc::Right) | Op::infix(Rule::coalesce, Assoc::Left))
        .op(Op::prefix(Rule::negation)
            | Op::prefix(Rule::spread_operator)
            | Op::prefix(Rule::invert)
            | Op::prefix(Rule::each))
        .op(Op::postfix(Rule::factorial))
        .op(Op::postfix(Rule::access)
            | Op::postfix(Rule::dot_access)
            | Op::postfix(Rule::call_list))
});

pub fn collect_list(
    pairs: Pairs<Rule>,
    heap: Rc<RefCell<Heap>>,
    bindings: Rc<RefCell<HashMap<String, Value>>>,
    call_depth: usize,
) -> Result<Vec<Value>> {
    Ok(pairs
        .into_iter()
        // This can't just be flat_map (at least I think) because we don't want to unwrap
        // each Result as we iterate, since that would result in failed values being skipped.
        .map(|value| {
            evaluate_expression(
                value.into_inner(),
                Rc::clone(&heap),
                Rc::clone(&bindings),
                call_depth,
            )
        })
        .collect::<Result<Vec<Value>>>()?
        .into_iter()
        // .flatten()
        .collect())
}

pub fn evaluate_expression(
    pairs: Pairs<Rule>,
    heap: Rc<RefCell<Heap>>,
    bindings: Rc<RefCell<HashMap<String, Value>>>,
    call_depth: usize,
) -> Result<Value> {
    PRATT
        .map_primary(|primary| match primary.as_rule() {
            Rule::number => Ok(Number(
                primary
                    .as_str()
                    .replace("_", "")
                    .parse::<f64>()
                    .map_err(|e| anyhow::Error::from(e))?,
            )),
            Rule::list => {
                let list_pairs = primary.into_inner();
                let values = collect_list(
                    list_pairs,
                    Rc::clone(&heap),
                    Rc::clone(&bindings),
                    call_depth,
                )?;

                Ok(heap.borrow_mut().insert_list(values))
            }
            Rule::record => {
                let record_pairs = primary.into_inner();
                let mut record = BTreeMap::new();

                for pair in record_pairs {
                    match pair.as_rule() {
                        Rule::record_pair => {
                            let mut inner_pairs = pair.into_inner();
                            let key_pair = inner_pairs.next().unwrap();
                            let key = match key_pair.as_rule() {
                                Rule::record_key_static => {
                                    let inner_key_pair = key_pair.into_inner().next().unwrap();
                                    match inner_key_pair.as_rule() {
                                        Rule::identifier => inner_key_pair.as_str().to_string(),
                                        Rule::string => {
                                            inner_key_pair.into_inner().as_str().to_string()
                                        }
                                        _ => unreachable!(),
                                    }
                                }
                                Rule::record_key_dynamic => evaluate_expression(
                                    key_pair.into_inner(),
                                    Rc::clone(&heap),
                                    Rc::clone(&bindings),
                                    call_depth,
                                )?
                                .as_string(&heap.borrow())?
                                .to_string(),
                                _ => unreachable!(),
                            };

                            let value = evaluate_expression(
                                inner_pairs.next().unwrap().into_inner(),
                                Rc::clone(&heap),
                                Rc::clone(&bindings),
                                call_depth,
                            )?;
                            record.insert(key, value);
                        }
                        Rule::record_shorthand => {
                            let ident = pair.into_inner().next().unwrap().as_str().to_string();
                            let value = bindings
                                .borrow()
                                .get(&ident)
                                .copied()
                                .ok_or(anyhow!("unknown identifier: {}", ident))?;

                            record.insert(ident, value);
                        }
                        // Rule::spread_expression => {
                        //     let spread_value = evaluate_expression(
                        //         pair.into_inner(),
                        //         Rc::clone(&heap),
                        //         Rc::clone(&bindings),
                        //         call_depth,
                        //     )?;

                        //     match spread_value {
                        //         Spread(spread_value) => match spread_value {
                        //             IterableValue::List(list) => {
                        //                 for (i, value) in list.into_iter().enumerate() {
                        //                     record.insert(i.to_string(), value);
                        //                 }
                        //             }
                        //             IterableValue::String(s) => {
                        //                 for (i, c) in s.chars().enumerate() {
                        //                     record.insert(
                        //                         i.to_string(),
                        //                         Value::String(c.to_string()),
                        //                     );
                        //                 }
                        //             }
                        //             IterableValue::Record(spread_record) => {
                        //                 for (k, v) in spread_record {
                        //                     record.insert(k, v);
                        //                 }
                        //             }
                        //         },
                        //         _ => {}
                        //     }
                        // }
                        _ => {}
                    }
                }

                Ok(heap.borrow_mut().insert_record(record))
            }
            Rule::bool => {
                let bool_str = primary.as_str();

                match bool_str {
                    "true" => Ok(Bool(true)),
                    "false" => Ok(Bool(false)),
                    _ => unreachable!(),
                }
            }
            Rule::null => Ok(Value::Null),
            Rule::string => {
                let contents = primary.into_inner().as_str().to_string();
                Ok(heap.borrow_mut().insert_string(contents))
            }
            Rule::assignment => {
                let mut inner_pairs = primary.into_inner();
                let ident = inner_pairs.next().unwrap().as_str();

                if BUILT_IN_FUNCTION_IDENTS.contains(&ident) {
                    return Err(anyhow!("cannot assign to built-in function: {}", ident));
                }

                if ident == "pi" || ident == "e" || ident == "infinity" {
                    return Err(anyhow!("cannot assign to constant: {}", ident));
                }

                if ident == "if"
                    || ident == "then"
                    || ident == "else"
                    || ident == "true"
                    || ident == "false"
                    || ident == "null"
                    || ident == "inputs"
                    || ident == "and"
                    || ident == "or"
                    || ident == "each"
                    || ident == "with"
                {
                    return Err(anyhow!("cannot assign to keyword: {}", ident));
                }

                if bindings.borrow().contains_key(ident) {
                    return Err(anyhow!("identifier already defined: {}", ident));
                }

                let expression = inner_pairs.next().unwrap();
                let mut value = evaluate_expression(
                    expression.into_inner(),
                    Rc::clone(&heap),
                    Rc::clone(&bindings),
                    call_depth,
                )?;

                match value {
                    Value::Lambda(pointer) => match pointer.reify_mut(&mut heap.borrow_mut()) {
                        HeapValue::Lambda(lambda) => {
                            lambda.set_name(ident.to_string());
                        }
                        _ => {}
                    },
                    _ => {}
                }

                bindings
                    .borrow_mut()
                    .insert(ident.to_string(), value.clone());
                Ok(value)
            }
            Rule::lambda => {
                let mut inner_pairs = primary.into_inner();
                let arg_pairs = inner_pairs.next().unwrap().into_inner();
                let mut has_rest = false;
                let mut has_optional = false;

                let args = arg_pairs
                    .map(|arg| match arg.as_rule() {
                        Rule::required_arg => {
                            if has_optional {
                                return Err(anyhow!(
                                    "required argument '{}' must come before optional arguments",
                                    arg.into_inner().as_str()
                                ));
                            }

                            if has_rest {
                                return Err(anyhow!(
                                    "required argument '{}' must come before rest arguments",
                                    arg.into_inner().as_str()
                                ));
                            }

                            Ok(LambdaArg::Required(arg.into_inner().as_str().to_string()))
                        }
                        Rule::optional_arg => {
                            if has_rest {
                                return Err(anyhow!(
                                    "optional argument '{}' must come before a rest argument",
                                    arg.into_inner().as_str()
                                ));
                            }

                            has_optional = true;
                            Ok(LambdaArg::Optional(arg.into_inner().as_str().to_string()))
                        }
                        Rule::rest_arg => {
                            if has_rest {
                                return Err(anyhow!("a function can only have one rest argument"));
                            }

                            has_rest = true;
                            Ok(LambdaArg::Rest(arg.into_inner().as_str().to_string()))
                        }
                        _ => unreachable!(),
                    })
                    .collect::<Result<Vec<LambdaArg>>>()?;

                let body = inner_pairs.next().unwrap();
                let lambda = heap.borrow_mut().insert_lambda(LambdaDef {
                    name: None,
                    args,
                    body: body.as_str().to_string(),
                });

                Ok(lambda)
            }
            Rule::conditional => {
                let mut inner_pairs = primary.into_inner();
                let condition_expr = inner_pairs.next().unwrap();
                let then_expr = inner_pairs.next().unwrap();
                let else_expr = inner_pairs.next().unwrap();

                let condition = evaluate_expression(
                    condition_expr.into_inner(),
                    Rc::clone(&heap),
                    Rc::clone(&bindings),
                    call_depth,
                )?
                .as_bool()?;

                if condition {
                    evaluate_expression(
                        then_expr.into_inner(),
                        Rc::clone(&heap),
                        Rc::clone(&bindings),
                        call_depth,
                    )
                } else {
                    evaluate_expression(
                        else_expr.into_inner(),
                        Rc::clone(&heap),
                        Rc::clone(&bindings),
                        call_depth,
                    )
                }
            }
            Rule::identifier => {
                let ident = primary.as_str();

                match ident {
                    "pi" => return Ok(Number(core::f64::consts::PI)),
                    "e" => return Ok(Number(core::f64::consts::E)),
                    "infinity" => return Ok(Number(f64::INFINITY)),
                    _ => {
                        if let Some(built_in_id) = get_built_in_function_id(ident) {
                            return Ok(Value::BuiltIn(built_in_id));
                        }

                        let v = bindings
                            .borrow_mut()
                            .get(ident)
                            .copied()
                            .ok_or(anyhow!("unknown identifier: {}", primary.as_str()));

                        v
                    }
                }
            }
            Rule::expression => {
                evaluate_expression(primary.into_inner(), heap, bindings, call_depth)
            }
            _ => unreachable!("{}", primary.as_str()),
        })
        .map_prefix(|op, rhs| match op.as_rule() {
            Rule::negation => {
                let rhs = rhs?.as_number()?;
                Ok(Number(-rhs))
            }
            Rule::spread_operator => {
                let rhs = rhs?;
                match rhs {
                    List(list) => return Ok(Spread(IterableValue::List(list))),
                    Value::String(s) => return Ok(Spread(IterableValue::String(s))),
                    Value::Record(record) => return Ok(Spread(IterableValue::Record(record))),
                    _ => return Err(anyhow!("expected a list, record, or string")),
                }
            }
            Rule::invert => {
                let rhs = rhs?.as_bool()?;
                Ok(Bool(!rhs))
            }
            Rule::each => {
                let rhs = rhs?;
                match rhs {
                    List(list) => return Ok(Value::Each(IterableValue::List(list))),
                    Value::String(s) => return Ok(Value::Each(IterableValue::String(s))),
                    Value::Record(record) => return Ok(Value::Each(IterableValue::Record(record))),
                    _ => return Err(anyhow!("expected a list, record, or string")),
                }
            }
            _ => unreachable!(),
        })
        .map_postfix(|lhs, op| match op.as_rule() {
            Rule::factorial => {
                let lhs = lhs?.as_number()?;

                if lhs >= 0.0 && lhs == (lhs as u64) as f64 {
                    Ok(Number(
                        (1..(lhs as u64) + 1).map(|x| x as f64).product::<f64>(),
                    ))
                } else {
                    Err(anyhow!("factorial only works on non-negative integers"))
                }
            }
            Rule::access => {
                let lhs = lhs?;
                match lhs {
                    Value::Record(record) => {
                        let key_value =
                            evaluate_expression(op.into_inner(), heap, bindings, call_depth)?;

                        let key = key_value.as_string()?;

                        Ok(record.get(key).copied().unwrap_or(Value::Null))
                    }
                    Value::List(list) => {
                        let index_value =
                            evaluate_expression(op.into_inner(), heap, bindings, call_depth)?;

                        let index = usize::try_from(index_value.as_number()? as u64)?;

                        Ok(list.get(index).copied().unwrap_or(Value::Null))
                    }
                    _ => Err(anyhow!(
                        "expected a record or list, but got a {}",
                        lhs.get_type()
                    )),
                }
            }
            Rule::dot_access => {
                let lhs = lhs?;
                let key = op.into_inner().as_str();

                match lhs {
                    Value::Record(record) => Ok(record.get(key).copied().unwrap_or(Value::Null)),
                    _ => Err(anyhow!("expected a record, but got a {}", lhs.get_type())),
                }
            }
            Rule::call_list => {
                let lhs = lhs?;
                let call_list = op.into_inner();
                let args = collect_list(call_list, Rc::clone(&bindings), call_depth)?;

                if !lhs.is_lambda() && !lhs.is_built_in() {
                    return Err(anyhow!("can't call a non-function: {}", lhs));
                }

                if let Some(def) = get_function_def(&lhs) {
                    return def.call(args, Rc::clone(&bindings), None, call_depth);
                }

                Err(anyhow!("unknown function: {}", lhs))
            }
            _ => unreachable!(),
        })
        .map_infix(|lhs, op, rhs| {
            let lhs = lhs?;
            let rhs = rhs?;
            let rule = op.as_rule();

            match (lhs, rhs) {
                (Value::Each(iterable_l), Value::Each(iterable_r)) => {
                    let iter_l = iterable_l.into_iter();
                    let mut iter_r = iterable_r.into_iter();

                    if iter_l.len() != iter_r.len() {
                        return Err(anyhow!(
                            "left- and right-hand-side iterables must be the same length"
                        ));
                    }

                    match rule {
                        Rule::equal => Ok(Bool(iter_l.zip(iter_r).all(|(l, r)| l == r))),
                        Rule::not_equal => Ok(Bool(iter_l.zip(iter_r).all(|(l, r)| l != r))),
                        Rule::less => Ok(Bool(iter_l.zip(iter_r).all(|(l, r)| l < r))),
                        Rule::less_eq => Ok(Bool(iter_l.zip(iter_r).all(|(l, r)| l <= r))),
                        Rule::greater => Ok(Bool(iter_l.zip(iter_r).all(|(l, r)| l > r))),
                        Rule::greater_eq => Ok(Bool(iter_l.zip(iter_r).all(|(l, r)| l >= r))),
                        Rule::and | Rule::natural_and => {
                            let mapped_list = iter_l
                                .zip(iter_r)
                                .map(|(l, r)| Ok(Bool(l.as_bool()? && r.as_bool()?)))
                                .collect::<Result<Vec<Value>>>()?;

                            Ok(Value::Each(IterableValue::List(mapped_list)))
                        }
                        Rule::or | Rule::natural_or => {
                            let mapped_list = iter_l
                                .zip(iter_r)
                                .map(|(l, r)| Ok(Bool(l.as_bool()? || r.as_bool()?)))
                                .collect::<Result<Vec<Value>>>()?;

                            Ok(Value::Each(IterableValue::List(mapped_list)))
                        }
                        Rule::add => {
                            if iter_r.all(|v| v.is_string()) {
                                let mapped_list = iter_l
                                    .zip(iter_r)
                                    .map(|(l, r)| {
                                        Ok(Value::String(format!(
                                            "{}{}",
                                            l.as_string()?,
                                            r.as_string()?
                                        )))
                                    })
                                    .collect::<Result<Vec<Value>>>()?;

                                return Ok(Value::Each(IterableValue::List(mapped_list)));
                            }

                            let mapped_list = iter_l
                                .zip(iter_r)
                                .map(|(l, r)| Ok(Number(l.as_number()? + r.as_number()?)))
                                .collect::<Result<Vec<Value>>>()?;

                            Ok(Value::Each(IterableValue::List(mapped_list)))
                        }
                        Rule::subtract => {
                            let mapped_list = iter_l
                                .zip(iter_r)
                                .map(|(l, r)| Ok(Number(l.as_number()? - r.as_number()?)))
                                .collect::<Result<Vec<Value>>>()?;

                            Ok(Value::Each(IterableValue::List(mapped_list)))
                        }
                        Rule::multiply => {
                            let mapped_list = iter_l
                                .zip(iter_r)
                                .map(|(l, r)| Ok(Number(l.as_number()? * r.as_number()?)))
                                .collect::<Result<Vec<Value>>>()?;

                            Ok(Value::Each(IterableValue::List(mapped_list)))
                        }
                        Rule::divide => {
                            let mapped_list = iter_l
                                .zip(iter_r)
                                .map(|(l, r)| Ok(Number(l.as_number()? / r.as_number()?)))
                                .collect::<Result<Vec<Value>>>()?;

                            Ok(Value::Each(IterableValue::List(mapped_list)))
                        }
                        Rule::modulo => {
                            let mapped_list = iter_l
                                .zip(iter_r)
                                .map(|(l, r)| Ok(Number(l.as_number()? % r.as_number()?)))
                                .collect::<Result<Vec<Value>>>()?;

                            Ok(Value::Each(IterableValue::List(mapped_list)))
                        }
                        Rule::power => {
                            let mapped_list = iter_l
                                .zip(iter_r)
                                .map(|(l, r)| Ok(Number(l.as_number()?.powf(r.as_number()?))))
                                .collect::<Result<Vec<Value>>>()?;

                            Ok(Value::Each(IterableValue::List(mapped_list)))
                        }
                        Rule::coalesce => {
                            let mapped_list = iter_l
                                .zip(iter_r)
                                .map(|(l, r)| if l == Value::Null { Ok(r) } else { Ok(l) })
                                .collect::<Result<Vec<Value>>>()?;

                            Ok(Value::Each(IterableValue::List(mapped_list)))
                        }
                        Rule::with => {
                            let mapped_list = iter_l
                                .zip(iter_r)
                                .map(|(l, r)| {
                                    if !r.is_lambda() && !r.is_built_in() {
                                        return Err(anyhow!(
                                            "right-hand iterable contains non-function {}",
                                            r
                                        ));
                                    }

                                    get_function_def(&r)
                                        .ok_or(anyhow!("can't call unknown function {}", r))?
                                        .call(vec![l], Rc::clone(&bindings), None, call_depth)
                                })
                                .collect::<Result<Vec<Value>>>()?;

                            Ok(Value::Each(IterableValue::List(mapped_list)))
                        }
                        _ => unreachable!(),
                    }
                }
                (Value::Each(iterable), rhs) => {
                    let mut iter_l = iterable.into_iter();

                    match rule {
                        Rule::equal => Ok(Bool(iter_l.all(|v| v == rhs))),
                        Rule::not_equal => Ok(Bool(iter_l.all(|v| v != rhs))),
                        Rule::less => Ok(Bool(iter_l.all(|v| v < rhs))),
                        Rule::less_eq => Ok(Bool(iter_l.all(|v| v <= rhs))),
                        Rule::greater => Ok(Bool(iter_l.all(|v| v > rhs))),
                        Rule::greater_eq => Ok(Bool(iter_l.all(|v| v >= rhs))),
                        Rule::and | Rule::natural_and => {
                            let mapped_list = iter_l
                                .map(|v| Ok(Bool(v.as_bool()? && rhs.as_bool()?)))
                                .collect::<Result<Vec<Value>>>()?;

                            Ok(Value::Each(IterableValue::List(mapped_list)))
                        }
                        Rule::or | Rule::natural_or => {
                            let mapped_list = iter_l
                                .map(|v| Ok(Bool(v.as_bool()? || rhs.as_bool()?)))
                                .collect::<Result<Vec<Value>>>()?;

                            Ok(Value::Each(IterableValue::List(mapped_list)))
                        }
                        Rule::add => {
                            if rhs.is_string() {
                                let rhs_string = rhs.as_string()?;
                                let mapped_list = iter_l
                                    .map(|v| {
                                        Ok(Value::String(format!(
                                            "{}{}",
                                            v.as_string()?,
                                            rhs_string
                                        )))
                                    })
                                    .collect::<Result<Vec<Value>>>()?;

                                return Ok(Value::Each(IterableValue::List(mapped_list)));
                            }

                            let mapped_list = iter_l
                                .map(|v| Ok(Number(v.as_number()? + rhs.as_number()?)))
                                .collect::<Result<Vec<Value>>>()?;

                            return Ok(Value::Each(IterableValue::List(mapped_list)));
                        }
                        Rule::subtract => {
                            let mapped_list = iter_l
                                .map(|v| Ok(Number(v.as_number()? - rhs.as_number()?)))
                                .collect::<Result<Vec<Value>>>()?;

                            Ok(Value::Each(IterableValue::List(mapped_list)))
                        }
                        Rule::multiply => {
                            let mapped_list = iter_l
                                .map(|v| Ok(Number(v.as_number()? * rhs.as_number()?)))
                                .collect::<Result<Vec<Value>>>()?;

                            Ok(Value::Each(IterableValue::List(mapped_list)))
                        }
                        Rule::divide => {
                            let mapped_list = iter_l
                                .map(|v| Ok(Number(v.as_number()? / rhs.as_number()?)))
                                .collect::<Result<Vec<Value>>>()?;

                            Ok(Value::Each(IterableValue::List(mapped_list)))
                        }
                        Rule::modulo => {
                            let mapped_list = iter_l
                                .map(|v| Ok(Number(v.as_number()? % rhs.as_number()?)))
                                .collect::<Result<Vec<Value>>>()?;

                            Ok(Value::Each(IterableValue::List(mapped_list)))
                        }
                        Rule::power => {
                            let mapped_list = iter_l
                                .map(|v| Ok(Number(v.as_number()?.powf(rhs.as_number()?))))
                                .collect::<Result<Vec<Value>>>()?;

                            Ok(Value::Each(IterableValue::List(mapped_list)))
                        }
                        Rule::coalesce => {
                            let mapped_list = iter_l
                                .map(|v| {
                                    if v == Value::Null {
                                        Ok(rhs.clone())
                                    } else {
                                        Ok(v)
                                    }
                                })
                                .collect::<Result<Vec<Value>>>()?;

                            Ok(Value::Each(IterableValue::List(mapped_list)))
                        }
                        Rule::with => {
                            if !rhs.is_lambda() && !rhs.is_built_in() {
                                return Err(anyhow!("can't call a non-function: {}", rhs));
                            }

                            let values: Vec<Value> = iter_l.collect();

                            let mapped_list = get_built_in_function_def("map")
                                .unwrap()
                                .call(
                                    vec![rhs, Value::List(values)],
                                    Rc::clone(&bindings),
                                    None,
                                    call_depth,
                                )?
                                .as_list()?
                                .to_owned();

                            Ok(Value::Each(IterableValue::List(mapped_list)))
                        }
                        _ => unreachable!(),
                    }
                }
                (lhs, rhs) => match rule {
                    Rule::equal => Ok(Bool(lhs == rhs)),
                    Rule::not_equal => Ok(Bool(lhs != rhs)),
                    Rule::less => Ok(Bool(lhs < rhs)),
                    Rule::less_eq => Ok(Bool(lhs <= rhs)),
                    Rule::greater => Ok(Bool(lhs > rhs)),
                    Rule::greater_eq => Ok(Bool(lhs >= rhs)),
                    Rule::and | Rule::natural_and => Ok(Bool(lhs.as_bool()? && rhs.as_bool()?)),
                    Rule::or | Rule::natural_or => Ok(Bool(lhs.as_bool()? || rhs.as_bool()?)),
                    Rule::add => {
                        if lhs.is_string() {
                            return Ok(Value::String(format!(
                                "{}{}",
                                lhs.as_string()?,
                                rhs.as_string()?
                            )));
                        }

                        Ok(Number(lhs.as_number()? + rhs.as_number()?))
                    }
                    Rule::subtract => Ok(Number(lhs.as_number()? - rhs.as_number()?)),
                    Rule::multiply => Ok(Number(lhs.as_number()? * rhs.as_number()?)),
                    Rule::divide => Ok(Number(lhs.as_number()? / rhs.as_number()?)),
                    Rule::modulo => Ok(Number(lhs.as_number()? % rhs.as_number()?)),
                    Rule::power => Ok(Number(lhs.as_number()?.powf(rhs.as_number()?))),
                    Rule::coalesce => {
                        if lhs == Value::Null {
                            Ok(rhs)
                        } else {
                            Ok(lhs)
                        }
                    }
                    Rule::with => {
                        if !rhs.is_lambda() && !rhs.is_built_in() {
                            return Err(anyhow!(
                                "can't call a non-function ({} is of type {})",
                                rhs,
                                rhs.get_type()
                            ));
                        }

                        if let Some(def) = get_function_def(&rhs) {
                            return def.call(vec![lhs], Rc::clone(&bindings), None, call_depth);
                        }

                        Err(anyhow!("unknown function: {}", rhs))
                    }
                    _ => unreachable!(),
                },
            }
        })
        .parse(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::get_pairs;

    fn parse_and_evaluate(
        input: &str,
        vars: Option<Rc<RefCell<HashMap<String, Value>>>>,
    ) -> Result<Value> {
        let binding = input.to_string();
        let mut pairs = get_pairs(&binding).unwrap();
        let expr = pairs.next().unwrap().into_inner();
        evaluate_expression(
            expr,
            vars.unwrap_or(Rc::new(RefCell::new(HashMap::new()))),
            0,
        )
    }

    #[test]
    fn addition_of_integers() {
        let result = parse_and_evaluate("5 + 2", None).unwrap();
        assert_eq!(result, Value::Number(7.0));
    }

    #[test]
    fn exponentiation_of_two_integers() {
        let result = parse_and_evaluate("2 ^ 3", None).unwrap();
        assert_eq!(result, Value::Number(8.0));
    }

    #[test]
    fn multiplication_of_integers() {
        let result = parse_and_evaluate("8 * 4", None).unwrap();
        assert_eq!(result, Value::Number(32.0));
    }

    #[test]
    fn division_with_integer_resulting_in_decimal() {
        let result = parse_and_evaluate("9 / 2", None).unwrap();
        assert_eq!(result, Value::Number(4.5));
    }

    #[test]
    fn addition_with_nested_expression() {
        let result = parse_and_evaluate("5 + (2 * 4)", None).unwrap();
        assert_eq!(result, Value::Number(13.0));
    }

    #[test]
    fn grouping_and_multiplication_in_expression() {
        let result = parse_and_evaluate("(3 + 2) * 2", None).unwrap();
        assert_eq!(result, Value::Number(10.0));
    }

    #[test]
    fn mixed_operations_with_decimal_and_precedence() {
        let result = parse_and_evaluate("6.5 / 2 + 4 * 2", None).unwrap();
        assert_eq!(result, Value::Number(11.25));
    }

    #[test]
    fn exponentiation_with_nested_expression() {
        let result = parse_and_evaluate("2 ^ (1 + 2)", None).unwrap();
        assert_eq!(result, Value::Number(8.0));
    }

    #[test]
    fn complex_expression_with_decimals() {
        let result = parse_and_evaluate("7.5 - 3.25 + 2 * (8 / 4)", None).unwrap();
        assert_eq!(result, Value::Number(8.25));
    }

    #[test]
    fn subtraction_with_decimal_result() {
        let result = parse_and_evaluate("10.75 - 3.5", None).unwrap();
        assert_eq!(result, Value::Number(7.25));
    }

    #[test]
    fn multiplication_of_two_decimals() {
        let result = parse_and_evaluate("3.5 * 2.0", None).unwrap();
        assert_eq!(result, Value::Number(7.0));
    }

    #[test]
    fn division_of_two_decimals() {
        let result = parse_and_evaluate("7.5 / 2.5", None).unwrap();
        assert_eq!(result, Value::Number(3.0));
    }

    #[test]
    fn boolean_and() {
        let result = parse_and_evaluate("true and false", None).unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn boolean_and_alt() {
        let result = parse_and_evaluate("true && false", None).unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn boolean_or() {
        let result = parse_and_evaluate("true or false", None).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn boolean_or_alt() {
        let result = parse_and_evaluate("true || false", None).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn boolean_and_with_nested_expression() {
        let result = parse_and_evaluate("true and (false or true)", None).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn boolean_and_with_nested_expression_alt() {
        let result = parse_and_evaluate("true && (false || true)", None).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn boolean_or_with_nested_expression() {
        let result = parse_and_evaluate("true or (false and true)", None).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn boolean_or_with_nested_expression_alt() {
        let result = parse_and_evaluate("true || (false && true)", None).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn logical_not() {
        let result = parse_and_evaluate("!true", None).unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn logical_not_with_nested_expression() {
        let result = parse_and_evaluate("!(true and false)", None).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn equality_of_two_integers() {
        let result = parse_and_evaluate("5 == 5", None).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn inequality_of_two_integers() {
        let result = parse_and_evaluate("5 != 5", None).unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn less_than_comparison() {
        let result = parse_and_evaluate("5 < 10", None).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn less_than_or_equal_comparison() {
        let result = parse_and_evaluate("5 <= 5", None).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn greater_than_comparison() {
        let result = parse_and_evaluate("10 > 5", None).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn greater_than_or_equal_comparison() {
        let result = parse_and_evaluate("5 >= 5", None).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn equality_of_two_lists() {
        let result = parse_and_evaluate("[1, 2, 3] == [1, 2, 3]", None).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn inequality_of_two_lists() {
        let result = parse_and_evaluate("[1, 2, 3] != [2, 3, 4]", None).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn conditional_expression_with_true_condition() {
        let result = parse_and_evaluate("if true then 5 else 10", None).unwrap();
        assert_eq!(result, Value::Number(5.0));
    }

    #[test]
    fn conditional_expression_with_false_condition() {
        let result = parse_and_evaluate("if false then 5 else 10", None).unwrap();
        assert_eq!(result, Value::Number(10.0));
    }

    #[test]
    fn factorial_of_integer() {
        let result = parse_and_evaluate("5!", None).unwrap();
        assert_eq!(result, Value::Number(120.0));
    }

    #[test]
    fn factorial_of_zero() {
        let result = parse_and_evaluate("0!", None).unwrap();
        assert_eq!(result, Value::Number(1.0));
    }

    #[test]
    fn factorial_of_negative_integer() {
        let result = parse_and_evaluate("(-5)!", None);
        assert!(result.is_err());
    }

    #[test]
    fn factorial_of_decimal() {
        let result = parse_and_evaluate("5.5!", None);
        assert!(result.is_err());
    }

    #[test]
    fn string_concatenation() {
        let result = parse_and_evaluate("\"hello\" + \"world\"", None).unwrap();
        assert_eq!(result, Value::String("helloworld".to_string()));
    }

    #[test]
    fn string_concatenation_with_integer() {
        let result = parse_and_evaluate("\"hello\" + 5", None);
        assert!(result.is_err());
    }

    #[test]
    fn variable_assignment() {
        let vars = Rc::new(RefCell::new(HashMap::new()));
        let result = parse_and_evaluate("x = 5", Some(Rc::clone(&vars))).unwrap();
        assert_eq!(result, Value::Number(5.0));
        assert_eq!(vars.borrow().get("x").unwrap(), &Value::Number(5.0));
    }

    #[test]
    fn variable_assignment_with_expression() {
        let vars = Rc::new(RefCell::new(HashMap::new()));
        let result = parse_and_evaluate("x = 5 + 2", Some(Rc::clone(&vars))).unwrap();
        assert_eq!(result, Value::Number(7.0));
        assert_eq!(vars.borrow().get("x").unwrap(), &Value::Number(7.0));
    }

    #[test]
    fn variable_assignment_with_lambda() {
        let vars = Rc::new(RefCell::new(HashMap::new()));
        let result = parse_and_evaluate("f = x => x + 1", Some(Rc::clone(&vars))).unwrap();

        assert_eq!(
            result,
            Value::Lambda(LambdaDef {
                name: Some("f".to_string()),
                args: vec![LambdaArg::Required("x".to_string())],
                body: "x + 1".to_string(),
            })
        );
        assert_eq!(
            vars.borrow().get("f").unwrap(),
            &Value::Lambda(LambdaDef {
                name: Some("f".to_string()),
                args: vec![LambdaArg::Required("x".to_string())],
                body: "x + 1".to_string(),
            })
        );
    }

    #[test]
    fn variable_assignment_with_lambda_and_call() {
        let vars = Rc::new(RefCell::new(HashMap::new()));
        let _ = parse_and_evaluate("f = x => x + 1", Some(Rc::clone(&vars))).unwrap();
        let result = parse_and_evaluate("f(5)", Some(Rc::clone(&vars))).unwrap();

        assert_eq!(result, Value::Number(6.0));
    }

    #[test]
    fn variable_assignment_with_lambda_and_call_with_multiple_args() {
        let vars = Rc::new(RefCell::new(HashMap::new()));
        let _ = parse_and_evaluate("f = (x, y) => x + y", Some(Rc::clone(&vars))).unwrap();
        let result = parse_and_evaluate("f(5, 2)", Some(Rc::clone(&vars))).unwrap();

        assert_eq!(result, Value::Number(7.0));
    }

    #[test]
    fn variable_assignment_with_lambda_and_call_with_multiple_args_and_expression() {
        let vars = Rc::new(RefCell::new(HashMap::new()));
        let _ = parse_and_evaluate("f = (x, y) => x + y", Some(Rc::clone(&vars))).unwrap();
        let result = parse_and_evaluate("f(5, 2) + 3", Some(Rc::clone(&vars))).unwrap();

        assert_eq!(result, Value::Number(10.0));
    }

    #[test]
    fn coalesce_operator_with_null() {
        let result = parse_and_evaluate("null ?? 5", None).unwrap();
        assert_eq!(result, Value::Number(5.0));
    }

    #[test]
    fn coalesce_operator_with_non_null() {
        let result = parse_and_evaluate("3 ?? 10", None).unwrap();
        assert_eq!(result, Value::Number(3.0));
    }
}
