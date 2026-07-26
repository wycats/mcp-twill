use std::{cmp::Ordering, collections::BTreeSet, fmt};

use mcp_twill::{FrameworkError, Result};
use serde::de::{DeserializeSeed, Error as _, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use sha2::{Digest, Sha256};

const DUPLICATE_VERSION: &str = "mcp-twill duplicate top-level version";
const DUPLICATE_CONTEXT: &str = "mcp-twill duplicate context member";
const DUPLICATE_CONTRACT: &str = "mcp-twill duplicate contract member";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UniqueJsonError {
    Malformed,
    DuplicateVersion,
    DuplicateContext,
    DuplicateContract,
}

pub(crate) fn canonical_json(value: &Value) -> Result<Vec<u8>> {
    validate_ijson(value)?;
    let mut output = Vec::new();
    write_canonical_json(value, &mut output)?;
    Ok(output)
}

pub(crate) fn parse_unique_json(bytes: &[u8]) -> std::result::Result<Value, UniqueJsonError> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    deserializer.disable_recursion_limit();
    let value = UniqueValueSeed {
        depth: 0,
        in_context: false,
    }
    .deserialize(&mut deserializer)
    .map_err(classify_unique_json_error)?;
    deserializer.end().map_err(|_| UniqueJsonError::Malformed)?;
    Ok(value)
}

pub(crate) fn framed_snapshot_hash(domain: &str, version: u32, payload: &[u8]) -> String {
    let mut framed = Vec::with_capacity(domain.len() + payload.len() + 13);
    framed.extend_from_slice(domain.as_bytes());
    framed.push(0);
    framed.extend_from_slice(&version.to_be_bytes());
    framed.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    framed.extend_from_slice(payload);
    Sha256::digest(framed)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn build_error(message: impl Into<String>) -> FrameworkError {
    FrameworkError::Build(message.into())
}

fn validate_ijson(value: &Value) -> Result<()> {
    match value {
        Value::Number(number) => {
            let exact_integer = number
                .as_u64()
                .is_none_or(|integer| (integer as f64) as u64 == integer)
                && number
                    .as_i64()
                    .is_none_or(|integer| (integer as f64) as i64 == integer);
            if !exact_integer || number.as_f64().is_some_and(|number| !number.is_finite()) {
                return Err(build_error(
                    "host adapter number is outside the exact I-JSON domain",
                ));
            }
        }
        Value::Array(values) => {
            for value in values {
                validate_ijson(value)?;
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                validate_ijson(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn write_canonical_json(value: &Value, output: &mut Vec<u8>) -> Result<()> {
    match value {
        Value::Null => output.extend_from_slice(b"null"),
        Value::Bool(true) => output.extend_from_slice(b"true"),
        Value::Bool(false) => output.extend_from_slice(b"false"),
        Value::Number(number) => output.extend_from_slice(canonical_number(number)?.as_bytes()),
        Value::String(string) => output.extend_from_slice(
            serde_json::to_string(string)
                .map_err(|error| build_error(format!("cannot encode canonical string: {error}")))?
                .as_bytes(),
        ),
        Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_canonical_json(value, output)?;
            }
            output.push(b']');
        }
        Value::Object(values) => {
            output.push(b'{');
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| utf16_cmp(left.0, right.0));
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                output.extend_from_slice(
                    serde_json::to_string(key)
                        .map_err(|error| {
                            build_error(format!("cannot encode canonical key: {error}"))
                        })?
                        .as_bytes(),
                );
                output.push(b':');
                write_canonical_json(value, output)?;
            }
            output.push(b'}');
        }
    }
    Ok(())
}

fn canonical_number(number: &serde_json::Number) -> Result<String> {
    if let Some(value) = number.as_u64() {
        return Ok(value.to_string());
    }
    if let Some(value) = number.as_i64() {
        return Ok(value.to_string());
    }
    let value = number
        .to_string()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| build_error("host adapter number is outside the I-JSON domain"))?;
    if value == 0.0 {
        return Ok("0".to_string());
    }
    let magnitude = value.abs();
    let raw = (1..=17)
        .map(|digits| format!("{:.*e}", digits - 1, magnitude))
        .find(|candidate| {
            candidate
                .parse::<f64>()
                .is_ok_and(|parsed| parsed.to_bits() == magnitude.to_bits())
        })
        .ok_or_else(|| build_error("cannot canonicalize JSON number"))?;
    let negative = value.is_sign_negative();
    let Some((mantissa, exponent)) = raw.split_once(['e', 'E']) else {
        let mut fixed = raw;
        if fixed.contains('.') {
            while fixed.ends_with('0') {
                fixed.pop();
            }
            if fixed.ends_with('.') {
                fixed.pop();
            }
        }
        return Ok(if negative { format!("-{fixed}") } else { fixed });
    };
    let exponent = exponent
        .parse::<i32>()
        .map_err(|_| build_error("cannot canonicalize JSON number exponent"))?;
    let decimal_index = mantissa.find('.').unwrap_or(mantissa.len());
    let digits = mantissa.replace('.', "");
    let normalized_exponent = exponent + i32::try_from(decimal_index).unwrap_or(i32::MAX) - 1;
    let body = if (-6..=20).contains(&normalized_exponent) {
        let point = normalized_exponent + 1;
        if point <= 0 {
            format!("0.{}{}", "0".repeat((-point) as usize), digits)
        } else if point as usize >= digits.len() {
            format!("{}{}", digits, "0".repeat(point as usize - digits.len()))
        } else {
            let point = point as usize;
            format!("{}.{}", &digits[..point], &digits[point..])
        }
    } else {
        let fraction = if digits.len() == 1 {
            String::new()
        } else {
            format!(".{}", &digits[1..])
        };
        let sign = if normalized_exponent >= 0 { "+" } else { "" };
        format!("{}{fraction}e{sign}{normalized_exponent}", &digits[..1])
    };
    Ok(if negative { format!("-{body}") } else { body })
}

fn utf16_cmp(left: &str, right: &str) -> Ordering {
    left.encode_utf16().cmp(right.encode_utf16())
}

#[derive(Clone, Copy)]
struct UniqueValueSeed {
    depth: usize,
    in_context: bool,
}

impl<'de> DeserializeSeed<'de> for UniqueValueSeed {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(UniqueValueVisitor(self))
    }
}

