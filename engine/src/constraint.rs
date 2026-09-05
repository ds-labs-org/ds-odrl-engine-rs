//! The leftOperand-to-claim mapping and supported operators (case study
//! Section 4.2, extended with date/time ordering operators and three more
//! set-based operators — see the `Operator` doc comment below for exactly
//! what each extension does and does not cover).

use serde::{Deserialize, Serialize};

use crate::claims::{ClaimValue, Claims};
use crate::temporal::parse_xsd_temporal_nanos;

/// The operators the Default Profile supports. `Eq`/`Neq`/`IsAnyOf` are
/// Section 4.2's original three, generic over any string-valued claim.
/// `Lt`/`Lteq`/`Gt`/`Gteq` are a later addition, closing both halves of
/// Section 7's "Numeric and date/time comparison operators ... remain
/// unimplemented" limitation. Each one dispatches per comparison
/// (`ordering_matches`, below) in a fixed order: first try both sides as a
/// recognized `xsd:dateTime`/`xsd:date` (`temporal::parse_xsd_temporal_nanos`
/// — the strict UTC `...Z` form, a numeric-UTC-offset `dateTime`, or a bare
/// `xsd:date` treated as midnight UTC), and only if that pairing fails —
/// either side not itself a recognized temporal value — fall back to
/// parsing both sides as a plain `f64` number. Temporal is tried first
/// because its lexical grammar (`-`/`:`/`T` separators, a trailing `Z` or
/// numeric offset) is strictly more specific than "looks like a number"
/// and is never itself ambiguous with one, so trying it first never steals
/// a genuine date away from the numeric path or vice versa. A value that
/// is neither a recognized temporal value nor a number on both sides is a
/// miss for these four operators, the same posture an absent claim key
/// already has for the original three.
///
/// `IsAllOf`/`IsNoneOf`/`IsPartOf` are a still later addition, closing out
/// the remaining set-based operators from the W3C ODRL 2.2 Vocabulary
/// (<https://www.w3.org/TR/odrl-vocab/>) alongside `IsAnyOf`. All three
/// reuse `IsAnyOf`'s own established, deliberate adaptation: the real
/// ODRL `rightOperand` for these operators is a JSON-LD list, but this
/// engine's `Constraint::right_operand` is a single `String` (Section 4.2
/// table), so each one treats `right_operand` as its own comma-delimited
/// list, carrying the same "no escaping convention" limitation `IsAnyOf`
/// already has (Section 7). See each variant's own doc comment for its
/// exact semantics — `IsPartOf`'s in particular states plainly what it
/// does *not* implement.
///
/// `Operator` also derives `Default` (`Eq` is the default) purely so
/// `Constraint`'s own `#[serde(default)]` `operator` field can deserialize
/// a purely-logical `{"odrl:and": [...]}` object that supplies no
/// `operator` at all — see `Constraint`'s own doc comment. That default is
/// never semantically read: `Constraint::evaluate` only ever consults
/// `operator` on the atomic path, which a logical constraint never takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Operator {
    #[default]
    #[serde(rename = "eq")]
    Eq,
    #[serde(rename = "neq")]
    Neq,
    #[serde(rename = "isAnyOf")]
    IsAnyOf,
    /// Satisfied when *every* element of `right_operand`'s comma-delimited
    /// list is present among the claim's own value(s) — a single-valued
    /// claim counts as a one-element set for this purpose
    /// (`ClaimValue::matches_all`). Per the W3C ODRL 2.2 Vocabulary,
    /// `isAllOf` is "a set-based operator indicating that a given value is
    /// all of the right operand of the Constraint"; this engine reads
    /// that the same direction `IsAnyOf` already reads its own spec text
    /// in: the claim's values must cover (be a superset of, or equal to)
    /// `right_operand`'s values. An absent claim key is a miss (not
    /// satisfied) — the same uniform posture every other operator here
    /// has for a missing `left_operand`, `IsNoneOf` (below) excepted.
    #[serde(rename = "isAllOf")]
    IsAllOf,
    /// Satisfied when *none* of `right_operand`'s comma-delimited elements
    /// are present among the claim's own value(s). Unlike every other
    /// operator in this enum, **an absent claim key satisfies `isNoneOf`**
    /// rather than missing: its own meaning is an exclusion ("this claim
    /// must not carry any of these values"), and a claim that is not
    /// present at all cannot carry a forbidden value, so there is nothing
    /// left to violate the exclusion. This is a deliberate, narrow
    /// divergence from the uniform "absent key is a miss" rule the rest of
    /// this enum follows (see `Constraint::evaluate`'s own doc comment) —
    /// and, to be precise about the one existing operator this could be
    /// confused with: `Neq` is also a negation, but `Neq`'s own absent-key
    /// case stays a miss (see this module's
    /// `missing_claim_key_is_a_miss_for_neq_too_not_the_negation_of_a_miss`
    /// test), so `IsNoneOf` does *not* mirror `Neq` here — it is its own,
    /// separately-justified exception.
    #[serde(rename = "isNoneOf")]
    IsNoneOf,
    /// This engine's own necessarily narrowed reading of ODRL's
    /// `isPartOf`. The real `isPartOf` is a range/hierarchy-membership
    /// test — the Vocabulary describes it as "a given value is contained
    /// by the right operand of the Constraint" (e.g. a point falling
    /// inside a named geographic region, or an instant falling inside a
    /// named period) — that this engine's flat, opaque string-claims model
    /// has no general way to evaluate: there is no notion here of one
    /// string "containing" another beyond set membership.
    /// **This is NOT general range/hierarchy membership.** As implemented,
    /// `IsPartOf` runs the exact same test as `IsAnyOf`
    /// (`ClaimValue::matches_any`): the claim value (or any element of a
    /// multi-valued claim) equals any element of `right_operand`'s comma
    /// list. It exists purely so a profile that wants to *name* its
    /// constraint `isPartOf` — because the modeled relationship really is
    /// "part of a named collection" even though this engine can only
    /// check that collection by flat, enumerated membership — gets an
    /// honestly-labeled operator instead of borrowing `isAnyOf`'s name for
    /// a different stated intent. An absent claim key is a miss, exactly
    /// as it is for `IsAnyOf`.
    #[serde(rename = "isPartOf")]
    IsPartOf,
    #[serde(rename = "lt")]
    Lt,
    #[serde(rename = "lteq")]
    Lteq,
    #[serde(rename = "gt")]
    Gt,
    #[serde(rename = "gteq")]
    Gteq,
}

/// `true` if `left` and `right` both parse as a recognized temporal value
/// (`temporal::parse_xsd_temporal_nanos`) and, ordered, satisfy
/// `satisfies` — or, only when that pairing fails (either side is not
/// itself a recognized temporal value), both parse as a plain, *finite*
/// `f64` and satisfy it that way instead. See `Operator`'s own doc comment
/// above for why temporal is tried first. Two lexical forms Rust's plain
/// `str::parse::<f64>` accepts are deliberately rejected here rather than
/// silently compared, both via the `is_finite()` guard: `NaN` (no
/// ordering relative to anything, itself included — `partial_cmp`
/// returning `None` already covered this case, `is_finite()` makes the
/// rejection explicit and covers the next case too) and `inf`/`-inf`/
/// `infinity` (every one of Rust's case-insensitive spellings) — without
/// this guard, a claim or right_operand of literally `"inf"` would make
/// `gt`/`gteq` vacuously match *every* finite number and `lt`/`lteq`
/// vacuously match none, in either direction, silently. This engine's
/// posture elsewhere is strict rejection of an edge-case lexical form
/// (a stray `+`/`-` in a fixed-width year field, an out-of-range UTC
/// offset) rather than tolerating it, and this fallback now matches that.
fn ordering_matches(left: &str, right: &str, satisfies: impl Fn(std::cmp::Ordering) -> bool) -> bool {
    if let (Some(l), Some(r)) = (parse_xsd_temporal_nanos(left), parse_xsd_temporal_nanos(right)) {
        return satisfies(l.cmp(&r));
    }
    if let (Ok(l), Ok(r)) = (left.parse::<f64>(), right.parse::<f64>()) {
        if l.is_finite() && r.is_finite() {
            if let Some(ordering) = l.partial_cmp(&r) {
                return satisfies(ordering);
            }
        }
    }
    false
}

