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
/// destructive) are left out: the variants read tokens only, not the set's
/// [`Tokens::dark`] hint.
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

/// A checkbox's side, shadcn's `size-4`.
pub const CHECKBOX_SIZE: Rems = Rems(1.);

/// A checkbox's corner radius, shadcn's `rounded-[4px]`.
pub const CHECKBOX_RADIUS: Rems = Rems(0.25);

/// The tick's size, shadcn's `size-3.5`.
pub const CHECKBOX_TICK_SIZE: Rems = Rems(0.875);

/// A checkbox's resolved colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CheckboxStyle {
    /// The fill: the primary colour when checked, none when not.
    pub background: Option<Hsla>,
    /// The border: the primary colour when checked, the input colour when not.
    pub border: Hsla,
    /// The tick.
    pub tick: Hsla,
}

/// shadcn's Checkbox: `border-input`, and when checked `bg-primary
/// border-primary text-primary-foreground`.
pub fn checkbox_style(checked: bool, t: &Tokens) -> CheckboxStyle {
    CheckboxStyle {
        background: checked.then_some(t.primary),
        border: if checked { t.primary } else { t.input },
        tick: t.primary_foreground,
    }
}

/// A switch track's width, shadcn's `w-8`.
pub const SWITCH_TRACK_WIDTH: Rems = Rems(2.);

/// A switch track's height, shadcn's `h-[1.15rem]`.
pub const SWITCH_TRACK_HEIGHT: Rems = Rems(1.15);

/// A switch track's border, shadcn's 1px `border`, in rems so it scales.
pub const SWITCH_TRACK_BORDER: Rems = Rems(0.0625);

/// A switch thumb's diameter, shadcn's `size-4`.
pub const SWITCH_THUMB_SIZE: Rems = Rems(1.);

/// A switch's resolved look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwitchStyle {
    /// The track: the primary colour when on, the input colour when off.
    pub track: Hsla,
    pub thumb: Hsla,
    /// How far the thumb sits from the track's inner left edge.
    pub thumb_offset: Rems,
}

/// shadcn's Switch: the track `data-[state=checked]:bg-primary
/// data-[state=unchecked]:bg-input`, the thumb `bg-background`, moved to
/// the far end when on. In a dark set ([`Tokens::dark`]) the thumb takes
/// shadcn's `dark:data-[state=unchecked]:bg-foreground
/// dark:data-[state=checked]:bg-primary-foreground`, since a near-black
/// thumb would all but vanish on the translucent unchecked track. That is
/// the only `dark:` refinement modelled; the track's `dark:bg-input/80` is
/// left out.
pub fn switch_style(checked: bool, t: &Tokens) -> SwitchStyle {
    let thumb = match (t.dark, checked) {
        (false, _) => t.background,
        (true, false) => t.foreground,
        (true, true) => t.primary_foreground,
    };
    SwitchStyle {
        track: if checked { t.primary } else { t.input },
        thumb,
        thumb_offset: switch_thumb_offset(checked),
    }
}

/// Where the thumb sits inside the track's border: against the left end
/// when off, against the right end when on (shadcn's
/// `translate-x-[calc(100%-2px)]`).
pub fn switch_thumb_offset(checked: bool) -> Rems {
    if checked { Rems(SWITCH_TRACK_WIDTH.0 - 2. * SWITCH_TRACK_BORDER.0 - SWITCH_THUMB_SIZE.0) } else { Rems(0.) }
}

/// A dialog's widest, shadcn's `sm:max-w-lg`.
pub const DIALOG_MAX_WIDTH: Rems = Rems(32.);

/// The least room between a dialog and each side of the window: shadcn's
/// `max-w-[calc(100%-2rem)]`, halved.
pub const DIALOG_EDGE_GAP: Rems = Rems(1.);

/// The close button's opacity at rest, shadcn's `opacity-70`. It is opaque
/// on hover.
pub const DIALOG_CLOSE_OPACITY: f32 = 0.7;

