#![cfg_attr(not(test), no_std)]
//! Integer-only, alloc-free hyperdimensional-computing core for edge targets.

pub mod hv;
pub mod store;
pub mod encode;

pub use hv::{Hv, DIM_BITS, WORDS};
pub use encode::FeatureEncoder;
