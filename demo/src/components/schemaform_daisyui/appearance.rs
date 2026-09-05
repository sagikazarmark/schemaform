//! The appearance axis every renderer in this package carries.

/// Whether the package emits the Tailwind utilities it lays itself out with.
///
/// daisyUI component classes (`fieldset`, `card`, `btn`, `alert`, `join`, ...) are always
/// emitted: a caller's utilities override them cleanly, because daisyUI sub-layers its
/// declarations. The utilities the package emits for its own layout — gaps, borders, widths, the
/// semantic text colours — are generated exactly as a caller's are, so a collision is a
/// source-order tie the caller cannot reliably win. `None` emits none of them, leaving the daisyUI
/// component classes, the `sr-only` utilities that keep hidden labels accessible, and every
/// layout decision to the caller.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Appearance {
    /// The package's own layout utilities, so a form looks finished on install.
    #[default]
    Default,
    /// No layout utilities: daisyUI component classes only.
    None,
}

impl Appearance {
    /// Every value of the axis.
    pub const ALL: [Self; 2] = [Self::Default, Self::None];

    /// `utilities` under [`Appearance::Default`], nothing under [`Appearance::None`].
    pub const fn utilities(self, utilities: &'static str) -> &'static str {
        match self {
            Self::Default => utilities,
            Self::None => "",
        }
    }
}
