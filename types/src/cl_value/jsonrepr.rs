use alloc::{string::String, vec, vec::Vec};

use serde::Serialize;
use serde_json::{json, Value};

use crate::{
    bytesrepr::{self, FromBytes, OPTION_NONE_TAG, OPTION_SOME_TAG, RESULT_ERR_TAG, RESULT_OK_TAG},
    check_summed_hex, CLType, CLValue, Key, PublicKey, URef, U128, U256, U512,
};

/// Returns a best-effort attempt to convert the `CLValue` into a meaningful JSON value.
pub fn cl_value_to_json(cl_value: &CLValue) -> Option<Value> {
    to_json(&cl_value.cl_type(), cl_value.inner_bytes()).and_then(|(json_value, remainder)| {
        if remainder.is_empty() {
            Some(json_value)
        } else {
            None
        }
    })
}

fn to_json<'a>(cl_type: &CLType, bytes: &'a [u8]) -> Option<(Value, &'a [u8])> {
    match cl_type {
        CLType::Bool => simple_type_to_json::<bool>(bytes),
        CLType::I32 => simple_type_to_json::<i32>(bytes),
        CLType::I64 => simple_type_to_json::<i64>(bytes),
        CLType::U8 => simple_type_to_json::<u8>(bytes),
        CLType::U32 => simple_type_to_json::<u32>(bytes),
        CLType::U64 => simple_type_to_json::<u64>(bytes),
        CLType::U128 => simple_type_to_json::<U128>(bytes),
        CLType::U256 => simple_type_to_json::<U256>(bytes),
        CLType::U512 => simple_type_to_json::<U512>(bytes),
        CLType::Unit => simple_type_to_json::<()>(bytes),
        CLType::String => simple_type_to_json::<String>(bytes),
        CLType::Key => simple_type_to_json::<Key>(bytes),
        CLType::URef => simple_type_to_json::<URef>(bytes),
        CLType::PublicKey => simple_type_to_json::<PublicKey>(bytes),
        CLType::Option(inner_cl_type) => {
            let (variant, remainder) = u8::from_bytes(bytes).ok()?;
            match variant {
                OPTION_NONE_TAG => Some((Value::Null, remainder)),
                OPTION_SOME_TAG => Some(to_json(inner_cl_type, remainder)?),
                _ => None,
            }
        }
        CLType::List(inner_cl_type) => {
            let (count, mut stream) = u32::from_bytes(bytes).ok()?;
            let mut result: Vec<Value> = Vec::new();
            for _ in 0..count {
                let (value, remainder) = to_json(inner_cl_type, &stream)?;
                result.push(value);
                stream = remainder;
            }
            Some((json!(result), stream))
        }
        CLType::ByteArray(length) => {
            let (bytes, remainder) = bytesrepr::safe_split_at(bytes, *length as usize).ok()?;
            Some((json![check_summed_hex::encode(bytes)], remainder))
        }
        CLType::Result { ok, err } => {
            let (variant, remainder) = u8::from_bytes(bytes).ok()?;
            match variant {
                RESULT_ERR_TAG => {
                    let (value, remainder) = to_json(err, remainder)?;
                    Some((json!({ "Err": value }), remainder))
                }
                RESULT_OK_TAG => {
                    let (value, remainder) = to_json(ok, remainder)?;
                    Some((json!({ "Ok": value }), remainder))
                }
                _ => None,
            }
        }
        CLType::Map { key, value } => {
            let (num_keys, mut stream) = u32::from_bytes(bytes).unwrap();
            let mut result: Vec<Value> = Vec::new();
            for _ in 0..num_keys {
                let (k, remainder) = to_json(key, stream)?;
                let (v, remainder) = to_json(value, remainder)?;
                result.push(json!({"key": k, "value": v}));
                stream = remainder;
            }
            Some((json!(result), stream))
        }
        CLType::Tuple1(arr) => {
            let (t1, remainder) = to_json(&arr[0], bytes)?;
            Some((json!([t1]), remainder))
        }
        CLType::Tuple2(arr) => {
            let (t1, remainder) = to_json(&arr[0], bytes)?;
            let (t2, remainder) = to_json(&arr[1], remainder)?;
            Some((json!([t1, t2]), remainder))
        }
        CLType::Tuple3(arr) => {
            let (t1, remainder) = to_json(&arr[0], bytes)?;
            let (t2, remainder) = to_json(&arr[1], remainder)?;
            let (t3, remainder) = to_json(&arr[2], remainder)?;
            Some((json!([t1, t2, t3]), remainder))
        }
        CLType::Any => None,
    }
}