struct UniqueValueVisitor(UniqueValueSeed);

impl<'de> Visitor<'de> for UniqueValueVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value with unique object member names")
    }

    fn visit_bool<E>(self, value: bool) -> std::result::Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E> {
        Ok(Value::String(value.to_string()))
    }

    fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        if self.0.depth >= 129 {
            return Err(A::Error::custom(if self.0.in_context {
                DUPLICATE_CONTEXT
            } else {
                DUPLICATE_CONTRACT
            }));
        }
        let mut values = Vec::new();
        let child = UniqueValueSeed {
            depth: self.0.depth + 1,
            in_context: self.0.in_context,
        };
        while let Some(value) = sequence.next_element_seed(child)? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        if self.0.depth >= 129 {
            return Err(A::Error::custom(if self.0.in_context {
                DUPLICATE_CONTEXT
            } else {
                DUPLICATE_CONTRACT
            }));
        }
        let mut values = serde_json::Map::new();
        let mut names = BTreeSet::new();
        while let Some(name) = map.next_key::<String>()? {
            if !names.insert(name.clone()) {
                let marker = if self.0.depth == 0 && name == "version" {
                    DUPLICATE_VERSION
                } else if self.0.in_context || (self.0.depth == 0 && name == "context") {
                    DUPLICATE_CONTEXT
                } else {
                    DUPLICATE_CONTRACT
                };
                return Err(A::Error::custom(marker));
            }
            let child = UniqueValueSeed {
                depth: self.0.depth + 1,
                in_context: self.0.in_context || (self.0.depth == 0 && name == "context"),
            };
            values.insert(name, map.next_value_seed(child)?);
        }
        Ok(Value::Object(values))
    }
}

fn classify_unique_json_error(error: serde_json::Error) -> UniqueJsonError {
    let message = error.to_string();
    if message.contains(DUPLICATE_VERSION) {
        UniqueJsonError::DuplicateVersion
    } else if message.contains(DUPLICATE_CONTEXT) {
        UniqueJsonError::DuplicateContext
    } else if message.contains(DUPLICATE_CONTRACT) {
        UniqueJsonError::DuplicateContract
    } else {
        UniqueJsonError::Malformed
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn canonical_json_matches_rfc_8785_key_and_number_spelling() {
        let value = json!({"😀": 1e30, "a": -0.0, "€": 0.000001});
        assert_eq!(
            String::from_utf8(canonical_json(&value).unwrap()).unwrap(),
            r#"{"a":0,"€":0.000001,"😀":1e+30}"#
        );
    }

    #[test]
    fn canonical_json_matches_rfc_8785_number_boundaries() {
        for (input, expected) in [
            ("333333333.33333329", "333333333.3333333"),
            ("1e30", "1e+30"),
            ("4.50", "4.5"),
            ("2e-3", "0.002"),
            ("1e-27", "1e-27"),
            ("1e20", "100000000000000000000"),
            ("1e21", "1e+21"),
            ("1e-6", "0.000001"),
            ("1e-7", "1e-7"),
            ("5e-324", "5e-324"),
            ("1.7976931348623157e308", "1.7976931348623157e+308"),
        ] {
            let value: Value = serde_json::from_str(input).unwrap();
            assert_eq!(
                String::from_utf8(canonical_json(&value).unwrap()).unwrap(),
                expected,
                "{input}"
            );
        }
    }

    #[test]
    fn canonical_json_accepts_only_exact_binary64_integers() {
        let exact = Value::Number(18_014_398_509_481_984_u64.into());
        assert_eq!(
            String::from_utf8(canonical_json(&exact).unwrap()).unwrap(),
            "18014398509481984"
        );

        let rounded = Value::Number(9_007_199_254_740_993_u64.into());
        assert!(canonical_json(&rounded).is_err());
    }

    #[test]
    fn duplicate_json_members_are_classified_before_map_construction() {
        assert_eq!(
            parse_unique_json(br#"{"version":1,"version":1}"#),
            Err(UniqueJsonError::DuplicateVersion)
        );
        assert_eq!(
            parse_unique_json(br#"{"version":1,"context":{"kind":"absent","kind":"unsupported"}}"#),
            Err(UniqueJsonError::DuplicateContext)
        );
        assert_eq!(
            parse_unique_json(br#"{"version":1,"arguments":{"id":"a","id":"b"}}"#),
            Err(UniqueJsonError::DuplicateContract)
        );
    }

    #[test]
    fn unique_json_parser_owns_the_logical_container_limit() {
        let envelope = |arrays: usize| {
            format!(
                r#"{{"version":1,"arguments":{{"value":{}null{}}}}}"#,
                "[".repeat(arrays),
                "]".repeat(arrays)
            )
        };
        assert!(parse_unique_json(envelope(127).as_bytes()).is_ok());
        assert_eq!(
            parse_unique_json(envelope(128).as_bytes()),
            Err(UniqueJsonError::DuplicateContract)
        );
    }
}
