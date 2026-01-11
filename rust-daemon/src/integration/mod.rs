//! Integration module for connecting with external systems like terminit.
//!
//! This module provides bridges and adapters for interoperability
//! with other AI agent monitoring and terminal management tools.

pub mod shared_types;
pub mod terminit;

// Re-export for external use
#[allow(unused_imports)]
pub use shared_types::*;
#[allow(unused_imports)]
pub use terminit::TerminitBridge;
