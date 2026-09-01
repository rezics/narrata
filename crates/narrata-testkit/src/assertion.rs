#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Difference {
    pub path: String,
    pub expected: String,
    pub actual: String,
}

pub fn first_json_difference(
    expected: &serde_json::Value,
    actual: &serde_json::Value,
) -> Option<Difference> {
    compare("$", expected, actual)
}

fn compare(
    path: &str,
    expected: &serde_json::Value,
    actual: &serde_json::Value,
) -> Option<Difference> {
    match (expected, actual) {
        (serde_json::Value::Array(left), serde_json::Value::Array(right)) => {
            for index in 0..left.len().max(right.len()) {
                match (left.get(index), right.get(index)) {
                    (Some(left), Some(right)) => {
                        if let Some(difference) = compare(&format!("{path}[{index}]"), left, right)
                        {
                            return Some(difference);
                        }
                    }
                    _ => return Some(difference(path, expected, actual)),
                }
            }
            None
        }
        (serde_json::Value::Object(left), serde_json::Value::Object(right)) => {
            let mut keys = left.keys().chain(right.keys()).collect::<Vec<_>>();
            keys.sort_unstable();
            keys.dedup();
            for key in keys {
                match (left.get(key), right.get(key)) {
                    (Some(left), Some(right)) => {
                        if let Some(difference) = compare(&format!("{path}.{key}"), left, right) {
                            return Some(difference);
                        }
                    }
                    _ => return Some(difference(&format!("{path}.{key}"), expected, actual)),
                }
            }
            None
        }
        _ if expected == actual => None,
        _ => Some(difference(path, expected, actual)),
    }
}

fn difference(path: &str, expected: &serde_json::Value, actual: &serde_json::Value) -> Difference {
    Difference {
        path: path.to_owned(),
        expected: expected.to_string(),
        actual: actual.to_string(),
    }
}