/// `true` if `claim` (a single value, or — consistent with `Eq`/`IsAnyOf`
/// — any element of a multi-valued one) stands in `ordering` relative to
/// `right_operand`, per `ordering_matches` above. For a multi-valued claim
/// this is "any element satisfies" — the exact rule the original,
/// dateTime-only version of this function already used for `Lt`/`Lteq`/
/// `Gt`/`Gteq`, carried over unchanged (not reinvented) for the new
/// numeric fallback path, so both share one multi-valued semantics.
fn temporal_matches(claim: &ClaimValue, right_operand: &str, satisfies: impl Fn(std::cmp::Ordering) -> bool) -> bool {
    match claim {
        ClaimValue::Single(v) => ordering_matches(v, right_operand, satisfies),
        ClaimValue::Multi(vs) => vs.iter().any(|v| ordering_matches(v, right_operand, &satisfies)),
    }
}

/// The maximum nesting depth `Constraint::evaluate` will descend into a
/// logical (`odrl:and`/`odrl:or`/`odrl:xone`) tree before treating every
/// node past that bound as a deterministic non-match instead of recursing
/// further.
///
/// Unlike `ResolvedConfig::covers`'s `includedIn`-chain walk (`profile.rs`),
/// which guards against a genuine graph *cycle* using a `visited`
/// `HashSet` (there, "current action" is a reusable identifier two edges
/// can both point back to), a `Constraint` tree has no analogous notion of
/// node identity to dedupe on — each nested `Constraint` is owned by value
/// inside its parent's `Vec`, so a literal cycle (a node that is its own
/// ancestor) is not representable in memory here at all, the way it would
/// be through shared/interior-mutable references. What *is* representable,
/// and what this bound actually guards against, is pathological **depth**:
/// a JSON payload (or a directly-constructed `Constraint`, bypassing JSON
/// entirely) nesting `odrl:and`/`odrl:or`/`odrl:xone` far deeper than any
/// real policy would, which would otherwise grow `evaluate`'s call stack
/// unboundedly and could exhaust it — a concern sharpened by
/// `wasm32-unknown-unknown` guests, which often run with a far smaller
/// stack than a native host's default. 64 is chosen generously above any
/// nesting depth a real ODRL policy in this corpus (or the case study)
/// exercises, while still bounding the recursion to a small, fixed number
/// of stack frames regardless of input. `serde_json`'s own deserializer
/// additionally enforces a general (unrelated, larger) recursion limit
/// across all JSON container nesting, but that is not specific to
/// `Constraint`'s own tree shape and is not a substitute for this bound,
/// which also applies to a tree assembled directly by Rust code, never
/// having passed through JSON at all. See
/// `nesting_past_max_constraint_depth_is_a_deterministic_non_match_not_a_panic`
/// for the exact boundary behavior this produces.
pub const MAX_CONSTRAINT_DEPTH: usize = 64;

/// One ODRL constraint: either the original flat `left_operand`/
/// `operator`/`right_operand` test, or a logical grouping of nested
/// `Constraint`s under JSON-LD's own `odrl:and`/`odrl:or`/`odrl:xone` keys
/// (W3C ODRL 2.2, `odrl:LogicalConstraint`). This is an **additive**
/// wire-format change: `left_operand`, `operator` and `right_operand` are
/// exactly the fields this struct always had, at the same JSON keys —
/// `and`/`or`/`xone` are three new, optional fields, each renamed on the
/// wire to its own `odrl:`-namespaced key so a flat constraint (which
/// carries none of them) is indistinguishable on the wire from before
/// this phase. See
/// `flat_json_still_deserializes_identically_with_no_logical_fields_set`
/// and `serializing_an_atomic_constraint_round_trips_to_the_original_flat_shape`
/// below for the round-trip proof this design leans on rather than
/// assumes.
///
/// **Which case a given `Constraint` value is** is decided by
/// `evaluate`/`logical_children` at a fixed precedence — `xone`, then
/// `or`, then `and`, then (if none of the three is `Some`) the atomic
/// `left_operand`/`operator`/`right_operand` fields — never by more than
/// one at once in practice, since every constructor here (`new`, `and`,
/// `or`, `xone`) only ever sets one shape. A hand-written JSON object is
/// the one way to populate more than one simultaneously (see the design
/// notes below); this precedence order is what makes that case
/// deterministic rather than an error.
///
/// Design alternatives tried and rejected before this one, for the record
/// (per this phase's own instructions to show the work rather than assert
/// a particular serde attribute combination "just works"):
///
/// - **An untagged enum** (`Constraint::Atomic(AtomicConstraint)` alongside
///   `And`/`Or`/`Xone` variants, `#[serde(untagged)]`) was the first design
///   tried, and it *does* round-trip the wire shape correctly — but it
///   breaks source compatibility with existing Rust call sites outside
///   this crate that read `Constraint`'s fields directly by name (a real
///   one: `compliance-runner/src/translate.rs`'s own test helper reads
///   `c.left_operand` on a `Vec<Constraint>` `to_dnf` returns, since that
///   adapter's `to_dnf` always builds atomic constraints via
///   `Constraint::new`). An enum has no `.left_operand` field on the enum
///   type itself, only on one variant — rejected because this phase's own
///   ground rules require `compliance-runner/src/translate.rs` to be left
///   completely untouched, and "keep the struct's fields as they are" is
///   also this phase's own stated preferred design.
/// - **An internally-tagged enum** (`#[serde(tag = "...")]`) was rejected
///   for the same field-access reason as above, plus it requires one
///   shared discriminator field/value on every variant that a flat
///   constraint has never carried — adding one to every existing fixture
///   would itself be exactly the breaking rename this phase must avoid.
/// - **The chosen design**: keep this struct's original three fields
///   exactly as they were (same names, same JSON keys, same non-`Option`
///   types), and add three new `Option<Vec<Constraint>>` fields —
///   `and`/`or`/`xone` — each `#[serde(rename = "odrl:...")]` and
///   `#[serde(default, skip_serializing_if = "Option::is_none")]` so a
///   flat object (old JSON, or `Constraint::new`) never gains those keys
///   on the wire and a purely-logical object never needs to supply
///   `left_operand`/`operator`/`right_operand` at all. That last part is
///   the one genuine wrinkle this design has to pay for: `Operator` gains
///   a `Default` impl (`Eq`, arbitrarily — see `Operator`'s own doc
///   comment) purely so a `{"odrl:and": [...]}` object with none of the
///   three atomic fields present can still build a `Constraint` value.
///   Those defaulted atomic fields are never read when a logical field is
///   `Some` (the fixed precedence above), so the arbitrary default is
///   inert in practice, not a silent correctness gap — see
///   `a_logical_constraints_defaulted_atomic_fields_are_never_consulted`.
///
/// **`Deserialize` is hand-written, not derived — this is load-bearing,
/// not style.** An earlier version of this type derived `Deserialize`
/// with `#[serde(default)]` on all three atomic fields, so *any* object
/// with none of `left_operand`/`operator`/`right_operand`/`odrl:and`/
/// `odrl:or`/`odrl:xone` present — a typo'd key, a missing `odrl:`
/// prefix, `{}` — silently deserialized into an inert, always-`false`
/// atomic constraint instead of failing to parse. Before this type's
/// nested fields existed at all, that same malformed input was a hard
/// parse error (`left_operand`/`operator`/`right_operand` were all
/// required). That silent widening is a real regression in the wire
/// contract's error posture: a malformed prohibition constraint stopped
/// producing an `Error` response and started silently never matching
/// instead — fail-*open* for a prohibition specifically, the worst
/// direction for a mistake to fail in. The hand-written impl below
/// restores the original strictness for the atomic case (all three
/// fields genuinely required unless a logical field is present) while
/// still letting a purely logical object omit them — see
/// `a_constraint_object_missing_every_known_field_is_a_parse_error_not_an_inert_false`
/// and `an_atomic_field_present_alongside_operator_missing_is_still_a_parse_error`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Constraint {
    pub left_operand: String,
    pub operator: Operator,
    pub right_operand: String,
    /// `odrl:and`: satisfied when *every* nested child constraint
    /// evaluates to `true` — an empty list is vacuously satisfied, the
    /// same convention `Rule::matches`'s own empty-`constraints` case
    /// already uses.
    #[serde(rename = "odrl:and", skip_serializing_if = "Option::is_none")]
    pub and: Option<Vec<Constraint>>,
    /// `odrl:or`: satisfied when *at least one* nested child evaluates to
    /// `true` — an empty list is never satisfied (there is nothing to
    /// witness the "at least one").
    #[serde(rename = "odrl:or", skip_serializing_if = "Option::is_none")]
    pub or: Option<Vec<Constraint>>,
    /// `odrl:xone`: satisfied when *exactly one* nested child evaluates to
    /// `true` — the genuinely new capability this phase exists to add.
    /// Section 7 (and this repo's own README, before this change) named
    /// this precisely as impossible under `compliance-runner`'s host-side
    /// `and`/`or` DNF-expansion workaround: DNF can express "at least one
    /// of these disjuncts", never "exactly one, not more" — an `Or` of
    /// every pairwise combination still admits two-or-more children
    /// matching simultaneously. Both boundaries are wrong for `xone`: zero
    /// matching children is not satisfied (same as `Or`), but *two or
    /// more* matching children is **also** not satisfied, unlike `Or`.
    /// See `xone_is_satisfied_by_exactly_one_matching_child_not_zero_and_not_two_or_more`.
    #[serde(rename = "odrl:xone", skip_serializing_if = "Option::is_none")]
    pub xone: Option<Vec<Constraint>>,
}

