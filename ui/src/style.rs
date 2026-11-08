//! Variant and size styles as data: the role `cva` plays in shadcn/ui.
//!
//! Each component's look is a pure function of its variant, its size and the
//! active [`Tokens`], so the whole table is unit-tested and a component only
//! applies the result. Sizes are in rems (GPUI's `h_9` is `rems(2.25)`, as
//! Tailwind's `h-9` is 2.25rem), so they scale with the window's text size.

use gpui::{Hsla, Rems, rems};

use crate::theme::Tokens;

/// The focus ring's opacity, shadcn's `ring-ring/50`.
pub const RING_OPACITY: f32 = 0.5;

/// The ring's opacity on an invalid input, shadcn's `ring-destructive/20`.
pub const INVALID_RING_OPACITY: f32 = 0.2;

/// The focus ring's colour: the ring token at [`RING_OPACITY`].
pub fn focus_ring(t: &Tokens) -> Hsla {
    t.ring.opacity(RING_OPACITY)
}

/// shadcn's button variants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ButtonVariant {
    #[default]
    Default,
    Destructive,
    Outline,
    Secondary,
    Ghost,
    Link,
}

impl ButtonVariant {
    pub const ALL: [ButtonVariant; 6] = [
        ButtonVariant::Default,
        ButtonVariant::Destructive,
        ButtonVariant::Outline,
        ButtonVariant::Secondary,
        ButtonVariant::Ghost,
        ButtonVariant::Link,
    ];
}

/// shadcn's button sizes. `Icon` is a square for an icon-only button.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ButtonSize {
    #[default]
    Default,
    Sm,
    Lg,
    Icon,
}

impl ButtonSize {
    pub const ALL: [ButtonSize; 4] = [
        ButtonSize::Default,
        ButtonSize::Sm,
        ButtonSize::Lg,
        ButtonSize::Icon,
    ];
}

/// A button's resolved look. `None` colours draw nothing (a transparent
/// background, no border).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ButtonStyle {
    pub background: Option<Hsla>,
    pub foreground: Hsla,
    pub border: Option<Hsla>,
    pub hover_background: Option<Hsla>,
    pub hover_foreground: Hsla,
    /// Underline the label on hover (the link variant).
    pub hover_underline: bool,
    pub height: Rems,
    /// A fixed width, for the square icon size.
    pub width: Option<Rems>,
    pub padding_x: Rems,
    /// The horizontal padding when the label has a leading icon
    /// (shadcn's `has-[>svg]:px-*`).
    pub padding_x_with_icon: Rems,
    /// Space between a leading icon and the label.
    pub gap: Rems,
    pub text_size: Rems,
    pub radius: Rems,
}

/// shadcn's `buttonVariants` for `variant` and `size`, in the colours of `t`.
///
/// shadcn's `dark:` refinements (the outline's input-coloured fill, the dimmed
/// destructive) are left out: a token set doesn't say whether it is dark.
pub fn button_style(variant: ButtonVariant, size: ButtonSize, t: &Tokens) -> ButtonStyle {
    // (background, foreground, border, hover background, hover foreground)
    let (background, foreground, border, hover_background, hover_foreground) = match variant {
        // bg-primary text-primary-foreground hover:bg-primary/90
        ButtonVariant::Default => (
            Some(t.primary),
            t.primary_foreground,
            None,
            Some(t.primary.opacity(0.9)),
            t.primary_foreground,
        ),
        // bg-destructive text-white hover:bg-destructive/90
        ButtonVariant::Destructive => (
            Some(t.destructive),
            t.destructive_foreground,
            None,
            Some(t.destructive.opacity(0.9)),
            t.destructive_foreground,
        ),
        // border bg-background hover:bg-accent hover:text-accent-foreground
        ButtonVariant::Outline => (
            Some(t.background),
            t.foreground,
            Some(t.border),
            Some(t.accent),
            t.accent_foreground,
        ),
        // bg-secondary text-secondary-foreground hover:bg-secondary/80
        ButtonVariant::Secondary => (
            Some(t.secondary),
            t.secondary_foreground,
            None,
            Some(t.secondary.opacity(0.8)),
            t.secondary_foreground,
        ),
        // hover:bg-accent hover:text-accent-foreground
        ButtonVariant::Ghost => (
            None,
            t.foreground,
            None,
            Some(t.accent),
            t.accent_foreground,
        ),
        // text-primary underline-offset-4 hover:underline
        ButtonVariant::Link => (None, t.primary, None, None, t.primary),
    };

    // (height, width, px, px with an icon, gap)
    let (height, width, padding_x, padding_x_with_icon, gap) = match size {
        // h-9 px-4 py-2 has-[>svg]:px-3
        ButtonSize::Default => (rems(2.25), None, rems(1.), rems(0.75), rems(0.5)),
        // h-8 gap-1.5 px-3 has-[>svg]:px-2.5
        ButtonSize::Sm => (rems(2.), None, rems(0.75), rems(0.625), rems(0.375)),
        // h-10 px-6 has-[>svg]:px-4
        ButtonSize::Lg => (rems(2.5), None, rems(1.5), rems(1.), rems(0.5)),
        // size-9
        ButtonSize::Icon => (rems(2.25), Some(rems(2.25)), rems(0.), rems(0.), rems(0.5)),
    };

    ButtonStyle {
        background,
        foreground,
        border,
        hover_background,
        hover_foreground,
        hover_underline: variant == ButtonVariant::Link,
        height,
        width,
        padding_x,
        padding_x_with_icon,
        gap,
        // text-sm rounded-md
        text_size: rems(0.875),
        radius: t.radius_md(),
    }
}