/// The close button's corner radius, shadcn's `rounded-xs` (2px).
pub const DIALOG_CLOSE_RADIUS: Rems = Rems(0.125);

/// A dialog panel's resolved colours and radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DialogStyle {
    pub background: Hsla,
    pub foreground: Hsla,
    pub border: Hsla,
    pub radius: Rems,
    /// The description's text.
    pub description: Hsla,
}

/// shadcn's DialogContent (`bg-background rounded-lg border`), in the
/// foreground colour, with DialogDescription's `text-muted-foreground`.
pub fn dialog_style(t: &Tokens) -> DialogStyle {
    DialogStyle {
        background: t.background,
        foreground: t.foreground,
        border: t.border,
        radius: t.radius_lg(),
        description: t.muted_foreground,
    }
}

/// A popover panel's width, shadcn's `w-72`.
pub const POPOVER_WIDTH: Rems = Rems(18.);

/// A popover panel's resolved colours and radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PopoverStyle {
    pub background: Hsla,
    pub foreground: Hsla,
    pub border: Hsla,
    pub radius: Rems,
}

/// shadcn's PopoverContent: `bg-popover text-popover-foreground rounded-md
/// border`.
pub fn popover_style(t: &Tokens) -> PopoverStyle {
    PopoverStyle {
        background: t.popover,
        foreground: t.popover_foreground,
        border: t.border,
        radius: t.radius_md(),
    }
}

/// The opacity of a decorative icon beside text: a select trigger's chevron
/// and a command filter's search glass, shadcn's `opacity-50`.
pub const MUTED_ICON_OPACITY: f32 = 0.5;

/// A select menu's least width, shadcn's `min-w-[8rem]`. The menu is also
/// at least as wide as its trigger.
pub const SELECT_MENU_MIN_WIDTH: Rems = Rems(8.);

/// A select menu's greatest height. shadcn's is whatever room the
/// positioner finds; the crate caps it at `max-h-96` and lets `Anchored`
/// flip or pin the menu.
pub const SELECT_MENU_MAX_HEIGHT: Rems = Rems(24.);

/// The tick beside the selected option, shadcn's `size-3.5`.
pub const SELECT_TICK_SIZE: Rems = Rems(0.875);

/// A select's resolved look: the trigger and the menu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectStyle {
    /// The trigger's border, the input colour.
    pub border: Hsla,
    /// The trigger's text when it shows the selected option's label.
    pub foreground: Hsla,
    /// The trigger's text when it shows the placeholder.
    pub placeholder: Hsla,
    /// The menu panel.
    pub menu_background: Hsla,
    pub menu_foreground: Hsla,
    pub menu_border: Hsla,
    /// The highlighted option.
    pub highlight: Hsla,
    pub highlight_foreground: Hsla,
    /// The trigger's and the menu's corners.
    pub radius: Rems,
    /// An option's corners.
    pub option_radius: Rems,
}

/// shadcn's Select: the trigger `border-input`, its placeholder
/// `text-muted-foreground`, the content `bg-popover text-popover-foreground
/// rounded-md border`, and the item `focus:bg-accent
/// focus:text-accent-foreground rounded-sm`.
pub fn select_style(t: &Tokens) -> SelectStyle {
    SelectStyle {
        border: t.input,
        foreground: t.foreground,
        placeholder: t.muted_foreground,
        menu_background: t.popover,
        menu_foreground: t.popover_foreground,
        menu_border: t.border,
        highlight: t.accent,
        highlight_foreground: t.accent_foreground,
        radius: t.radius_md(),
        option_radius: t.radius_sm(),
    }
}

/// A command list's greatest height, shadcn's `max-h-[300px]`, in rems so
/// it scales.
pub const COMMAND_LIST_MAX_HEIGHT: Rems = Rems(18.75);

/// A command list's resolved look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CommandStyle {
    pub background: Hsla,
    pub foreground: Hsla,
    /// The line under the filter input.
    pub border: Hsla,
    /// Group headings, item details and shortcuts.
    pub muted: Hsla,
    /// The highlighted item.
    pub highlight: Hsla,
    pub highlight_foreground: Hsla,
    /// The list's corners.
    pub radius: Rems,
    /// An item's corners.
    pub item_radius: Rems,
}

