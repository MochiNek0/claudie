//! Shared visual tokens for the GDI-drawn pet HUD windows.
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

// Lines & elevation -------------------------------------------------------

/// Divider between header and content; very faint.
pub(crate) const HAIRLINE: u32 = rgb(228, 232, 240);

// Text --------------------------------------------------------------------

/// Primary text.
pub(crate) const INK: u32 = rgb(17, 24, 39);

// Radii -------------------------------------------------------------------

pub(crate) const RADIUS_FIELD: i32 = 10;
pub(crate) const RADIUS_CHIP: i32 = 8;
