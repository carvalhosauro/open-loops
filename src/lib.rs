//! open-loops: recupera contexto de trabalhos pausados.
//! Spec: docs/superpowers/specs/2026-06-10-open-loops-mvp-design.md

pub mod cache;
pub mod cli;
pub mod config;
pub mod distill;
pub mod ignores;
pub mod output;
pub mod scanner;
pub mod sessions;
#[cfg(test)]
pub mod testutil;