/// An input frame's resolved colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputStyle {
    /// The frame's border: the input colour, the ring colour while the ring
    /// shows, and the destructive colour when invalid, which wins.
    pub border: Hsla,
    /// The ring while it shows: the ring colour at half opacity, or the
    /// destructive colour at a fifth when invalid.
    pub ring: Option<Hsla>,
}

/// shadcn's Input frame: `border-input`, `focus-visible:border-ring
/// focus-visible:ring-ring/50`, and `aria-invalid:border-destructive
/// aria-invalid:ring-destructive/20`. `ring` is whether the ring shows,
/// which for a text input is whenever it is focused.
pub fn input_style(ring: bool, invalid: bool, t: &Tokens) -> InputStyle {
    let border = if invalid {
        t.destructive
    } else if ring {
        t.ring
    } else {
        t.input
    };
    let ring = ring.then(|| if invalid { t.destructive.opacity(INVALID_RING_OPACITY) } else { focus_ring(t) });
    InputStyle { border, ring }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Pick = fn(&Tokens) -> Hsla;

    /// What each variant's colours must be, named by token.
    struct VariantRow {
        variant: ButtonVariant,
        background: Option<Pick>,
        foreground: Pick,
        border: Option<Pick>,
        /// The token and the opacity it is drawn at on hover.
        hover_background: Option<(Pick, f32)>,
        hover_foreground: Pick,
        hover_underline: bool,
    }

    fn variant_rows() -> [VariantRow; 6] {
        [
            VariantRow {
                variant: ButtonVariant::Default,
                background: Some(|t| t.primary),
                foreground: |t| t.primary_foreground,
                border: None,
                hover_background: Some((|t| t.primary, 0.9)),
                hover_foreground: |t| t.primary_foreground,
                hover_underline: false,
            },
            VariantRow {
                variant: ButtonVariant::Destructive,
                background: Some(|t| t.destructive),
                foreground: |t| t.destructive_foreground,
                border: None,
                hover_background: Some((|t| t.destructive, 0.9)),
                hover_foreground: |t| t.destructive_foreground,
                hover_underline: false,
            },
            VariantRow {
                variant: ButtonVariant::Outline,
                background: Some(|t| t.background),
                foreground: |t| t.foreground,
                border: Some(|t| t.border),
                hover_background: Some((|t| t.accent, 1.0)),
                hover_foreground: |t| t.accent_foreground,
                hover_underline: false,
            },
            VariantRow {
                variant: ButtonVariant::Secondary,
                background: Some(|t| t.secondary),
                foreground: |t| t.secondary_foreground,
                border: None,
                hover_background: Some((|t| t.secondary, 0.8)),
                hover_foreground: |t| t.secondary_foreground,
                hover_underline: false,
            },
            VariantRow {
                variant: ButtonVariant::Ghost,
                background: None,
                foreground: |t| t.foreground,
                border: None,
                hover_background: Some((|t| t.accent, 1.0)),
                hover_foreground: |t| t.accent_foreground,
                hover_underline: false,
            },
            VariantRow {
                variant: ButtonVariant::Link,
                background: None,
                foreground: |t| t.primary,
                border: None,
                hover_background: None,
                hover_foreground: |t| t.primary,
                hover_underline: true,
            },
        ]
    }

    /// (size, height, width, px, px with an icon, gap), in rems.
    const SIZE_ROWS: [(ButtonSize, f32, Option<f32>, f32, f32, f32); 4] = [
        (ButtonSize::Default, 2.25, None, 1.0, 0.75, 0.5),
        (ButtonSize::Sm, 2.0, None, 0.75, 0.625, 0.375),
        (ButtonSize::Lg, 2.5, None, 1.5, 1.0, 0.5),
        (ButtonSize::Icon, 2.25, Some(2.25), 0.0, 0.0, 0.5),
    ];

    #[test]
    fn every_variant_and_size_against_both_sets() {
        let sets = [
            ("light", Tokens::neutral_light()),
            ("dark", Tokens::neutral_dark()),
        ];
        let mut combinations = 0;
        for (set, t) in &sets {
            for row in variant_rows() {
                for (size, height, width, px, px_icon, gap) in SIZE_ROWS {
                    let case = format!("{set} {:?} {size:?}", row.variant);
                    let style = button_style(row.variant, size, t);
                    let expected = ButtonStyle {
                        background: row.background.map(|pick| pick(t)),
                        foreground: (row.foreground)(t),
                        border: row.border.map(|pick| pick(t)),
                        hover_background: row
                            .hover_background
                            .map(|(pick, opacity)| pick(t).opacity(opacity)),
                        hover_foreground: (row.hover_foreground)(t),
                        hover_underline: row.hover_underline,
                        height: rems(height),
                        width: width.map(rems),
                        padding_x: rems(px),
                        padding_x_with_icon: rems(px_icon),
                        gap: rems(gap),
                        text_size: rems(0.875),
                        radius: t.radius_md(),
                    };
                    assert_eq!(style, expected, "{case}");
                    combinations += 1;
                }
            }
        }
        // 6 variants x 4 sizes, in each of the two sets.
        assert_eq!(combinations, 2 * 24);
        assert_eq!(ButtonVariant::ALL.len() * ButtonSize::ALL.len(), 24);
    }

    #[test]
    fn hover_fades_where_shadcn_uses_opacity() {
        for t in [Tokens::neutral_light(), Tokens::neutral_dark()] {
            let hover = |v| button_style(v, ButtonSize::Default, &t).hover_background.unwrap();
            // bg-primary/90 and bg-destructive/90
            let primary = hover(ButtonVariant::Default);
            assert_eq!((primary.h, primary.s, primary.l), (t.primary.h, t.primary.s, t.primary.l));
            assert!((primary.a - 0.9).abs() < 1e-6);
            assert!((hover(ButtonVariant::Destructive).a - 0.9).abs() < 1e-6);
            // bg-secondary/80
            assert!((hover(ButtonVariant::Secondary).a - 0.8).abs() < 1e-6);
            // hover:bg-accent, opaque
            assert_eq!(hover(ButtonVariant::Ghost), t.accent);
        }
    }

    /// The input's border: the input colour, the ring colour while the ring
    /// shows, and the destructive colour when invalid, ring or not.
    #[test]
    fn input_border_is_input_ring_or_destructive() {
        for t in [Tokens::neutral_light(), Tokens::neutral_dark()] {
            // (ring shown, invalid, expected border, expected ring)
            let cases: [(bool, bool, Hsla, Option<Hsla>); 4] = [
                (false, false, t.input, None),
                (true, false, t.ring, Some(t.ring.opacity(0.5))),
                (false, true, t.destructive, None),
                (true, true, t.destructive, Some(t.destructive.opacity(0.2))),
            ];
            for (ring, invalid, border, ring_color) in cases {
                let style = input_style(ring, invalid, &t);
                assert_eq!(style, InputStyle { border, ring: ring_color }, "ring={ring} invalid={invalid}");
            }
            assert_eq!(focus_ring(&t), t.ring.opacity(RING_OPACITY));
        }
    }

    #[test]
    fn tables_cover_every_variant_and_size() {
        let rows: Vec<_> = variant_rows().iter().map(|r| r.variant).collect();
        assert_eq!(rows, ButtonVariant::ALL);
        let sizes: Vec<_> = SIZE_ROWS.iter().map(|r| r.0).collect();
        assert_eq!(sizes, ButtonSize::ALL);
    }
}
