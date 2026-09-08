//! Static chunk-terrain frontend boundary.
//!
//! Terrain v1 reuses the indexed world-mesh ABI and VulkanicGAL path. This module names the
//! section lifecycle and compatibility concepts so future terrain work has a focused home instead
//! of expanding the primitive frontend.

pub mod diagnostics;
pub mod materials;
pub mod mesh_cache;
pub mod placement;
pub mod resources;
pub mod section;
pub mod submission;
pub mod visibility;

pub use section::{LayerKind, SectionLifecycle};
