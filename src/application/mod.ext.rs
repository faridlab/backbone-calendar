// This directory's extension (hand-written; ADR-0031). The generated mod.rs (lib.rs for
// src/) pulls it in with include!, so its lines resolve against this directory.

// The permission-vocabulary layer (application/auth/*): hand-written files
// in the generated shape, user-owned per metaphor.codegen.yaml. The
// declaration lives in the custom block so regeneration keeps it.
pub mod auth;