/// The wire shape `Constraint::deserialize` actually parses into first —
/// every field optional, so serde can tell us what was and wasn't present
/// — before `Constraint`'s own `Deserialize` impl decides, from *which*
/// fields showed up, whether this was meant to be atomic (all three of
/// `left_operand`/`operator`/`right_operand` required) or logical (none
/// of them required). Kept private: nothing outside this module should
/// construct a `Constraint` from partially-known fields.
#[derive(Deserialize)]
struct RawConstraint {
    left_operand: Option<String>,
    operator: Option<Operator>,
    right_operand: Option<String>,
    #[serde(rename = "odrl:and", default)]
    and: Option<Vec<Constraint>>,
    #[serde(rename = "odrl:or", default)]
    or: Option<Vec<Constraint>>,
    #[serde(rename = "odrl:xone", default)]
    xone: Option<Vec<Constraint>>,
}

impl<'de> Deserialize<'de> for Constraint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawConstraint::deserialize(deserializer)?;
        let is_logical = raw.and.is_some() || raw.or.is_some() || raw.xone.is_some();
        if is_logical {
            // Atomic fields are never consulted once a logical field is
            // `Some` (the fixed xone > or > and precedence) — default
            // them rather than requiring a caller to write out
            // `"left_operand": ""` on every logical object.
            return Ok(Constraint {
                left_operand: raw.left_operand.unwrap_or_default(),
                operator: raw.operator.unwrap_or_default(),
                right_operand: raw.right_operand.unwrap_or_default(),
                and: raw.and,
                or: raw.or,
                xone: raw.xone,
            });
        }
        // No logical field present: this must be a complete atomic
        // constraint, exactly as strictly as before this type had any
        // logical fields to be confused with at all.
        Ok(Constraint {
            left_operand: raw
                .left_operand
                .ok_or_else(|| serde::de::Error::missing_field("left_operand"))?,
            operator: raw.operator.ok_or_else(|| serde::de::Error::missing_field("operator"))?,
            right_operand: raw
                .right_operand
                .ok_or_else(|| serde::de::Error::missing_field("right_operand"))?,
            and: None,
            or: None,
            xone: None,
        })
    }
}

impl Constraint {
    /// Builds a flat, atomic constraint — unchanged in meaning and call
    /// shape from before this phase; every existing call site in this
    /// workspace (`compliance-runner/src/translate.rs` included) keeps
    /// compiling and behaving identically against this constructor.
    pub fn new(
        left_operand: impl Into<String>,
        operator: Operator,
        right_operand: impl Into<String>,
    ) -> Self {
        Self {
            left_operand: left_operand.into(),
            operator,
            right_operand: right_operand.into(),
            and: None,
            or: None,
            xone: None,
        }
    }

    /// Builds an `odrl:and` logical constraint over `children`. The
    /// atomic fields are left at their (unused) defaults — see this
    /// type's own doc comment on why that is safe.
    pub fn and(children: Vec<Constraint>) -> Self {
        Self { and: Some(children), ..Self::new("", Operator::default(), "") }
    }

    /// Builds an `odrl:or` logical constraint over `children`.
    pub fn or(children: Vec<Constraint>) -> Self {
        Self { or: Some(children), ..Self::new("", Operator::default(), "") }
    }

    /// Builds an `odrl:xone` logical constraint over `children`.
    pub fn xone(children: Vec<Constraint>) -> Self {
        Self { xone: Some(children), ..Self::new("", Operator::default(), "") }
    }

    /// `true` when this is one of the three logical variants (`and`/`or`/
    /// `xone` is `Some`) rather than the flat atomic case.
    pub fn is_logical(&self) -> bool {
        self.and.is_some() || self.or.is_some() || self.xone.is_some()
    }

    /// Evaluates this constraint against `claims`.
    ///
    /// For the atomic case (`and`/`or`/`xone` all `None`): a `left_operand`
    /// absent from `claims` is a **miss, not an error** (Section 4.2) —
    /// this holds uniformly across every operator here, `neq` included: an
    /// absent claim does not satisfy `neq` merely because it fails to
    /// satisfy `eq`. The claims-map lookup, not the operator's own logic,
    /// decides the absent-key case for all of them *except* `IsNoneOf`,
    /// whose own doc comment explains why an absent key satisfies it
    /// instead — see the early return below.
    ///
    /// For a logical constraint, recurses into `and`/`or`/`xone`'s own
    /// children (see each field's own doc comment above for its exact
    /// semantics), at the fixed `xone` > `or` > `and` precedence this
    /// type's own doc comment names, and treats a node nested deeper than
    /// `MAX_CONSTRAINT_DEPTH` as a non-match rather than recursing past
    /// it — see that constant's own doc comment for why, and its exact
    /// boundary behavior.
    pub fn evaluate(&self, claims: &Claims) -> bool {
        self.evaluate_bounded(claims, 0)
    }

