//! Parser and editor for The Alters (11 bit studios) save files.
//!
//! No public documentation exists for this format; everything here was
//! reverse-engineered from real saves (v-6, game versions mid-2026). The
//! format notes live in each module's docs.
//!
//! # Layers
//!
//! - [`sav`]: outer file framing - plaintext prefix, zlib chunk stream, and
//!   the two EOF-relative size fields that must stay consistent.
//! - [`resources`]: locate and edit the base's stored resource amounts
//!   inside the decompressed body.
//!
//! # Example
//!
//! ```no_run
//! use alters_save_core::sav::SaveFile;
//! use alters_save_core::resources;
//!
//! let bytes = std::fs::read("save.sav")?;
//! let mut save = SaveFile::parse(&bytes)?;
//! for container in resources::containers(&save.body) {
//!     println!("{}: {}", container.resource.0, container.amount);
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod alters;
mod elb;
pub mod error;
pub mod items;
pub mod meta;
pub mod quests;
pub mod research;
pub mod resources;
pub mod sav;
pub mod time;
pub mod verify;

pub use error::{Error, Result};
