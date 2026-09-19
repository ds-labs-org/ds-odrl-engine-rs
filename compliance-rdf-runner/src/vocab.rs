#![allow(non_snake_case)] // mirrors RDF term casing (TestCase, ClaimKey, ...) verbatim, deliberately.

//! IRI constants for the terms this translator actually reads, built off
//! `graph::{odrl, dsc, report_ns}`'s namespace helpers. Centralized
//! here rather than inlined at each call site so a typo'd predicate name
//! fails to compile in exactly one place instead of silently mismatching
//! at a dozen call sites.

use crate::graph::{dsc, odrl, report_ns};

macro_rules! iri_consts {
    ($ns:ident, $($const_name:ident => $local:literal),+ $(,)?) => {
        $(pub fn $const_name() -> String { $ns($local) })+
    };
}

// -- odrl: ------------------------------------------------------------
iri_consts!(odrl,
    odrl_target => "target",
    odrl_action => "action",
    odrl_permission => "permission",
    odrl_prohibition => "prohibition",
    odrl_obligation => "obligation",
    odrl_assigner => "assigner",
    odrl_assignee => "assignee",
    odrl_conflict => "conflict",
    odrl_constraint => "constraint",
    odrl_refinement => "refinement",
    odrl_duty => "duty",
    odrl_remedy => "remedy",
    odrl_consequence => "consequence",
    odrl_leftOperand => "leftOperand",
    odrl_operator => "operator",
    odrl_rightOperand => "rightOperand",
    odrl_and => "and",
    odrl_or => "or",
    odrl_xone => "xone",
    odrl_andSequence => "andSequence",
    odrl_includedIn => "includedIn",
    odrl_partOf => "partOf",
    odrl_Profile => "Profile",
);

pub const RDF_VALUE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#value";

// -- dsc: ---------------------------------------------------------------
iri_consts!(dsc,
    dsc_TestCase => "TestCase",
    dsc_request => "request",
    dsc_expectedOutcome => "expectedOutcome",
    dsc_profile => "profile",
    dsc_policy => "policy",
    dsc_claim => "claim",
    dsc_inheritsFrom => "inheritsFrom",
    dsc_dutyMode => "dutyMode",
    dsc_behaviour => "behaviour",
    dsc_partyIdentityClaim => "partyIdentityClaim",
    dsc_agreementAssigneeClaim => "agreementAssigneeClaim",
    dsc_key => "key",
    dsc_policyKind => "policyKind",
    dsc_ClaimKey => "ClaimKey",
    dsc_expectedDecision => "expectedDecision",
);

// -- report: --------------------------------------------------------------
iri_consts!(report_ns,
    report_policy => "policy",
    report_ruleReport => "ruleReport",
    report_rule => "rule",
    report_activationState => "activationState",
    report_attemptState => "attemptState",
    report_performanceState => "performanceState",
    report_deonticState => "deonticState",
    report_PermissionReport => "PermissionReport",
    report_ProhibitionReport => "ProhibitionReport",
    report_DutyReport => "DutyReport",
);
