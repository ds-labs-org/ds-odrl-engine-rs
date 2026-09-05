//! How a probe carries a field the engine's own typed `Request` cannot
//! hold.
//!
//! Most of this catalog's *negative* probes assert that some real ODRL
//! property is inert — `odrl:conflict`, a per-permission `odrl:duty`, a
//! per-rule `odrl:target`, `odrl:andSequence`, an out-of-enum operator
//! token. None of those is a field on `engine::Request`, by definition:
//! the whole claim is that the wire contract has no such field. So a probe
//! is authored as a typed `engine::Request` (which guarantees the
//! *supported* half of the request is exactly the shape Section 5.2
//! documents, checked by the compiler), serialized through
//! `serde_json::to_value`, and then **patched** — the one place unknown
//! JSON keys enter, and the only place they can.
//!
//! [`apply_patches`] is deliberately strict: a pointer that does not
//! resolve, resolves to something that is not a JSON object, or a
//! `Remove` naming a key that is not there, is an error that fails
//! `cargo run -p coverage-probes` outright. The failure mode this guards
//! against is a typo'd pointer silently producing a request without the
//! injected key at all — which would still *pass* its probe (the expected
//! decision is the one the un-patched request produces anyway, that being
//! the whole point of an "this property is inert" probe), while proving
//! nothing whatsoever.

use serde_json::Value;

/// Insert-or-replace one key on the object at `pointer`, or remove one.
/// Nothing in the catalog needs any other operation, so no other operation
/// exists.
#[derive(Debug, Clone, PartialEq)]
pub enum Patch {
    Set { pointer: String, key: String, value: Value },
    Remove { pointer: String, key: String },
}

impl Patch {
    /// `pointer` is an RFC 6901 JSON Pointer into the request (`""` for
    /// the request object itself, `/policies/0` for the first policy, and
    /// so on).
    pub fn set(pointer: &str, key: &str, value: Value) -> Self {
        Patch::Set { pointer: pointer.to_string(), key: key.to_string(), value }
    }

    pub fn remove(pointer: &str, key: &str) -> Self {
        Patch::Remove { pointer: pointer.to_string(), key: key.to_string() }
    }

    fn pointer(&self) -> &str {
        match self {
            Patch::Set { pointer, .. } | Patch::Remove { pointer, .. } => pointer,
        }
    }

    fn key(&self) -> &str {
        match self {
            Patch::Set { key, .. } | Patch::Remove { key, .. } => key,
        }
    }
}

fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Applies every patch in order, in place. Errors — never silently
/// no-ops — on a pointer that does not resolve, a pointer that resolves to
/// a non-object, or a `Remove` of a key that is not present.
pub fn apply_patches(value: &mut Value, patches: &[Patch]) -> Result<(), String> {
    for patch in patches {
        let pointer = patch.pointer().to_string();
        let key = patch.key().to_string();

        let target = value
            .pointer_mut(&pointer)
            .ok_or_else(|| format!("patch pointer `{pointer}` does not resolve in this request"))?;
        let kind = kind_of(target);
        let object = target
            .as_object_mut()
            .ok_or_else(|| format!("patch pointer `{pointer}` resolves to a {kind}, not an object"))?;

        match patch {
            Patch::Set { value: new_value, .. } => {
                object.insert(key, new_value.clone());
            }
            Patch::Remove { .. } => {
                if object.remove(&key).is_none() {
                    return Err(format!("patch pointer `{pointer}` carries no key `{key}` to remove"));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn document() -> Value {
        json!({
            "policies": [ { "id": "probe", "permissions": [ { "action": "use" } ] } ],
            "config": { "behaviour": "closed" }
        })
    }

    #[test]
    fn set_inserts_a_key_the_typed_request_could_never_carry() {
        let mut value = document();
        apply_patches(&mut value, &[Patch::set("/policies/0", "conflict", json!("perm"))]).unwrap();
        assert_eq!(value["policies"][0]["conflict"], json!("perm"));
    }

    #[test]
    fn set_replaces_an_existing_key_rather_than_duplicating_it() {
        let mut value = document();
        apply_patches(&mut value, &[Patch::set("/config", "behaviour", json!("default"))]).unwrap();
        assert_eq!(value["config"]["behaviour"], json!("default"));
        assert_eq!(value["config"].as_object().unwrap().len(), 1);
    }

    #[test]
    fn set_reaches_the_request_root_through_the_empty_pointer() {
        let mut value = document();
        apply_patches(&mut value, &[Patch::set("", "inheritFrom", json!("parent"))]).unwrap();
        assert_eq!(value["inheritFrom"], json!("parent"));
    }

    #[test]
    fn remove_deletes_a_present_key() {
        let mut value = document();
        apply_patches(&mut value, &[Patch::remove("/config", "behaviour")]).unwrap();
        assert!(value["config"].get("behaviour").is_none());
    }

    #[test]
    fn a_pointer_that_does_not_resolve_is_an_error_not_a_silent_no_op() {
        // The exact failure this guards: a typo'd pointer would otherwise
        // leave the request un-patched, and an "this property is inert"
        // probe would still reach its expected decision while having
        // injected nothing at all.
        let mut value = document();
        let err = apply_patches(&mut value, &[Patch::set("/policies/7", "conflict", json!("perm"))]).unwrap_err();
        assert!(err.contains("does not resolve"), "{err}");
    }

    #[test]
    fn a_pointer_resolving_to_a_non_object_is_an_error_naming_what_it_found() {
        let mut value = document();
        let err = apply_patches(&mut value, &[Patch::set("/policies", "conflict", json!("perm"))]).unwrap_err();
        assert!(err.contains("resolves to a array"), "{err}");

        let err = apply_patches(&mut value, &[Patch::set("/policies/0/id", "x", json!(1))]).unwrap_err();
        assert!(err.contains("resolves to a string"), "{err}");
    }

    #[test]
    fn removing_an_absent_key_is_an_error_too() {
        let mut value = document();
        let err = apply_patches(&mut value, &[Patch::remove("/config", "dutyMode")]).unwrap_err();
        assert!(err.contains("carries no key `dutyMode`"), "{err}");
    }

    #[test]
    fn patches_apply_in_order_so_a_later_one_sees_an_earlier_ones_object() {
        let mut value = document();
        apply_patches(&mut value, &[
            Patch::set("/policies/0", "duty", json!({})),
            Patch::set("/policies/0/duty", "action", json!("compensate")),
        ])
        .unwrap();
        assert_eq!(value["policies"][0]["duty"]["action"], json!("compensate"));
    }
}
