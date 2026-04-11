//! Built-in theme identifiers and slug ↔ preset resolution.

use super::themes::{breeze_nostalgia, commander, cosmos, neos_dream, orange_monochrome, oxide};
use super::UiPalette;

/// Closed set of built-in themes; extend when adding presets in [`super::themes`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ThemeId {
    #[default]
    Oxide,
    Commander,
    OrangeMonochrome,
    BreezeNostalgia,
    NeosDream,
    Cosmos,
}

impl ThemeId {
    /// Presets shown in F9 → Theme (order = cycle order when multiple exist).
    pub const ALL: &'static [ThemeId] = &[
        ThemeId::Oxide,
        ThemeId::Commander,
        ThemeId::OrangeMonochrome,
        ThemeId::BreezeNostalgia,
        ThemeId::NeosDream,
        ThemeId::Cosmos,
    ];

    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            ThemeId::Oxide => "oxide",
            ThemeId::Commander => "commander",
            ThemeId::OrangeMonochrome => "orangemonochrome",
            ThemeId::BreezeNostalgia => "breezenostalgia",
            ThemeId::NeosDream => "neosdream",
            ThemeId::Cosmos => "cosmos",
        }
    }

    /// Value stored in `settings.json` under `theme`.
    #[must_use]
    pub fn from_slug(s: &str) -> Self {
        if s.eq_ignore_ascii_case(ThemeId::Cosmos.slug()) {
            ThemeId::Cosmos
        } else if s.eq_ignore_ascii_case(ThemeId::NeosDream.slug()) {
            ThemeId::NeosDream
        } else if s.eq_ignore_ascii_case(ThemeId::BreezeNostalgia.slug()) {
            ThemeId::BreezeNostalgia
        } else if s.eq_ignore_ascii_case(ThemeId::OrangeMonochrome.slug()) {
            ThemeId::OrangeMonochrome
        } else if s.eq_ignore_ascii_case(ThemeId::Commander.slug()) {
            ThemeId::Commander
        } else if s.eq_ignore_ascii_case(ThemeId::Oxide.slug()) {
            ThemeId::Oxide
        } else {
            ThemeId::Oxide
        }
    }

    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            ThemeId::Oxide => "Oxide (default)",
            ThemeId::Commander => "Commander",
            ThemeId::OrangeMonochrome => "Orange monochrome",
            ThemeId::BreezeNostalgia => "Breeze nostalgia",
            ThemeId::NeosDream => "Neo's dream",
            ThemeId::Cosmos => "Cosmos",
        }
    }

    #[must_use]
    pub fn palette(self) -> UiPalette {
        match self {
            ThemeId::Oxide => oxide::PALETTE,
            ThemeId::Commander => commander::PALETTE,
            ThemeId::OrangeMonochrome => orange_monochrome::PALETTE,
            ThemeId::BreezeNostalgia => breeze_nostalgia::PALETTE,
            ThemeId::NeosDream => neos_dream::PALETTE,
            ThemeId::Cosmos => cosmos::PALETTE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ThemeId;

    #[test]
    fn from_slug_commander() {
        assert_eq!(
            ThemeId::from_slug("commander"),
            ThemeId::Commander
        );
        assert_eq!(
            ThemeId::from_slug("COMMANDER"),
            ThemeId::Commander
        );
    }

    #[test]
    fn from_slug_unknown_falls_back_to_oxide() {
        assert_eq!(
            ThemeId::from_slug("no-such-theme"),
            ThemeId::Oxide
        );
    }

    #[test]
    fn from_slug_orange_monochrome() {
        assert_eq!(
            ThemeId::from_slug("orangemonochrome"),
            ThemeId::OrangeMonochrome
        );
        assert_eq!(
            ThemeId::from_slug("ORANGEMONOCHROME"),
            ThemeId::OrangeMonochrome
        );
    }

    #[test]
    fn from_slug_breeze_neos_cosmos() {
        assert_eq!(
            ThemeId::from_slug("breezenostalgia"),
            ThemeId::BreezeNostalgia
        );
        assert_eq!(
            ThemeId::from_slug("neosdream"),
            ThemeId::NeosDream
        );
        assert_eq!(
            ThemeId::from_slug("COSMOS"),
            ThemeId::Cosmos
        );
    }
}
