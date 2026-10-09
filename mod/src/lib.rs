//! Elden Resonant: an Elden Ring x CONTROL Resonant mashup.
//!
//! Loaded into Elden Ring by ModEngine2. It turns the Wretch into a Resonant
//! who starts with the Aberrant's six forms (each with its combo finisher)
//! and Dylan's Resonance powers, and plays CONTROL Resonant's own sounds,
//! read from the player's install, when they are used.

pub mod config;
pub mod cotr;
pub mod cues;
pub mod fmg;
#[allow(clippy::all)]
pub mod generated;
pub mod locate;

#[cfg(windows)]
mod win;

#[cfg(windows)]
pub use win::DllMain;