    fn evaluate_bounded(&self, claims: &Claims, depth: usize) -> bool {
        if depth > MAX_CONSTRAINT_DEPTH {
            return false;
        }
        if let Some(children) = &self.xone {
            return children.iter().filter(|c| c.evaluate_bounded(claims, depth + 1)).count() == 1;
        }
        if let Some(children) = &self.or {
            return children.iter().any(|c| c.evaluate_bounded(claims, depth + 1));
        }
        if let Some(children) = &self.and {
            return children.iter().all(|c| c.evaluate_bounded(claims, depth + 1));
        }

        let value = claims.get(&self.left_operand);

        if self.operator == Operator::IsNoneOf {
            let candidates: Vec<&str> = self.right_operand.split(',').collect();
            return match value {
                None => true,
                Some(value) => !value.matches_any(&candidates),
            };
        }

        let Some(value) = value else {
            return false;
        };

        use std::cmp::Ordering::{Greater, Less};
        match self.operator {
            Operator::Eq => value.matches(&self.right_operand),
            Operator::Neq => !value.matches(&self.right_operand),
            Operator::IsAnyOf | Operator::IsPartOf => {
                let candidates: Vec<&str> = self.right_operand.split(',').collect();
                value.matches_any(&candidates)
            }
            Operator::IsAllOf => {
                let candidates: Vec<&str> = self.right_operand.split(',').collect();
                value.matches_all(&candidates)
            }
            // Handled by the early return above; every other operator
            // needs `value` to be `Some`, which this arm cannot reach.
            Operator::IsNoneOf => unreachable!("IsNoneOf returns before this match"),
            Operator::Lt => temporal_matches(value, &self.right_operand, |o| o == Less),
            Operator::Lteq => temporal_matches(value, &self.right_operand, |o| o != Greater),
            Operator::Gt => temporal_matches(value, &self.right_operand, |o| o == Greater),
            Operator::Gteq => temporal_matches(value, &self.right_operand, |o| o != Less),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claims::ClaimValue;

    fn claims_with(pairs: &[(&str, ClaimValue)]) -> Claims {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn eq_matches_single_valued_claim() {
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        let constraint = Constraint::new("sub", Operator::Eq, "alice");
        assert!(constraint.evaluate(&claims));

        let constraint = Constraint::new("sub", Operator::Eq, "bob");
        assert!(!constraint.evaluate(&claims));
    }

    #[test]
    fn eq_matches_membership_in_multi_valued_claim() {
        let claims = claims_with(&[(
            "nationality",
            ClaimValue::Multi(vec!["FR".into(), "DE".into()]),
        )]);
        assert!(Constraint::new("nationality", Operator::Eq, "DE").evaluate(&claims));
        assert!(!Constraint::new("nationality", Operator::Eq, "US").evaluate(&claims));
    }

    #[test]
    fn neq_negates_eq_when_the_claim_is_present() {
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert!(Constraint::new("sub", Operator::Neq, "bob").evaluate(&claims));
        assert!(!Constraint::new("sub", Operator::Neq, "alice").evaluate(&claims));
    }

    #[test]
    fn missing_claim_key_is_a_miss_for_eq() {
        let claims = claims_with(&[]);
        assert!(!Constraint::new("sub", Operator::Eq, "alice").evaluate(&claims));
    }

    #[test]
    fn missing_claim_key_is_a_miss_for_neq_too_not_the_negation_of_a_miss() {
        let claims = claims_with(&[]);
        // The Section 4.2 posture is: absent key => not satisfied, full
        // stop, for every operator. It would be wrong (double-negative)
        // for `neq` to read a missing claim as satisfying itself just
        // because `eq` would have missed.
        assert!(!Constraint::new("sub", Operator::Neq, "alice").evaluate(&claims));
    }

    #[test]
    fn missing_claim_key_is_a_miss_for_is_any_of() {
        let claims = claims_with(&[]);
        assert!(!Constraint::new("scope", Operator::IsAnyOf, "read,write").evaluate(&claims));
    }

    #[test]
    fn is_any_of_splits_the_right_operand_on_commas() {
        let claims = claims_with(&[("scope", ClaimValue::Single("write".into()))]);
        assert!(Constraint::new("scope", Operator::IsAnyOf, "read,write,delete").evaluate(&claims));
        assert!(!Constraint::new("scope", Operator::IsAnyOf, "read,delete").evaluate(&claims));
    }

    #[test]
    fn is_any_of_matches_any_element_of_a_multi_valued_claim() {
        let claims = claims_with(&[(
            "nationality",
            ClaimValue::Multi(vec!["FR".into(), "DE".into()]),
        )]);
        assert!(Constraint::new("nationality", Operator::IsAnyOf, "US,DE,GB").evaluate(&claims));
        assert!(!Constraint::new("nationality", Operator::IsAnyOf, "US,GB").evaluate(&claims));
    }

    #[test]
    fn is_any_of_with_empty_right_operand_matches_nothing_but_an_empty_claim_value() {
        // Section 7's own documented limitation: no escaping convention,
        // so an empty string just splits into a single empty candidate.
        let claims = claims_with(&[("scope", ClaimValue::Single("read".into()))]);
        assert!(!Constraint::new("scope", Operator::IsAnyOf, "").evaluate(&claims));

        let empty_claim = claims_with(&[("scope", ClaimValue::Single(String::new()))]);
        assert!(Constraint::new("scope", Operator::IsAnyOf, "").evaluate(&empty_claim));
    }

    #[test]
    fn eq_against_a_multi_valued_claim_does_not_match_a_concatenation() {
        // Type-mismatch edge case: the right_operand must equal one
        // element, not the claim's list "as if" it were a single string.
        let claims = claims_with(&[(
            "scope",
            ClaimValue::Multi(vec!["read".into(), "write".into()]),
        )]);
        assert!(!Constraint::new("scope", Operator::Eq, "read,write").evaluate(&claims));
        assert!(Constraint::new("scope", Operator::Eq, "read").evaluate(&claims));
    }

    #[test]
    fn lt_and_gt_compare_utc_datetimes_chronologically() {
        let claims = claims_with(&[("dateTime", ClaimValue::Single("2024-02-12T11:20:10.999Z".into()))]);
        assert!(Constraint::new("dateTime", Operator::Gt, "2024-01-01T00:00:00Z").evaluate(&claims));
        assert!(!Constraint::new("dateTime", Operator::Lt, "2024-01-01T00:00:00Z").evaluate(&claims));
        assert!(Constraint::new("dateTime", Operator::Lt, "2024-12-31T23:59:59Z").evaluate(&claims));
    }

    #[test]
    fn lteq_and_gteq_include_the_boundary() {
        let claims = claims_with(&[("dateTime", ClaimValue::Single("2024-01-01T00:00:00.000Z".into()))]);
        assert!(Constraint::new("dateTime", Operator::Gteq, "2024-01-01T00:00:00Z").evaluate(&claims));
        assert!(Constraint::new("dateTime", Operator::Lteq, "2024-01-01T00:00:00Z").evaluate(&claims));
        assert!(!Constraint::new("dateTime", Operator::Gt, "2024-01-01T00:00:00Z").evaluate(&claims));
    }

    #[test]
    fn ordering_operators_miss_on_an_unparseable_value_or_right_operand() {
        let claims = claims_with(&[("dateTime", ClaimValue::Single("not-a-date".into()))]);
        assert!(!Constraint::new("dateTime", Operator::Gt, "2024-01-01T00:00:00Z").evaluate(&claims));

        let claims = claims_with(&[("dateTime", ClaimValue::Single("2024-01-01T00:00:00Z".into()))]);
        assert!(!Constraint::new("dateTime", Operator::Gt, "not-a-date").evaluate(&claims));
    }

    // -- numeric comparison (Lt/Lteq/Gt/Gteq fallback) --------------------

    #[test]
    fn numeric_operators_compare_plain_integers_that_do_not_parse_as_a_datetime() {
        let claims = claims_with(&[("age", ClaimValue::Single("42".into()))]);
        assert!(Constraint::new("age", Operator::Gt, "18").evaluate(&claims));
        assert!(!Constraint::new("age", Operator::Lt, "18").evaluate(&claims));
        assert!(Constraint::new("age", Operator::Gteq, "42").evaluate(&claims));
        assert!(Constraint::new("age", Operator::Lteq, "42").evaluate(&claims));
        assert!(!Constraint::new("age", Operator::Gt, "42").evaluate(&claims));
    }

    #[test]
    fn numeric_operators_compare_negative_and_fractional_numbers_correctly() {
        let claims = claims_with(&[("balance", ClaimValue::Single("-3.5".into()))]);
        assert!(Constraint::new("balance", Operator::Lt, "-3.25").evaluate(&claims));
        assert!(Constraint::new("balance", Operator::Gt, "-10").evaluate(&claims));
        assert!(!Constraint::new("balance", Operator::Gt, "0").evaluate(&claims));
    }

    #[test]
    fn numeric_fallback_only_applies_once_the_temporal_parse_fails_on_either_side() {
        // "20240101" parses as neither xsd:date (wrong shape: no dashes)
        // nor xsd:dateTime, so it correctly falls through to the numeric
        // path and compares as the plain number 20240101.
        let claims = claims_with(&[("code", ClaimValue::Single("20240101".into()))]);
        assert!(Constraint::new("code", Operator::Gt, "20240100").evaluate(&claims));
        assert!(!Constraint::new("code", Operator::Lt, "20240100").evaluate(&claims));
    }

    #[test]
    fn a_numeric_looking_but_unparseable_value_misses_rather_than_panicking() {
        let claims = claims_with(&[("age", ClaimValue::Single("42abc".into()))]);
        assert!(!Constraint::new("age", Operator::Gt, "18").evaluate(&claims));
        assert!(!Constraint::new("age", Operator::Lt, "18").evaluate(&claims));

        // Right-hand side unparseable instead.
        let claims = claims_with(&[("age", ClaimValue::Single("42".into()))]);
        assert!(!Constraint::new("age", Operator::Gt, "eighteen").evaluate(&claims));
    }

    #[test]
    fn numeric_operators_miss_when_a_side_is_the_literal_string_nan() {
        // NaN has no ordering relative to anything, even another NaN --
        // `f64::partial_cmp` returns `None` for it either way.
        let claims = claims_with(&[("value", ClaimValue::Single("NaN".into()))]);
        assert!(!Constraint::new("value", Operator::Gt, "1").evaluate(&claims));
        assert!(!Constraint::new("value", Operator::Lt, "1").evaluate(&claims));
        assert!(!Constraint::new("value", Operator::Gteq, "NaN").evaluate(&claims));
        assert!(!Constraint::new("value", Operator::Lteq, "NaN").evaluate(&claims));
    }

    #[test]
    fn numeric_comparison_matches_any_element_of_a_multi_valued_claim_consistent_with_the_temporal_rule() {
        // Same "any element satisfies" rule the multi-valued dateTime path
        // already uses (see lt_and_gt_compare_utc_datetimes_chronologically
        // and this module's Multi handling in ordering_matches), applied
        // to the numeric fallback for consistency.
        let claims = claims_with(&[(
            "scores",
            ClaimValue::Multi(vec!["3".into(), "99".into()]),
        )]);
        assert!(Constraint::new("scores", Operator::Gt, "50").evaluate(&claims));
        assert!(!Constraint::new("scores", Operator::Gt, "100").evaluate(&claims));
    }

    // -- widened dateTime acceptance (xsd:date, numeric offsets) ----------

    #[test]
    fn ordering_operators_accept_a_bare_xsd_date_as_midnight_utc() {
        let claims = claims_with(&[("validFrom", ClaimValue::Single("2024-01-01".into()))]);
        assert!(Constraint::new("validFrom", Operator::Lt, "2024-01-01T00:00:01Z").evaluate(&claims));
        assert!(Constraint::new("validFrom", Operator::Lteq, "2024-01-01T00:00:00Z").evaluate(&claims));
        assert!(!Constraint::new("validFrom", Operator::Gt, "2024-01-01T00:00:00Z").evaluate(&claims));
    }

    #[test]
    fn ordering_operators_accept_a_numeric_utc_offset_instead_of_only_z() {
        let claims = claims_with(&[("dateTime", ClaimValue::Single("2024-01-01T02:00:00+02:00".into()))]);
        // Equivalent UTC instant is 2024-01-01T00:00:00Z.
        assert!(Constraint::new("dateTime", Operator::Gteq, "2024-01-01T00:00:00Z").evaluate(&claims));
        assert!(Constraint::new("dateTime", Operator::Lteq, "2024-01-01T00:00:00Z").evaluate(&claims));
        assert!(!Constraint::new("dateTime", Operator::Gt, "2024-01-01T00:00:00Z").evaluate(&claims));
    }

    #[test]
    fn ordering_operators_convert_an_offset_that_crosses_a_utc_day_boundary() {
        // 01:00 local at +05:00 is 2023-12-31T20:00:00Z -- the previous day.
        let claims = claims_with(&[("dateTime", ClaimValue::Single("2024-01-01T01:00:00+05:00".into()))]);
        assert!(Constraint::new("dateTime", Operator::Lt, "2023-12-31T23:00:00Z").evaluate(&claims));
        assert!(!Constraint::new("dateTime", Operator::Gt, "2023-12-31T23:00:00Z").evaluate(&claims));
    }

    #[test]
    fn ordering_operators_still_miss_on_a_z_datetime_compared_to_garbage() {
        // Confirms the widened dispatch doesn't accidentally start
        // succeeding on genuinely unparseable input via the numeric path.
        let claims = claims_with(&[("dateTime", ClaimValue::Single("2024-01-01T00:00:00Z".into()))]);
        assert!(!Constraint::new("dateTime", Operator::Gt, "not-a-date").evaluate(&claims));
    }

    #[test]
    fn deserializes_from_the_documented_json_shape() {
        let json = r#"{"left_operand":"nationality","operator":"isAnyOf","right_operand":"FR,DE"}"#;
        let constraint: Constraint = serde_json::from_str(json).unwrap();
        assert!(!constraint.is_logical());
        assert_eq!(constraint.left_operand, "nationality");
        assert_eq!(constraint.operator, Operator::IsAnyOf);
        assert_eq!(constraint.right_operand, "FR,DE");
    }

    // -- isAllOf --------------------------------------------------------

    #[test]
    fn is_all_of_matches_a_single_valued_claim_equal_to_the_lone_right_operand_element() {
        // Worked example: a single-valued claim can only satisfy isAllOf
        // when right_operand's list is (up to repeats) exactly that value.
        let claims = claims_with(&[("role", ClaimValue::Single("admin".into()))]);
        assert!(Constraint::new("role", Operator::IsAllOf, "admin").evaluate(&claims));
        assert!(!Constraint::new("role", Operator::IsAllOf, "admin,editor").evaluate(&claims));
    }

    #[test]
    fn is_all_of_requires_every_right_operand_element_in_a_multi_valued_claim() {
        // Worked example: scope carries read+write+delete; isAllOf(read,write)
        // is satisfied (both present), isAllOf(read,admin) is not (admin absent).
        let claims = claims_with(&[(
            "scope",
            ClaimValue::Multi(vec!["read".into(), "write".into(), "delete".into()]),
        )]);
        assert!(Constraint::new("scope", Operator::IsAllOf, "read,write").evaluate(&claims));
        assert!(Constraint::new("scope", Operator::IsAllOf, "delete,read,write").evaluate(&claims));
        assert!(!Constraint::new("scope", Operator::IsAllOf, "read,admin").evaluate(&claims));
    }

    #[test]
    fn missing_claim_key_is_a_miss_for_is_all_of() {
        let claims = claims_with(&[]);
        assert!(!Constraint::new("scope", Operator::IsAllOf, "read,write").evaluate(&claims));
    }

    #[test]
    fn is_all_of_with_empty_right_operand_matches_nothing_but_an_empty_claim_value() {
        // Same "no escaping convention" edge case IsAnyOf already has:
        // an empty right_operand splits into one empty-string candidate,
        // not zero candidates.
        let claims = claims_with(&[("scope", ClaimValue::Single("read".into()))]);
        assert!(!Constraint::new("scope", Operator::IsAllOf, "").evaluate(&claims));

        let empty_claim = claims_with(&[("scope", ClaimValue::Single(String::new()))]);
        assert!(Constraint::new("scope", Operator::IsAllOf, "").evaluate(&empty_claim));
    }

    // -- isNoneOf ---------------------------------------------------------

    #[test]
    fn is_none_of_is_satisfied_when_a_single_valued_claim_matches_none_of_the_list() {
        // Worked example: sub=alice does not appear in the banned-users list.
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert!(Constraint::new("sub", Operator::IsNoneOf, "bob,carol").evaluate(&claims));
        assert!(!Constraint::new("sub", Operator::IsNoneOf, "alice,bob").evaluate(&claims));
    }

    #[test]
    fn is_none_of_fails_as_soon_as_any_element_of_a_multi_valued_claim_is_excluded() {
        let claims = claims_with(&[(
            "nationality",
            ClaimValue::Multi(vec!["FR".into(), "DE".into()]),
        )]);
        assert!(Constraint::new("nationality", Operator::IsNoneOf, "US,GB").evaluate(&claims));
        assert!(!Constraint::new("nationality", Operator::IsNoneOf, "US,DE").evaluate(&claims));
    }

    #[test]
    fn missing_claim_key_is_vacuously_satisfied_for_is_none_of_unlike_every_other_operator() {
        // The one deliberate exception to "absent key is a miss" in this
        // module: isNoneOf is an exclusion, and there is nothing present
        // to violate an exclusion, so an absent claim key satisfies it.
        let claims = claims_with(&[]);
        assert!(Constraint::new("sub", Operator::IsNoneOf, "bob,carol").evaluate(&claims));
    }

    #[test]
    fn is_none_of_with_empty_right_operand_still_follows_the_empty_candidate_convention() {
        let claims = claims_with(&[("scope", ClaimValue::Single("read".into()))]);
        // "" splits to one empty-string candidate; "read" isn't it, so
        // none of the (one) candidates match => satisfied.
        assert!(Constraint::new("scope", Operator::IsNoneOf, "").evaluate(&claims));

        let empty_claim = claims_with(&[("scope", ClaimValue::Single(String::new()))]);
        // The claim's own value *is* the lone empty-string candidate, so
        // it is excluded => not satisfied.
        assert!(!Constraint::new("scope", Operator::IsNoneOf, "").evaluate(&empty_claim));
    }

    // -- isPartOf ---------------------------------------------------------

    #[test]
    fn is_part_of_matches_exactly_like_is_any_of_for_a_single_valued_claim() {
        // Worked example: identical to is_any_of_splits_the_right_operand_on_commas,
        // by design -- IsPartOf is a documented degenerate alias for flat
        // set membership, not general range/hierarchy containment.
        let claims = claims_with(&[("scope", ClaimValue::Single("write".into()))]);
        assert!(Constraint::new("scope", Operator::IsPartOf, "read,write,delete").evaluate(&claims));
        assert!(!Constraint::new("scope", Operator::IsPartOf, "read,delete").evaluate(&claims));
    }

    #[test]
    fn is_part_of_matches_any_element_of_a_multi_valued_claim() {
        let claims = claims_with(&[(
            "nationality",
            ClaimValue::Multi(vec!["FR".into(), "DE".into()]),
        )]);
        assert!(Constraint::new("nationality", Operator::IsPartOf, "US,DE,GB").evaluate(&claims));
        assert!(!Constraint::new("nationality", Operator::IsPartOf, "US,GB").evaluate(&claims));
    }

    #[test]
    fn missing_claim_key_is_a_miss_for_is_part_of() {
        let claims = claims_with(&[]);
        assert!(!Constraint::new("scope", Operator::IsPartOf, "read,write").evaluate(&claims));
    }

    #[test]
    fn is_part_of_with_empty_right_operand_matches_nothing_but_an_empty_claim_value() {
        let claims = claims_with(&[("scope", ClaimValue::Single("read".into()))]);
        assert!(!Constraint::new("scope", Operator::IsPartOf, "").evaluate(&claims));

        let empty_claim = claims_with(&[("scope", ClaimValue::Single(String::new()))]);
        assert!(Constraint::new("scope", Operator::IsPartOf, "").evaluate(&empty_claim));
    }

    #[test]
    fn deserializes_the_three_new_operators_from_their_documented_wire_names() {
        for (json_op, expected) in [
            ("isAllOf", Operator::IsAllOf),
            ("isNoneOf", Operator::IsNoneOf),
            ("isPartOf", Operator::IsPartOf),
        ] {
            let json = format!(
                r#"{{"left_operand":"scope","operator":"{json_op}","right_operand":"read,write"}}"#
            );
            let constraint: Constraint = serde_json::from_str(&json).unwrap();
            assert!(!constraint.is_logical());
            assert_eq!(constraint.operator, expected);
            assert_eq!(serde_json::to_string(&expected).unwrap(), format!("\"{json_op}\""));
        }
    }

    // -- logical nesting (odrl:and / odrl:or / odrl:xone) ------------------

    #[test]
    fn flat_json_still_deserializes_identically_with_no_logical_fields_set() {
        // The exact regression this phase's own instructions call for: an
        // existing fixture's exact JSON, byte for byte, still produces the
        // same value it always did -- `and`/`or`/`xone` all `None`, equal
        // in every field and identical in evaluation behavior.
        let json = r#"{"left_operand":"sub","operator":"eq","right_operand":"alice"}"#;
        let constraint: Constraint = serde_json::from_str(json).unwrap();
        assert_eq!(constraint, Constraint::new("sub", Operator::Eq, "alice"));
        assert!(!constraint.is_logical());
        assert_eq!(constraint.and, None);
        assert_eq!(constraint.or, None);
        assert_eq!(constraint.xone, None);

        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert!(constraint.evaluate(&claims));
    }

    #[test]
    fn serializing_an_atomic_constraint_round_trips_to_the_original_flat_shape() {
        // Confirms the wire representation itself, not just successful
        // parsing: the three new `Option` fields are `skip_serializing_if
        // = "Option::is_none"`, so an atomic constraint serializes back to
        // exactly the bare flat object -- no `odrl:and`/`odrl:or`/
        // `odrl:xone` key at all -- and a round trip through this type
        // never mutates an existing fixture's own JSON shape.
        let constraint = Constraint::new("nationality", Operator::IsAnyOf, "FR,DE");
        let json = serde_json::to_value(&constraint).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "left_operand": "nationality",
                "operator": "isAnyOf",
                "right_operand": "FR,DE"
            })
        );
    }