fn simple_type_to_json<T: FromBytes + Serialize>(bytes: &[u8]) -> Option<(Value, &[u8])> {
    let (value, remainder) = T::from_bytes(bytes).ok()?;
    Some((json!(value), remainder))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bytesrepr::ToBytes, AsymmetricType, CLTyped, SecretKey};
    use alloc::collections::BTreeMap;

    fn test_value<T: ToBytes + Serialize + Clone + CLTyped>(value: T) {
        let cl_value = CLValue::from_t(value.clone()).unwrap();
        let cl_value_as_json: Value = cl_value_to_json(&cl_value).unwrap();
        let expected = json!(value);
        assert_eq!(cl_value_as_json, expected);
    }

    #[test]
    fn list_of_ints_to_json_value() {
        test_value::<Vec<i32>>(vec![]);
        test_value(vec![10u32, 12u32]);
    }

    #[test]
    fn list_of_bools_to_json_value() {
        test_value(vec![true, false]);
    }

    #[test]
    fn list_of_string_to_json_value() {
        test_value(vec!["rust", "python"]);
    }

    #[test]
    fn list_of_public_keys_to_json_value() {
        let a = PublicKey::from(
            &SecretKey::secp256k1_from_bytes([3; SecretKey::SECP256K1_LENGTH]).unwrap(),
        );
        let b = PublicKey::from(
            &SecretKey::ed25519_from_bytes([3; SecretKey::ED25519_LENGTH]).unwrap(),
        );
        let a_hex = a.to_hex();
        let b_hex = b.to_hex();
        let cl_value = CLValue::from_t(vec![a, b]).unwrap();
        let cl_value_as_json: Value = cl_value_to_json(&cl_value).unwrap();
        let expected = json!([a_hex, b_hex]);
        assert_eq!(cl_value_as_json, expected);
    }

    #[test]
    fn list_of_list_of_public_keys_to_json_value() {
        let a = PublicKey::from(
            &SecretKey::secp256k1_from_bytes([3; SecretKey::SECP256K1_LENGTH]).unwrap(),
        );
        let b = PublicKey::from(
            &SecretKey::ed25519_from_bytes([3; PublicKey::ED25519_LENGTH]).unwrap(),
        );
        let c = PublicKey::from(
            &SecretKey::ed25519_from_bytes([6; PublicKey::ED25519_LENGTH]).unwrap(),
        );
        let a_hex = a.to_hex();
        let b_hex = b.to_hex();
        let c_hex = c.to_hex();
        let cl_value = CLValue::from_t(vec![vec![a, b], vec![c]]).unwrap();
        let cl_value_as_json: Value = cl_value_to_json(&cl_value).unwrap();
        let expected = json!([[a_hex, b_hex], [c_hex]]);
        assert_eq!(cl_value_as_json, expected);
    }

    #[test]
    fn map_of_string_to_list_of_ints_to_json_value() {
        let key1 = String::from("first");
        let key2 = String::from("second");
        let value1 = vec![];
        let value2 = vec![1, 2, 3];
        let mut map: BTreeMap<String, Vec<i32>> = BTreeMap::new();
        map.insert(key1.clone(), value1.clone());
        map.insert(key2.clone(), value2.clone());
        let cl_value = CLValue::from_t(map).unwrap();
        let cl_value_as_json: Value = cl_value_to_json(&cl_value).unwrap();
        let expected = json!([
            { "key": key1, "value": value1 },
            { "key": key2, "value": value2 }
        ]);
        assert_eq!(cl_value_as_json, expected);
    }

    #[test]
    fn option_some_of_lists_to_json_value() {
        test_value(Some(vec![1, 2, 3]));
    }

    #[test]
    fn option_none_to_json_value() {
        test_value(Option::<i32>::None);
    }

    #[test]
    fn bytes_to_json_value() {
        let bytes = [1_u8, 2];
        let cl_value = CLValue::from_t(bytes).unwrap();
        let cl_value_as_json = cl_value_to_json(&cl_value).unwrap();
        let expected = json!(check_summed_hex::encode(&bytes));
        assert_eq!(cl_value_as_json, expected);
    }

    #[test]
    fn result_ok_to_json_value() {
        test_value(Result::<Vec<i32>, String>::Ok(vec![1, 2, 3]));
    }

    #[test]
    fn result_error_to_json_value() {
        test_value(Result::<Vec<i32>, String>::Err(String::from("Upsss")));
    }

    #[test]
    fn tuples_to_json_value() {
        let v1 = String::from("Hello");
        let v2 = vec![1, 2, 3];
        let v3 = 1u8;

        test_value((v1.clone(),));
        test_value((v1.clone(), v2.clone()));
        test_value((v1, v2, v3));
    }
}
