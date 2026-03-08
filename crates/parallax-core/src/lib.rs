//! # parallax-core
//!
//! Foundation types for the Parallax graph engine. This crate has no
//! knowledge of storage, networking, or query execution. It defines
//! the shared vocabulary: entities, relationships, properties, timestamps,
//! and source tracking.
//!
//! **Spec reference:** `specs/01-data-model.md`

pub mod entity;
pub mod error;
pub mod property;
pub mod relationship;
pub mod source;
pub mod timestamp;