    #[test]
    fn a_logical_constraints_defaulted_atomic_fields_are_never_consulted() {
        // The one wrinkle this design's own doc comment names: a
        // purely-logical constraint (built via `Constraint::and`, or
        // deserialized from `{"odrl:and": [...]}` with no atomic fields
        // supplied) carries a defaulted `left_operand`/`right_operand`
        // (`""`) and `operator` (`Operator::Eq`, `Operator::default()`).
        // This confirms those defaults are genuinely inert: evaluation
        // never reads them once `and`/`or`/`xone` is `Some`.
        let constraint = Constraint::and(vec![Constraint::new("sub", Operator::Eq, "alice")]);
        assert_eq!(constraint.left_operand, "");
        assert_eq!(constraint.right_operand, "");
        assert_eq!(constraint.operator, Operator::Eq);

        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert!(
            constraint.evaluate(&claims),
            "evaluate must follow the `and` field, never fall through to an atomic eq(\"\", \"\") test"
        );
    }

    #[test]
    fn a_constraint_object_missing_every_known_field_is_a_parse_error_not_an_inert_false() {
        // The exact regression an adversarial review caught: before this
        // type had logical fields at all, `left_operand`/`operator`/
        // `right_operand` were plain required fields, so `{}` was a hard
        // parse error. A derived `Deserialize` with `#[serde(default)]`
        // on those three fields (this type's first cut at supporting
        // logical constraints) silently turned `{}` into an inert,
        // always-`false` atomic constraint instead — fail-*open* for a
        // prohibition specifically, since a malformed prohibition
        // constraint would then simply never match rather than surface as
        // an error. The hand-written `Deserialize` impl restores the
        // original strictness.
        assert!(serde_json::from_str::<Constraint>("{}").is_err());
        // A typo'd/mis-prefixed logical key must not be silently accepted
        // as "no logical field present, therefore atomic" either -- it's
        // simply an unknown key, and with no atomic fields present the
        // object still has none of the six known fields.
        let typo = r#"{"and": [{"left_operand": "sub", "operator": "eq", "right_operand": "alice"}]}"#;
        assert!(serde_json::from_str::<Constraint>(typo).is_err());
    }