/// shadcn's Command: `bg-popover text-popover-foreground rounded-md`, the
/// input row's `border-b`, headings and shortcuts in `text-muted-foreground`,
/// and the item `data-[selected=true]:bg-accent
/// data-[selected=true]:text-accent-foreground rounded-sm`.
pub fn command_style(t: &Tokens) -> CommandStyle {
    CommandStyle {
        background: t.popover,
        foreground: t.popover_foreground,
        border: t.border,
        muted: t.muted_foreground,
        highlight: t.accent,
        highlight_foreground: t.accent_foreground,
        radius: t.radius_md(),
        item_radius: t.radius_sm(),
    }
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

    /// A checkbox is a primary square with a primary-foreground tick when
    /// checked, and an input-coloured outline when not.
    #[test]
    fn checkbox_fills_with_primary_when_checked() {
        for t in [Tokens::neutral_light(), Tokens::neutral_dark()] {
            assert_eq!(
                checkbox_style(true, &t),
                CheckboxStyle { background: Some(t.primary), border: t.primary, tick: t.primary_foreground }
            );
            assert_eq!(
                checkbox_style(false, &t),
                CheckboxStyle { background: None, border: t.input, tick: t.primary_foreground }
            );
        }
        // size-4, rounded-[4px], a size-3.5 tick.
        assert_eq!((CHECKBOX_SIZE, CHECKBOX_RADIUS, CHECKBOX_TICK_SIZE), (rems(1.), rems(0.25), rems(0.875)));
    }

    /// A switch's track is primary when on and input-coloured when off. The
    /// thumb is the background in a light set; in a dark one it is the
    /// foreground when off and the primary foreground when on, shadcn's
    /// `dark:` thumb colours.
    #[test]
    fn switch_colours_for_each_state_and_set() {
        // (set, checked, expected track, expected thumb)
        let cases: [(&str, Tokens, bool, Pick, Pick); 4] = [
            ("light", Tokens::neutral_light(), false, |t| t.input, |t| t.background),
            ("light", Tokens::neutral_light(), true, |t| t.primary, |t| t.background),
            ("dark", Tokens::neutral_dark(), false, |t| t.input, |t| t.foreground),
            ("dark", Tokens::neutral_dark(), true, |t| t.primary, |t| t.primary_foreground),
        ];
        for (set, t, checked, track, thumb) in cases {
            let style = switch_style(checked, &t);
            assert_eq!((style.track, style.thumb), (track(&t), thumb(&t)), "{set} checked={checked}");
        }
        // The hint alone picks the thumb: the dark colours with the flag
        // cleared are the light rule's.
        let dark_unflagged = Tokens { dark: false, ..Tokens::neutral_dark() };
        assert_eq!(switch_style(false, &dark_unflagged).thumb, dark_unflagged.background);
        assert_eq!(switch_style(true, &dark_unflagged).thumb, dark_unflagged.background);
        // The unchecked dark thumb stands out from the track behind it.
        let dark = Tokens::neutral_dark();
        assert!(switch_style(false, &dark).thumb.l > 0.9);
    }

    /// The thumb moves from one end of the track to the other.
    #[test]
    fn switch_thumb_sits_at_the_end_for_its_state() {
        for t in [Tokens::neutral_light(), Tokens::neutral_dark()] {
            let on = switch_style(true, &t);
            let off = switch_style(false, &t);
            assert_eq!(off.thumb_offset, rems(0.));
            // shadcn: translate-x-[calc(100%-2px)], the thumb's width less the borders.
            assert_eq!(on.thumb_offset, rems(0.875));
        }
        // On, the thumb ends against the track's inner right edge.
        let inner_right = SWITCH_TRACK_WIDTH.0 - SWITCH_TRACK_BORDER.0;
        assert_eq!(switch_thumb_offset(true).0 + SWITCH_THUMB_SIZE.0 + SWITCH_TRACK_BORDER.0, inner_right);
        // And it fits the track's height.
        assert!(SWITCH_THUMB_SIZE.0 <= SWITCH_TRACK_HEIGHT.0 - 2. * SWITCH_TRACK_BORDER.0 + f32::EPSILON);
    }

    /// A dialog is the background colour with a border, rounded-lg, and
    /// its description is muted.
    #[test]
    fn dialog_colours_come_from_the_tokens() {
        for t in [Tokens::neutral_light(), Tokens::neutral_dark()] {
            assert_eq!(
                dialog_style(&t),
                DialogStyle {
                    background: t.background,
                    foreground: t.foreground,
                    border: t.border,
                    radius: t.radius_lg(),
                    description: t.muted_foreground,
                }
            );
        }
        // sm:max-w-lg, 1rem from each side, opacity-70, rounded-xs.
        assert_eq!((DIALOG_MAX_WIDTH, DIALOG_EDGE_GAP, DIALOG_CLOSE_RADIUS), (rems(32.), rems(1.), rems(0.125)));
        assert_eq!(DIALOG_CLOSE_OPACITY, 0.7);
    }

    /// A popover is the popover pair with a border, rounded-md, w-72.
    #[test]
    fn popover_colours_come_from_the_tokens() {
        for t in [Tokens::neutral_light(), Tokens::neutral_dark()] {
            assert_eq!(
                popover_style(&t),
                PopoverStyle {
                    background: t.popover,
                    foreground: t.popover_foreground,
                    border: t.border,
                    radius: t.radius_md(),
                }
            );
        }
        assert_eq!(POPOVER_WIDTH, rems(18.));
    }

    /// A select's trigger is input-bordered with a muted placeholder; its
    /// menu is the popover pair with the accent pair for the highlight.
    #[test]
    fn select_colours_come_from_the_tokens() {
        for t in [Tokens::neutral_light(), Tokens::neutral_dark()] {
            assert_eq!(
                select_style(&t),
                SelectStyle {
                    border: t.input,
                    foreground: t.foreground,
                    placeholder: t.muted_foreground,
                    menu_background: t.popover,
                    menu_foreground: t.popover_foreground,
                    menu_border: t.border,
                    highlight: t.accent,
                    highlight_foreground: t.accent_foreground,
                    radius: t.radius_md(),
                    option_radius: t.radius_sm(),
                }
            );
        }
        // min-w-[8rem], max-h-96, a size-3.5 tick, opacity-50 icons.
        assert_eq!((SELECT_MENU_MIN_WIDTH, SELECT_MENU_MAX_HEIGHT, SELECT_TICK_SIZE), (rems(8.), rems(24.), rems(0.875)));
        assert_eq!(MUTED_ICON_OPACITY, 0.5);
    }

    /// A command list is the popover pair, muted headings and shortcuts,
    /// and the accent pair for the highlight.
    #[test]
    fn command_colours_come_from_the_tokens() {
        for t in [Tokens::neutral_light(), Tokens::neutral_dark()] {
            assert_eq!(
                command_style(&t),
                CommandStyle {
                    background: t.popover,
                    foreground: t.popover_foreground,
                    border: t.border,
                    muted: t.muted_foreground,
                    highlight: t.accent,
                    highlight_foreground: t.accent_foreground,
                    radius: t.radius_md(),
                    item_radius: t.radius_sm(),
                }
            );
        }
        // max-h-[300px] at the 16px root size.
        assert_eq!(COMMAND_LIST_MAX_HEIGHT, rems(18.75));
    }

    #[test]
    fn tables_cover_every_variant_and_size() {
        let rows: Vec<_> = variant_rows().iter().map(|r| r.variant).collect();
        assert_eq!(rows, ButtonVariant::ALL);
        let sizes: Vec<_> = SIZE_ROWS.iter().map(|r| r.0).collect();
        assert_eq!(sizes, ButtonSize::ALL);
    }
}
