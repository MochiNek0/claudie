//! Shared visual tokens for the settings panel and overlay popups.
//!
//! The look is a "modern frosted light" approximation: cool-tinted neutrals,
//! restrained borders, a clean accent, and tight typography. Real backdrop
//! blur is not available in this GDI surface, so depth is conveyed through
//! layered tints, hairline borders, and soft shadow strips rather than alpha
//! blur.

pub(crate) const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

// Surfaces ---------------------------------------------------------------

/// Primary card / sheet background.
pub(crate) const SURFACE: u32 = rgb(255, 255, 255);
/// Header strip, subtly elevated surface.
pub(crate) const SURFACE_ALT: u32 = rgb(249, 251, 254);
/// Input field background.
pub(crate) const FIELD: u32 = rgb(242, 245, 250);

// Lines & elevation -------------------------------------------------------

/// Divider between header and content; very faint.
pub(crate) const HAIRLINE: u32 = rgb(228, 232, 240);
/// Input border.
pub(crate) const FIELD_BORDER: u32 = rgb(218, 224, 234);

// Text --------------------------------------------------------------------

/// Primary text.
pub(crate) const INK: u32 = rgb(17, 24, 39);
/// Helper / hint text.
pub(crate) const MUTED: u32 = rgb(107, 114, 128);
/// Even softer muted (timestamps, micro-labels).
pub(crate) const MUTED_SOFT: u32 = rgb(156, 163, 175);

// Accent (Apple iOS-style blue) ------------------------------------------

pub(crate) const ACCENT: u32 = rgb(10, 132, 255);
pub(crate) const ACCENT_SOFT: u32 = rgb(229, 240, 255);

// Danger ------------------------------------------------------------------

pub(crate) const DANGER: u32 = rgb(220, 53, 69);
pub(crate) const DANGER_SOFT: u32 = rgb(254, 226, 232);

// Radii -------------------------------------------------------------------

pub(crate) const RADIUS_CARD: i32 = 14;
pub(crate) const RADIUS_FIELD: i32 = 10;
pub(crate) const RADIUS_BUTTON: i32 = 9;
pub(crate) const RADIUS_CHIP: i32 = 8;