    #[test]
    fn an_atomic_field_present_alongside_operator_missing_is_still_a_parse_error() {
        // Partial atomic input (some but not all of the three fields, and
        // no logical field to justify treating it as logical-with-unused-
        // defaults) must still fail, not silently default the missing
        // field.
        let json = r#"{"left_operand": "sub", "right_operand": "alice"}"#;
        assert!(serde_json::from_str::<Constraint>(json).is_err());
    }

    #[test]
    fn a_logical_object_may_omit_every_atomic_field_and_still_parse() {
        // The one case the hand-written Deserialize impl must keep
        // working exactly as the derived one did: a purely logical object
        // supplies none of `left_operand`/`operator`/`right_operand`.
        let json = r#"{"odrl:and": [{"left_operand": "sub", "operator": "eq", "right_operand": "alice"}]}"#;
        let constraint: Constraint = serde_json::from_str(json).unwrap();
        assert!(constraint.is_logical());
    }

    #[test]
    fn setting_more_than_one_logical_field_at_once_resolves_by_the_documented_xone_or_and_precedence() {
        // This type's own doc comment names hand-written JSON setting more
        // than one of `and`/`or`/`xone` simultaneously as the one
        // reachable way to have more than one shape at once, and says the
        // fixed `xone > or > and` precedence is what makes that
        // deterministic. An adversarial review found this claim untested
        // — this proves it against all three pairings, each rigged so the
        // two candidate branches would disagree if the wrong one won.
        let and_true_or_false = r#"{
            "odrl:and": [{"left_operand": "sub", "operator": "eq", "right_operand": "alice"}],
            "odrl:or": [{"left_operand": "sub", "operator": "eq", "right_operand": "nobody"}]
        }"#;
        let constraint: Constraint = serde_json::from_str(and_true_or_false).unwrap();
        let claims = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert!(
            !constraint.evaluate(&claims),
            "or must win over and: the rigged `or` branch (matching nobody) does not match, so \
             a true result here would mean `and` (which does match) was consulted instead"
        );

        let or_true_xone_false = r#"{
            "odrl:or": [{"left_operand": "sub", "operator": "eq", "right_operand": "alice"}],
            "odrl:xone": [
                {"left_operand": "sub", "operator": "eq", "right_operand": "alice"},
                {"left_operand": "sub", "operator": "eq", "right_operand": "alice"}
            ]
        }"#;
        let constraint: Constraint = serde_json::from_str(or_true_xone_false).unwrap();
        assert!(
            !constraint.evaluate(&claims),
            "xone must win over or: the rigged `xone` branch has two matching children (not \
             exactly one), so a true result here would mean `or` was consulted instead"
        );
    }

    #[test]
    fn ordering_operators_reject_infinity_on_either_side_rather_than_matching_vacuously() {
        // An adversarial review found that Rust's plain `str::parse::<f64>`
        // accepts "inf"/"-inf"/"infinity" (case-insensitively), which
        // would otherwise make `gt`/`gteq` match literally every finite
        // number and `lt`/`lteq` match none, in either direction, for a
        // claim or right_operand of exactly that lexical form -- silently,
        // with no diagnostic. This engine's posture elsewhere is strict
        // rejection of such edge-case lexical forms.
        let inf_claim = claims_with(&[("count", ClaimValue::Single("inf".into()))]);
        assert!(!Constraint::new("count", Operator::Gt, "1000000").evaluate(&inf_claim));
        assert!(!Constraint::new("count", Operator::Gteq, "1000000").evaluate(&inf_claim));
        assert!(!Constraint::new("count", Operator::Lt, "1000000").evaluate(&inf_claim));
        assert!(!Constraint::new("count", Operator::Lteq, "1000000").evaluate(&inf_claim));

        let finite_claim = claims_with(&[("count", ClaimValue::Single("42".into()))]);
        assert!(!Constraint::new("count", Operator::Lteq, "-infinity").evaluate(&finite_claim));
        assert!(!Constraint::new("count", Operator::Gteq, "Infinity").evaluate(&finite_claim));
    }

    #[test]
    fn nested_json_and_deserializes_and_evaluates_conjunctively() {
        let json = r#"{"odrl:and": [
            {"left_operand": "sub", "operator": "eq", "right_operand": "alice"},
            {"left_operand": "scope", "operator": "isAnyOf", "right_operand": "read,write"}
        ]}"#;
        let constraint: Constraint = serde_json::from_str(json).unwrap();
        assert!(constraint.is_logical());
        assert_eq!(constraint.and.as_ref().map(Vec::len), Some(2));

        let matching = claims_with(&[
            ("sub", ClaimValue::Single("alice".into())),
            ("scope", ClaimValue::Single("write".into())),
        ]);
        assert!(constraint.evaluate(&matching), "both children match -> odrl:and is satisfied");

        let only_one_matches = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert!(
            !constraint.evaluate(&only_one_matches),
            "only one of two children matches -> odrl:and is not satisfied"
        );
    }

    #[test]
    fn nested_json_or_deserializes_and_is_satisfied_by_a_single_matching_child() {
        let json = r#"{"odrl:or": [
            {"left_operand": "sub", "operator": "eq", "right_operand": "alice"},
            {"left_operand": "sub", "operator": "eq", "right_operand": "bob"}
        ]}"#;
        let constraint: Constraint = serde_json::from_str(json).unwrap();

        let alice = claims_with(&[("sub", ClaimValue::Single("alice".into()))]);
        assert!(constraint.evaluate(&alice));

        let carol = claims_with(&[("sub", ClaimValue::Single("carol".into()))]);
        assert!(!constraint.evaluate(&carol));
    }

    #[test]
    fn a_two_level_and_of_or_nest_evaluates_correctly_in_both_directions() {
        // (sub eq alice) AND (scope isAnyOf read,write OR scope isAnyOf admin)
        let constraint = Constraint::and(vec![
            Constraint::new("sub", Operator::Eq, "alice"),
            Constraint::or(vec![
                Constraint::new("scope", Operator::IsAnyOf, "read,write"),
                Constraint::new("scope", Operator::Eq, "admin"),
            ]),
        ]);

        let allowed = claims_with(&[
            ("sub", ClaimValue::Single("alice".into())),
            ("scope", ClaimValue::Single("admin".into())),
        ]);
        assert!(constraint.evaluate(&allowed));

        let wrong_subject = claims_with(&[
            ("sub", ClaimValue::Single("bob".into())),
            ("scope", ClaimValue::Single("admin".into())),
        ]);
        assert!(!constraint.evaluate(&wrong_subject));

        let neither_scope_option = claims_with(&[
            ("sub", ClaimValue::Single("alice".into())),
            ("scope", ClaimValue::Single("delete".into())),
        ]);
        assert!(!constraint.evaluate(&neither_scope_option));
    }

    #[test]
    fn a_three_level_mixed_nest_or_containing_and_containing_a_plain_constraint_evaluates_correctly() {
        // outer OR( AND(sub eq alice, scope eq admin), sub eq root )
        let constraint = Constraint::or(vec![
            Constraint::and(vec![
                Constraint::new("sub", Operator::Eq, "alice"),
                Constraint::new("scope", Operator::Eq, "admin"),
            ]),
            Constraint::new("sub", Operator::Eq, "root"),
        ]);

        let root = claims_with(&[("sub", ClaimValue::Single("root".into()))]);
        assert!(constraint.evaluate(&root), "the plain-constraint disjunct alone should satisfy the outer OR");

        let alice_admin = claims_with(&[
            ("sub", ClaimValue::Single("alice".into())),
            ("scope", ClaimValue::Single("admin".into())),
        ]);
        assert!(constraint.evaluate(&alice_admin), "the nested AND disjunct, fully satisfied, should satisfy the outer OR");

        let alice_not_admin = claims_with(&[
            ("sub", ClaimValue::Single("alice".into())),
            ("scope", ClaimValue::Single("guest".into())),
        ]);
        assert!(
            !constraint.evaluate(&alice_not_admin),
            "neither disjunct is fully satisfied: alice is not root, and the nested AND's second \
             conjunct (scope eq admin) misses"
        );
    }

    #[test]
    fn xone_is_satisfied_by_exactly_one_matching_child_not_zero_and_not_two_or_more() {
        // The one case this phase exists to make possible: DNF (odrl:or of
        // pairwise odrl:and combinations) can express "at least one of
        // these", never "exactly one, not more" -- see this crate's own
        // `Constraint::Xone` doc comment and this repo's README.
        let constraint = Constraint::xone(vec![
            Constraint::new("scope", Operator::IsAnyOf, "read"),
            Constraint::new("scope", Operator::IsAnyOf, "write"),
            Constraint::new("scope", Operator::IsAnyOf, "admin"),
        ]);

        let zero_match = claims_with(&[("scope", ClaimValue::Single("delete".into()))]);
        assert!(!constraint.evaluate(&zero_match), "0 matching children must not satisfy xone");

        let exactly_one_match = claims_with(&[("scope", ClaimValue::Single("write".into()))]);
        assert!(constraint.evaluate(&exactly_one_match), "exactly 1 matching child must satisfy xone");

        // A multi-valued claim whose elements satisfy two of the three
        // children simultaneously -- the case DNF-as-OR would wrongly
        // allow through, and xone must not.
        let two_or_more_match = claims_with(&[(
            "scope",
            ClaimValue::Multi(vec!["read".into(), "admin".into()]),
        )]);
        assert!(
            !constraint.evaluate(&two_or_more_match),
            "2 matching children must not satisfy xone -- this is the exact 'exactly one, not \
             more' distinction DNF cannot express"
        );
    }

    #[test]
    fn xone_with_an_empty_children_list_is_never_satisfied() {
        let constraint = Constraint::xone(vec![]);
        let claims = claims_with(&[]);
        assert!(!constraint.evaluate(&claims), "0 of 0 children matching is still 0, not exactly 1");
    }

    #[test]
    fn nested_json_xone_deserializes_into_the_xone_field() {
        let json = r#"{"odrl:xone": [
            {"left_operand": "role", "operator": "eq", "right_operand": "admin"},
            {"left_operand": "role", "operator": "eq", "right_operand": "guest"}
        ]}"#;
        let constraint: Constraint = serde_json::from_str(json).unwrap();
        assert!(constraint.is_logical());
        assert_eq!(constraint.xone.as_ref().map(Vec::len), Some(2));

        let admin = claims_with(&[("role", ClaimValue::Single("admin".into()))]);
        assert!(constraint.evaluate(&admin));

        let neither = claims_with(&[("role", ClaimValue::Single("member".into()))]);
        assert!(!constraint.evaluate(&neither));
    }

    #[test]
    fn nesting_past_max_constraint_depth_is_a_deterministic_non_match_not_a_panic() {
        // Builds a chain of MAX_CONSTRAINT_DEPTH + 10 nested single-child
        // `odrl:and` wrappers around one always-true leaf (an unconstrained
        // `isNoneOf` against an absent claim key). Directly constructed in
        // Rust, not via JSON, precisely to prove the guard is a property of
        // `evaluate` itself and not merely of a parser-side recursion limit
        // (see `MAX_CONSTRAINT_DEPTH`'s own doc comment on why a `Constraint`
        // tree has no cycle to guard against the way `profile.rs`'s
        // `includedIn` walk does, only unbounded depth).
        let leaf = Constraint::new("absent-claim", Operator::IsNoneOf, "anything");
        let mut deeply_nested = leaf;
        for _ in 0..(MAX_CONSTRAINT_DEPTH + 10) {
            deeply_nested = Constraint::and(vec![deeply_nested]);
        }

        let claims = claims_with(&[]);
        // Must not panic (stack overflow) and must resolve deterministically
        // to false, per this bound's documented behavior -- even though the
        // sole leaf, evaluated on its own, would be `true`.
        assert!(
            !deeply_nested.evaluate(&claims),
            "a structure nested past MAX_CONSTRAINT_DEPTH must be treated as a deterministic \
             non-match, not evaluated as if the depth bound did not exist"
        );

        // A structure nested exactly AT the bound (not past it) must still
        // evaluate normally and reach the always-true leaf.
        let mut at_bound = Constraint::new("absent-claim", Operator::IsNoneOf, "anything");
        for _ in 0..MAX_CONSTRAINT_DEPTH {
            at_bound = Constraint::and(vec![at_bound]);
        }
        assert!(
            at_bound.evaluate(&claims),
            "a structure nested exactly at MAX_CONSTRAINT_DEPTH (not past it) must still \
             evaluate normally"
        );
    }
}
