//! Variant and size styles as data: the role `cva` plays in shadcn/ui.
//!
//! Each component's look is a pure function of its variant, its size and the
//! active [`Tokens`], so the whole table is unit-tested and a component only
//! applies the result. Sizes are in rems (GPUI's `h_9` is `rems(2.25)`, as
//! Tailwind's `h-9` is 2.25rem), so they scale with the window's text size.

use gpui::{Corners, Edges, FontWeight, Hsla, Pixels, Rems, px, rems, transparent_black};

use crate::theme::Tokens;

/// The focus ring's opacity, shadcn's `ring-ring/50`.
pub const RING_OPACITY: f32 = 0.5;

/// The ring's opacity on an invalid input, shadcn's `ring-destructive/20`.
pub const INVALID_RING_OPACITY: f32 = 0.2;

/// The focus ring's colour: the ring token at [`RING_OPACITY`].
pub fn focus_ring(t: &Tokens) -> Hsla {
    t.ring.opacity(RING_OPACITY)
}

/// Where the focus ring's box goes, relative to the control it rings: a
/// border `width` wide, drawn just outside the control's border box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingGeometry {
    /// How far each edge of the ring's box lies outside the control's
    /// padding box, which is what an absolutely positioned child's insets
    /// are measured from (taffy, as CSS): the control's border plus the
    /// ring's width.
    pub outset: Edges<Pixels>,
    /// The ring box's corners: the control's, grown by the ring's width,
    /// so the ring's inner edge follows the control's outer edge.
    pub radii: Corners<Pixels>,
    /// The ring's border width.
    pub width: Pixels,
}

/// The ring around a control with these `border` widths and corner `radii`
/// (shadcn's `ring-[3px]`, a box shadow with a spread and no blur, which
/// sits outside the border box and never under the control).
pub fn ring_geometry(border: Edges<Pixels>, radii: Corners<Pixels>, width: Pixels) -> RingGeometry {
    RingGeometry {
        outset: Edges {
            top: border.top + width,
            right: border.right + width,
            bottom: border.bottom + width,
            left: border.left + width,
        },
        radii: Corners {
            top_left: radii.top_left + width,
            top_right: radii.top_right + width,
            bottom_right: radii.bottom_right + width,
            bottom_left: radii.bottom_left + width,
        },
        width,
    }
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

/// shadcn's button sizes. `Icon`, `IconSm` (`icon-sm`) and `IconXs`
/// (`icon-xs`) are squares for an icon-only button.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ButtonSize {
    #[default]
    Default,
    Sm,
    Lg,
    Icon,
    IconSm,
    IconXs,
}

impl ButtonSize {
    pub const ALL: [ButtonSize; 6] = [
        ButtonSize::Default,
        ButtonSize::Sm,
        ButtonSize::Lg,
        ButtonSize::Icon,
        ButtonSize::IconSm,
        ButtonSize::IconXs,
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
    /// The focus ring's colour while it shows.
    pub ring: Hsla,
    /// The border's colour while the ring shows, for a variant that has a
    /// border (shadcn's `focus-visible:border-ring`).
    pub focus_border: Option<Hsla>,
    pub height: Rems,
    /// A fixed width, for the square icon size.
    pub width: Option<Rems>,
    pub padding_x: Rems,
    /// The horizontal padding when the label has a leading icon
    /// (shadcn's `has-[>svg]:px-*`).
    pub padding_x_with_icon: Rems,
    /// Space between a leading icon and the label.
    pub gap: Rems,
    /// The icon's side: 1rem (`size-4`), 0.75rem in `IconXs`
    /// (`[&_svg:not([class*='size-'])]:size-3`).
    pub icon_size: Rems,
    pub text_size: Rems,
    pub radius: Rems,
}

/// shadcn's `buttonVariants` (the new-york-v4 style) for `variant` and
/// `size`, in the colours of `t`.
///
/// In a dark set ([`Tokens::dark`]) the variants take shadcn's `dark:`
/// refinements: the destructive fill at 60% and its ring at 40%, the
/// outline's input-coloured border and translucent input fill, and the
/// ghost's half-faded hover. Where a `dark:` class and a `hover:` or
/// `focus-visible:` class set the same property, the colour is the one
/// shadcn's compiled CSS shows: the two rules have the same specificity and
/// the `dark:` one comes later, so it wins. The base's
/// `dark:aria-invalid:ring-destructive/40` is left out, since a button has no
/// invalid state here.
pub fn button_style(variant: ButtonVariant, size: ButtonSize, t: &Tokens) -> ButtonStyle {
    let dark = t.dark;
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
        // bg-destructive text-white hover:bg-destructive/90, and
        // dark:bg-destructive/60, which also holds on hover: a dark
        // destructive button doesn't change on hover.
        ButtonVariant::Destructive => {
            let (fill, hover) = if dark {
                (t.destructive.opacity(0.6), t.destructive.opacity(0.6))
            } else {
                (t.destructive, t.destructive.opacity(0.9))
            };
            (Some(fill), t.destructive_foreground, None, Some(hover), t.destructive_foreground)
        }
        // border bg-background hover:bg-accent hover:text-accent-foreground,
        // and dark:border-input dark:bg-input/30 dark:hover:bg-input/50
        ButtonVariant::Outline if dark => (
            Some(t.input.opacity(0.3)),
            t.foreground,
            Some(t.input),
            Some(t.input.opacity(0.5)),
            t.accent_foreground,
        ),
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
        // hover:bg-accent hover:text-accent-foreground, and
        // dark:hover:bg-accent/50
        ButtonVariant::Ghost => (
            None,
            t.foreground,
            None,
            Some(if dark { t.accent.opacity(0.5) } else { t.accent }),
            t.accent_foreground,
        ),
        // text-primary underline-offset-4 hover:underline
        ButtonVariant::Link => (None, t.primary, None, None, t.primary),
    };

    // focus-visible:ring-ring/50, which the destructive variant replaces
    // with focus-visible:ring-destructive/20 dark:focus-visible:ring-destructive/40.
    let ring = match variant {
        ButtonVariant::Destructive => t.destructive.opacity(if dark { 0.4 } else { 0.2 }),
        _ => focus_ring(t),
    };
    // focus-visible:border-ring, except that the dark outline's
    // dark:border-input wins and keeps the input colour.
    let focus_border =
        border.map(|border| if dark && variant == ButtonVariant::Outline { border } else { t.ring });

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
        // size-8
        ButtonSize::IconSm => (rems(2.), Some(rems(2.)), rems(0.), rems(0.), rems(0.5)),
        // size-6
        ButtonSize::IconXs => (rems(1.5), Some(rems(1.5)), rems(0.), rems(0.), rems(0.5)),
    };
    let icon_size = if size == ButtonSize::IconXs { rems(0.75) } else { rems(1.) };

    ButtonStyle {
        background,
        foreground,
        border,
        hover_background,
        hover_foreground,
        hover_underline: variant == ButtonVariant::Link,
        ring,
        focus_border,
        height,
        width,
        padding_x,
        padding_x_with_icon,
        gap,
        icon_size,
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
/// thumb would all but vanish on the translucent unchecked track. The
/// track's `dark:data-[state=unchecked]:bg-input/80` is left out.
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

/// A command group's padding on every side, shadcn's CommandGroup `p-1`.
pub const COMMAND_GROUP_PADDING: Rems = Rems(0.25);

/// The space around one row of a command list, which stands for the group
/// it is in: each row is inset by the group's padding at the sides, the
/// group's first row also at the top and its last at the bottom. A row's
/// background (the highlight) fills the group's inner width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CommandRowInsets {
    pub top: Rems,
    pub bottom: Rems,
    /// Left and right.
    pub x: Rems,
}

/// The insets of a row that is or isn't its group's first and last row (a
/// group of one row is both).
pub fn command_row_insets(first_in_group: bool, last_in_group: bool) -> CommandRowInsets {
    let edge = |at_edge: bool| if at_edge { COMMAND_GROUP_PADDING } else { Rems(0.) };
    CommandRowInsets { top: edge(first_in_group), bottom: edge(last_in_group), x: COMMAND_GROUP_PADDING }
}

/// A command separator's height, shadcn's `h-px`: one pixel, not a rem
/// step, as in Tailwind.
pub const COMMAND_SEPARATOR_HEIGHT: Pixels = px(1.);

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
    /// The line between two groups.
    pub separator: Hsla,
    /// The list's corners.
    pub radius: Rems,
    /// An item's corners.
    pub item_radius: Rems,
}

/// shadcn's Command: `bg-popover text-popover-foreground rounded-md`, the
/// input row's `border-b`, headings and shortcuts in `text-muted-foreground`,
/// and the item `data-[selected=true]:bg-accent
/// data-[selected=true]:text-accent-foreground rounded-sm`, and
/// CommandSeparator's `bg-border`.
pub fn command_style(t: &Tokens) -> CommandStyle {
    CommandStyle {
        background: t.popover,
        foreground: t.popover_foreground,
        border: t.border,
        muted: t.muted_foreground,
        highlight: t.accent,
        highlight_foreground: t.accent_foreground,
        separator: t.border,
        radius: t.radius_md(),
        item_radius: t.radius_sm(),
    }
}

/// shadcn's toggle variants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ToggleVariant {
    #[default]
    Default,
    Outline,
}

impl ToggleVariant {
    pub const ALL: [ToggleVariant; 2] = [ToggleVariant::Default, ToggleVariant::Outline];
}

/// shadcn's toggle sizes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ToggleSize {
    #[default]
    Default,
    Sm,
    Lg,
}

impl ToggleSize {
    pub const ALL: [ToggleSize; 3] = [ToggleSize::Default, ToggleSize::Sm, ToggleSize::Lg];
}

/// A toggle's resolved look. `None` colours draw nothing (a transparent
/// background, no border).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToggleStyle {
    pub background: Option<Hsla>,
    pub foreground: Hsla,
    pub border: Option<Hsla>,
    pub hover_background: Hsla,
    pub hover_foreground: Hsla,
    /// The focus ring's colour while it shows.
    pub ring: Hsla,
    /// The border's colour while the ring shows, for the outline variant
    /// (shadcn's `focus-visible:border-ring`).
    pub focus_border: Option<Hsla>,
    pub height: Rems,
    /// `min-w-*`: an icon or a one-letter label makes a square.
    pub min_width: Rems,
    pub padding_x: Rems,
    /// Space between an icon and the label, shadcn's `gap-2`.
    pub gap: Rems,
    /// The icon's side, shadcn's `size-4`.
    pub icon_size: Rems,
    pub text_size: Rems,
    pub radius: Rems,
}

/// shadcn's `toggleVariants` (the new-york-v4 style) for `variant`, `size`
/// and the `pressed` state, in the colours of `t`.
///
/// The base is `hover:bg-muted hover:text-muted-foreground
/// data-[state=on]:bg-accent data-[state=on]:text-accent-foreground`; the
/// outline variant adds `border border-input bg-transparent
/// hover:bg-accent hover:text-accent-foreground`. A pressed toggle keeps the
/// accent pair under the pointer: the `data-[state=on]:` rule has the same
/// specificity as the `hover:` one and comes later in Tailwind's output, so
/// it wins. The toggle has no `dark:` refinements besides the invalid ring,
/// which is left out (a toggle has no invalid state here).
pub fn toggle_style(variant: ToggleVariant, size: ToggleSize, pressed: bool, t: &Tokens) -> ToggleStyle {
    let (border, mut hover_background, mut hover_foreground) = match variant {
        ToggleVariant::Default => (None, t.muted, t.muted_foreground),
        ToggleVariant::Outline => (Some(t.input), t.accent, t.accent_foreground),
    };
    let (background, foreground) = if pressed {
        hover_background = t.accent;
        hover_foreground = t.accent_foreground;
        (Some(t.accent), t.accent_foreground)
    } else {
        (None, t.foreground)
    };
    // (height and min width, px): h-9 px-2 min-w-9, h-8 px-1.5 min-w-8,
    // h-10 px-2.5 min-w-10.
    let (height, padding_x) = match size {
        ToggleSize::Default => (rems(2.25), rems(0.5)),
        ToggleSize::Sm => (rems(2.), rems(0.375)),
        ToggleSize::Lg => (rems(2.5), rems(0.625)),
    };
    ToggleStyle {
        background,
        foreground,
        border,
        hover_background,
        hover_foreground,
        ring: focus_ring(t),
        focus_border: border.map(|_| t.ring),
        height,
        min_width: height,
        padding_x,
        gap: rems(0.5),
        icon_size: rems(1.),
        // text-sm rounded-md
        text_size: rems(0.875),
        radius: t.radius_md(),
    }
}

/// A tooltip's resolved look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TooltipStyle {
    /// `bg-foreground`: the tooltip is drawn inverted.
    pub background: Hsla,
    /// `text-background`.
    pub foreground: Hsla,
    /// The shortcut's keycaps inside it: `bg-background/20`
    /// (`dark:bg-background/10`).
    pub kbd_background: Hsla,
    /// `text-background`.
    pub kbd_foreground: Hsla,
    /// `px-3`.
    pub padding_x: Rems,
    /// `py-1.5`.
    pub padding_y: Rems,
    /// The right padding beside a shortcut, `pr-1.5`.
    pub padding_right_with_shortcut: Rems,
    /// Between the label and the shortcut, `gap-2`.
    pub gap: Rems,
    /// `text-xs`.
    pub text_size: Rems,
    /// `rounded-md`.
    pub radius: Rems,
}

/// shadcn's TooltipContent (`bg-foreground text-background rounded-md px-3
/// py-1.5 text-xs`), and its Kbd example (`pr-1.5`, the label and the keys
/// `gap-2`), with the keycaps in the Kbd's tooltip colours
/// (`[[data-slot=tooltip-content]_&]:bg-background/20 text-background
/// dark:…bg-background/10`).
pub fn tooltip_style(t: &Tokens) -> TooltipStyle {
    TooltipStyle {
        background: t.foreground,
        foreground: t.background,
        kbd_background: t.background.opacity(if t.dark { 0.1 } else { 0.2 }),
        kbd_foreground: t.background,
        padding_x: rems(0.75),
        padding_y: rems(0.375),
        padding_right_with_shortcut: rems(0.375),
        gap: rems(0.5),
        text_size: rems(0.75),
        radius: t.radius_md(),
    }
}

/// A menu's least width, shadcn's `min-w-[8rem]`.
pub const MENU_MIN_WIDTH: Rems = Rems(8.);

/// A menu's resolved look: the panel, its items and separators.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuStyle {
    /// The panel: `bg-popover text-popover-foreground border`.
    pub background: Hsla,
    pub foreground: Hsla,
    pub border: Hsla,
    /// The highlighted item: `focus:bg-accent focus:text-accent-foreground`.
    pub highlight: Hsla,
    pub highlight_foreground: Hsla,
    /// An item's description line and its shortcut's keycaps:
    /// `text-muted-foreground`, on the `muted` keycaps of `Kbd`.
    pub muted: Hsla,
    /// A separator: `bg-border`.
    pub separator: Hsla,
    /// The panel's corners, `rounded-md`.
    pub radius: Rems,
    /// An item's corners, `rounded-sm`.
    pub item_radius: Rems,
    /// An item's label, `text-sm`.
    pub text_size: Rems,
    /// A description line, `text-xs`.
    pub description_size: Rems,
}

/// shadcn's DropdownMenu and ContextMenu (new-york-v4): the content
/// `bg-popover text-popover-foreground rounded-md border p-1 shadow-md
/// min-w-[8rem]`, the item `rounded-sm px-2 py-1.5 text-sm gap-2
/// focus:bg-accent focus:text-accent-foreground data-[disabled]:opacity-50`,
/// the separator `-mx-1 my-1 h-px bg-border` and the shortcut
/// `text-muted-foreground ml-auto`, in the colours of `t`. The description
/// line is the crate's addition, in the shortcut's muted colour at
/// `text-xs`.
pub fn menu_style(t: &Tokens) -> MenuStyle {
    MenuStyle {
        background: t.popover,
        foreground: t.popover_foreground,
        border: t.border,
        highlight: t.accent,
        highlight_foreground: t.accent_foreground,
        muted: t.muted_foreground,
        separator: t.border,
        radius: t.radius_md(),
        item_radius: t.radius_sm(),
        text_size: rems(0.875),
        description_size: rems(0.75),
    }
}

/// shadcn's tab list variants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TabVariant {
    /// shadcn's default: a muted pill, the active trigger raised on the
    /// background.
    #[default]
    Default,
    /// shadcn's `line`: no surfaces; the active trigger in the foreground
    /// with a primary line along its top.
    Line,
}

impl TabVariant {
    pub const ALL: [TabVariant; 2] = [TabVariant::Default, TabVariant::Line];
}

/// The line along an active `Line` trigger's top edge, and the bar that
/// marks where a dragged trigger will land: two pixels, as shadcn's
/// `h-0.5` underline.
pub const TAB_LINE_WIDTH: Pixels = px(2.);

/// A list's height, shadcn's `h-9`.
pub const TAB_LIST_HEIGHT: Rems = Rems(2.25);

/// The room around a list's triggers, shadcn's `p-[3px]`, in pixels as
/// shadcn's is. It is also the focus ring's width, so the active trigger's
/// ring stays inside the list.
pub const TAB_LIST_PADDING: Pixels = px(3.);

/// The widest a dragged trigger's preview is drawn.
pub const TAB_DRAG_MAX_WIDTH: Rems = Rems(15.);

/// A tab list's resolved look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabListStyle {
    /// `bg-muted` for the pill, nothing for `line`.
    pub background: Option<Hsla>,
    /// `h-9`.
    pub height: Rems,
    /// `p-[3px]`.
    pub padding: Edges<Pixels>,
    /// Between triggers: none in the pill, `gap-1` in `line`.
    pub gap: Rems,
    /// `rounded-lg`; `line` is square.
    pub radius: Rems,
}

/// shadcn's TabsList (`bg-muted rounded-lg p-[3px] h-9`, and the line
/// variant's `gap-1 bg-transparent rounded-none`), in the colours of `t`.
pub fn tab_list_style(variant: TabVariant, t: &Tokens) -> TabListStyle {
    let all = Edges::all(TAB_LIST_PADDING);
    match variant {
        TabVariant::Default => TabListStyle {
            background: Some(t.muted),
            height: TAB_LIST_HEIGHT,
            padding: all,
            gap: rems(0.),
            radius: t.radius_lg(),
        },
        TabVariant::Line => TabListStyle {
            background: None,
            height: TAB_LIST_HEIGHT,
            padding: all,
            gap: rems(0.25),
            radius: rems(0.),
        },
    }
}

/// A trigger's own colours, which its owner may give it (a terminal tab
/// drawn in its terminal's colours). A colour left `None` comes from the
/// variant's tokens, so a trigger given none looks as shadcn's does.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TabColors {
    /// The surface the trigger is drawn on, active or not.
    pub surface: Option<Hsla>,
    /// The text colour: full when active or hovered, at 60% opacity
    /// otherwise.
    pub text: Option<Hsla>,
    /// The line along the active trigger's top edge (`Line` variant), the
    /// `primary` token when not given.
    pub line: Option<Hsla>,
}

/// A tab trigger's resolved look. `None` colours draw nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabStyle {
    pub background: Option<Hsla>,
    pub foreground: Hsla,
    /// A 1px border in this colour (often transparent, shadcn's
    /// `border-transparent`); `None` for no border at all.
    pub border: Option<Hsla>,
    pub hover_background: Option<Hsla>,
    pub hover_foreground: Hsla,
    /// The [`TAB_LINE_WIDTH`] line along the top edge.
    pub top_line: Option<Hsla>,
    /// shadcn's `shadow-sm` under the active pill.
    pub shadow: bool,
    /// The focus ring's colour while it shows.
    pub ring: Hsla,
    /// The border's colour while the ring shows (`focus-visible:border-ring`).
    pub focus_border: Option<Hsla>,
    pub radius: Rems,
    pub padding_left: Rems,
    pub padding_right: Rems,
    /// The right padding beside a close button.
    pub padding_right_with_close: Rems,
    /// Between the leading slot, the label and the adornments, `gap-1.5`.
    pub gap: Rems,
    /// A leading icon's side, `size-4`.
    pub icon_size: Rems,
    pub text_size: Rems,
    pub font_weight: FontWeight,
}

/// shadcn's TabsTrigger (new-york-v4) for `variant` and the `active` state,
/// in the colours of `t`; `colors` are the trigger's own, if its owner gave
/// any.
///
/// The pill and line triggers are `border border-transparent rounded-md
/// px-2 py-1 text-sm font-medium gap-1.5`, inactive in
/// `text-foreground/60 hover:text-foreground` (`dark:text-muted-foreground
/// dark:hover:text-foreground`), active in `text-foreground`; the active
/// pill is `bg-background shadow-sm` (`dark:bg-input/30
/// dark:border-input`), and the active line trigger has no surface and,
/// per the spec, a primary line along its top where shadcn draws a
/// foreground underline below. Focused from the keyboard, both take
/// `focus-visible:border-ring` and the ring.
///
/// The owner's colours (an addition to shadcn) apply in both states: a
/// `surface` replaces the trigger's background, an inactive one included;
/// `text` replaces the foreground, at 60% opacity while inactive and not
/// hovered and at full opacity when active or hovered; `line` replaces
/// `primary` as the active `Line` trigger's top line.
pub fn tab_style(variant: TabVariant, active: bool, colors: TabColors, t: &Tokens) -> TabStyle {
    let dark = t.dark;
    let inactive_text = if dark { t.muted_foreground } else { t.foreground.opacity(0.6) };
    let shadcn = |background, foreground, border, top_line, shadow| TabStyle {
        background,
        foreground,
        border: Some(border),
        hover_background: None,
        hover_foreground: t.foreground,
        top_line,
        shadow,
        ring: focus_ring(t),
        focus_border: Some(t.ring),
        radius: t.radius_md(),
        // px-2
        padding_left: rems(0.5),
        padding_right: rems(0.5),
        padding_right_with_close: rems(0.25),
        gap: rems(0.375),
        icon_size: rems(1.),
        text_size: rems(0.875),
        font_weight: FontWeight::MEDIUM,
    };
    let clear = transparent_black();
    let mut style = match (variant, active) {
        (_, false) => shadcn(None, inactive_text, clear, None, false),
        (TabVariant::Default, true) => {
            let (background, border) = if dark { (t.input.opacity(0.3), t.input) } else { (t.background, clear) };
            shadcn(Some(background), t.foreground, border, None, true)
        }
        (TabVariant::Line, true) => shadcn(None, t.foreground, clear, Some(colors.line.unwrap_or(t.primary)), false),
    };
    if let Some(surface) = colors.surface {
        style.background = Some(surface);
    }
    if let Some(text) = colors.text {
        style.foreground = if active { text } else { text.opacity(0.6) };
        style.hover_foreground = text;
    }
    style
}

/// How narrow and how wide a trigger may get: it shrinks towards `min`
/// when the list is crowded, grows with its label up to `max`, and past
/// that the label is cut with an ellipsis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabWidth {
    min: Rems,
    max: Rems,
}

impl TabWidth {
    /// Limits of `min` and `max`. A `max` below `min` is raised to it: as in
    /// CSS, the minimum wins.
    pub fn new(min: Rems, max: Rems) -> Self {
        Self { min, max: Rems(max.0.max(min.0)) }
    }

    pub fn min(&self) -> Rems {
        self.min
    }

    pub fn max(&self) -> Rems {
        self.max
    }

    /// The width a trigger is drawn at when its content would take
    /// `natural`, room allowing: its natural width, held between the two.
    /// A crowded list shrinks it further, but never below `min`.
    pub fn clamp(&self, natural: Rems) -> Rems {
        Rems(natural.0.min(self.max.0).max(self.min.0))
    }
}

/// A dragged trigger's preview: the background token with a primary
/// border, as the tab being moved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabDragStyle {
    pub background: Hsla,
    pub foreground: Hsla,
    pub border: Hsla,
    pub radius: Rems,
}

pub fn tab_drag_style(t: &Tokens) -> TabDragStyle {
    TabDragStyle { background: t.background, foreground: t.foreground, border: t.primary, radius: t.radius_md() }
}

#[cfg(test)]
mod tests {
    use gpui::hsla;

    use super::*;

    type Pick = fn(&Tokens) -> Hsla;

    /// A token and the opacity it is drawn at.
    type Faded = (Pick, f32);

    /// What each variant's colours must be, named by token.
    struct VariantRow {
        variant: ButtonVariant,
        background: Option<Faded>,
        foreground: Pick,
        border: Option<Pick>,
        hover_background: Option<Faded>,
        hover_foreground: Pick,
        hover_underline: bool,
        ring: Faded,
        focus_border: Option<Pick>,
    }

    /// The rows for a light set, or with `dark` for a dark one, where
    /// shadcn's `dark:` classes change the destructive, outline and ghost
    /// variants.
    fn variant_rows(dark: bool) -> [VariantRow; 6] {
        // focus-visible:ring-ring/50
        let ring: Faded = (|t| t.ring, 0.5);
        [
            VariantRow {
                variant: ButtonVariant::Default,
                background: Some((|t| t.primary, 1.0)),
                foreground: |t| t.primary_foreground,
                border: None,
                hover_background: Some((|t| t.primary, 0.9)),
                hover_foreground: |t| t.primary_foreground,
                hover_underline: false,
                ring,
                focus_border: None,
            },
            VariantRow {
                variant: ButtonVariant::Destructive,
                // dark:bg-destructive/60, on hover too.
                background: Some((|t| t.destructive, if dark { 0.6 } else { 1.0 })),
                foreground: |t| t.destructive_foreground,
                border: None,
                hover_background: Some((|t| t.destructive, if dark { 0.6 } else { 0.9 })),
                hover_foreground: |t| t.destructive_foreground,
                hover_underline: false,
                // focus-visible:ring-destructive/20 dark:focus-visible:ring-destructive/40
                ring: (|t| t.destructive, if dark { 0.4 } else { 0.2 }),
                focus_border: None,
            },
            if dark {
                // dark:border-input dark:bg-input/30 dark:hover:bg-input/50,
                // and the input border holds while focused.
                VariantRow {
                    variant: ButtonVariant::Outline,
                    background: Some((|t| t.input, 0.3)),
                    foreground: |t| t.foreground,
                    border: Some(|t| t.input),
                    hover_background: Some((|t| t.input, 0.5)),
                    hover_foreground: |t| t.accent_foreground,
                    hover_underline: false,
                    ring,
                    focus_border: Some(|t| t.input),
                }
            } else {
                VariantRow {
                    variant: ButtonVariant::Outline,
                    background: Some((|t| t.background, 1.0)),
                    foreground: |t| t.foreground,
                    border: Some(|t| t.border),
                    hover_background: Some((|t| t.accent, 1.0)),
                    hover_foreground: |t| t.accent_foreground,
                    hover_underline: false,
                    ring,
                    focus_border: Some(|t| t.ring),
                }
            },
            VariantRow {
                variant: ButtonVariant::Secondary,
                background: Some((|t| t.secondary, 1.0)),
                foreground: |t| t.secondary_foreground,
                border: None,
                hover_background: Some((|t| t.secondary, 0.8)),
                hover_foreground: |t| t.secondary_foreground,
                hover_underline: false,
                ring,
                focus_border: None,
            },
            VariantRow {
                variant: ButtonVariant::Ghost,
                background: None,
                foreground: |t| t.foreground,
                border: None,
                // dark:hover:bg-accent/50
                hover_background: Some((|t| t.accent, if dark { 0.5 } else { 1.0 })),
                hover_foreground: |t| t.accent_foreground,
                hover_underline: false,
                ring,
                focus_border: None,
            },
            VariantRow {
                variant: ButtonVariant::Link,
                background: None,
                foreground: |t| t.primary,
                border: None,
                hover_background: None,
                hover_foreground: |t| t.primary,
                hover_underline: true,
                ring,
                focus_border: None,
            },
        ]
    }

    /// The style `row` and the size row describe, in the colours of `t`.
    fn expected_style(row: &VariantRow, size_row: SizeRow, t: &Tokens) -> ButtonStyle {
        let (_, height, width, px, px_icon, gap, icon) = size_row;
        let faded = |(pick, opacity): Faded| pick(t).opacity(opacity);
        ButtonStyle {
            background: row.background.map(faded),
            foreground: (row.foreground)(t),
            border: row.border.map(|pick| pick(t)),
            hover_background: row.hover_background.map(faded),
            hover_foreground: (row.hover_foreground)(t),
            hover_underline: row.hover_underline,
            ring: faded(row.ring),
            focus_border: row.focus_border.map(|pick| pick(t)),
            height: rems(height),
            width: width.map(rems),
            padding_x: rems(px),
            padding_x_with_icon: rems(px_icon),
            gap: rems(gap),
            icon_size: rems(icon),
            text_size: rems(0.875),
            radius: t.radius_md(),
        }
    }

    /// (size, height, width, px, px with an icon, gap, icon side), in rems.
    type SizeRow = (ButtonSize, f32, Option<f32>, f32, f32, f32, f32);

    const SIZE_ROWS: [SizeRow; 6] = [
        (ButtonSize::Default, 2.25, None, 1.0, 0.75, 0.5, 1.0),
        (ButtonSize::Sm, 2.0, None, 0.75, 0.625, 0.375, 1.0),
        (ButtonSize::Lg, 2.5, None, 1.5, 1.0, 0.5, 1.0),
        (ButtonSize::Icon, 2.25, Some(2.25), 0.0, 0.0, 0.5, 1.0),
        (ButtonSize::IconSm, 2.0, Some(2.0), 0.0, 0.0, 0.5, 1.0),
        (ButtonSize::IconXs, 1.5, Some(1.5), 0.0, 0.0, 0.5, 0.75),
    ];

    #[test]
    fn every_variant_and_size_against_both_sets() {
        // The rows are chosen by the set, not by its hint, so a wrong hint
        // fails here too.
        let sets = [
            ("light", Tokens::neutral_light(), false),
            ("dark", Tokens::neutral_dark(), true),
        ];
        let mut combinations = 0;
        for (set, t, dark) in &sets {
            for row in variant_rows(*dark) {
                for size_row in SIZE_ROWS {
                    let case = format!("{set} {:?} {:?}", row.variant, size_row.0);
                    assert_eq!(button_style(row.variant, size_row.0, t), expected_style(&row, size_row, t), "{case}");
                    combinations += 1;
                }
            }
        }
        // 6 variants x 6 sizes, in each of the two sets.
        assert_eq!(combinations, 2 * 36);
        assert_eq!(ButtonVariant::ALL.len() * ButtonSize::ALL.len(), 36);
    }

    /// The two new icon sizes are shadcn's `size-8` and `size-6` squares,
    /// the existing sizes are untouched, and only `IconXs` shrinks its icon.
    #[test]
    fn icon_sizes_are_squares_of_two_and_one_and_a_half_rems() {
        let t = Tokens::neutral_light();
        let style = |size| button_style(ButtonVariant::Ghost, size, &t);
        for (size, side, icon) in [
            (ButtonSize::Icon, 2.25, 1.),
            (ButtonSize::IconSm, 2., 1.),
            (ButtonSize::IconXs, 1.5, 0.75),
        ] {
            let s = style(size);
            assert_eq!((s.height, s.width, s.icon_size), (rems(side), Some(rems(side)), rems(icon)), "{size:?}");
            assert_eq!((s.padding_x, s.padding_x_with_icon), (rems(0.), rems(0.)), "{size:?}");
        }
        for size in [ButtonSize::Default, ButtonSize::Sm, ButtonSize::Lg] {
            assert_eq!(style(size).width, None, "{size:?}");
            assert_eq!(style(size).icon_size, rems(1.), "{size:?}");
        }
        // Heights of the existing sizes: 2.25, 2, 2.5 rem.
        assert_eq!(style(ButtonSize::Default).height, rems(2.25));
        assert_eq!(style(ButtonSize::Sm).height, rems(2.));
        assert_eq!(style(ButtonSize::Lg).height, rems(2.5));
    }

    /// The hint alone picks the `dark:` refinements: the dark colours with
    /// the flag cleared follow the light rows.
    #[test]
    fn the_dark_hint_picks_the_dark_refinements() {
        let dark_unflagged = Tokens { dark: false, ..Tokens::neutral_dark() };
        for row in variant_rows(false) {
            for size_row in SIZE_ROWS {
                let (style, case) = (button_style(row.variant, size_row.0, &dark_unflagged), (row.variant, size_row.0));
                assert_eq!(style, expected_style(&row, size_row, &dark_unflagged), "{case:?}");
            }
        }
    }

    /// The `dark:` values in the neutral dark set, as alphas: the input
    /// colour is white at 15%, so `bg-input/30` is white at 4.5%.
    #[test]
    fn dark_refinements_in_the_neutral_dark_set() {
        let t = Tokens::neutral_dark();
        let style = |v| button_style(v, ButtonSize::Default, &t);
        let close = |a: f32, b: f32| (a - b).abs() < 1e-6;
        let white = |c: Hsla| c.l == 1. && c.s == 0.;

        let outline = style(ButtonVariant::Outline);
        let (fill, hover) = (outline.background.unwrap(), outline.hover_background.unwrap());
        assert!(white(fill) && close(fill.a, 0.045), "dark:bg-input/30 is {fill:?}");
        assert!(white(hover) && close(hover.a, 0.075), "dark:hover:bg-input/50 is {hover:?}");
        assert_eq!(outline.border, Some(hsla(0., 0., 1., 0.15)));
        assert_eq!(outline.focus_border, outline.border);

        let destructive = style(ButtonVariant::Destructive);
        let fill = destructive.background.unwrap();
        assert_eq!((fill.h, fill.s, fill.l), (t.destructive.h, t.destructive.s, t.destructive.l));
        assert!(close(fill.a, 0.6), "dark:bg-destructive/60 is {fill:?}");
        assert_eq!(destructive.hover_background, Some(fill));
        assert!(close(destructive.ring.a, 0.4), "dark:focus-visible:ring-destructive/40");
        assert_eq!(Hsla { a: 1., ..destructive.ring }, t.destructive);

        assert!(close(style(ButtonVariant::Ghost).hover_background.unwrap().a, 0.5), "dark:hover:bg-accent/50");
        // The variants without dark: classes are as in a light set.
        assert_eq!(style(ButtonVariant::Default).ring, focus_ring(&t));
        assert_eq!(style(ButtonVariant::Secondary).hover_background, Some(t.secondary.opacity(0.8)));
    }

    #[test]
    fn hover_fades_where_shadcn_uses_opacity() {
        for t in [Tokens::neutral_light(), Tokens::neutral_dark()] {
            let hover = |v| button_style(v, ButtonSize::Default, &t).hover_background.unwrap();
            // bg-primary/90, and bg-destructive/90 (a dark set's /60 holds on hover)
            let primary = hover(ButtonVariant::Default);
            assert_eq!((primary.h, primary.s, primary.l), (t.primary.h, t.primary.s, t.primary.l));
            assert!((primary.a - 0.9).abs() < 1e-6);
            let destructive = if t.dark { 0.6 } else { 0.9 };
            assert!((hover(ButtonVariant::Destructive).a - destructive).abs() < 1e-6);
            // bg-secondary/80
            assert!((hover(ButtonVariant::Secondary).a - 0.8).abs() < 1e-6);
            // hover:bg-accent, opaque, and dark:hover:bg-accent/50
            assert_eq!(hover(ButtonVariant::Ghost), if t.dark { t.accent.opacity(0.5) } else { t.accent });
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

    /// The ring lies just outside each control's border box: its box is
    /// outset from the padding box by the border plus the ring's width, and
    /// its corners grow by that width, so it never overlaps the control.
    #[test]
    fn ring_sits_outside_the_border_box() {
        use gpui::px;
        let edges = |w: f32| Edges::all(px(w));
        let corners = |r: f32| Corners::all(px(r));
        // (control, border, radius, expected outset, expected radius)
        let cases = [
            // A button without a border (default, ghost, link variants), rounded-md.
            ("borderless button", edges(0.), corners(8.), edges(3.), corners(11.)),
            // An outline button, an input, a select trigger, a checkbox.
            ("1px border", edges(1.), corners(8.), edges(4.), corners(11.)),
            // The dialog's close button: rounded-xs, no border.
            ("close button", edges(0.), corners(2.), edges(3.), corners(5.)),
            // A switch: rounded-full (gpui's 9999px, clamped when painted).
            ("switch", edges(1.), corners(9999.), edges(4.), corners(10002.)),
        ];
        for (control, border, radii, outset, ring_radii) in cases {
            let ring = ring_geometry(border, radii, px(3.));
            assert_eq!(ring, RingGeometry { outset, radii: ring_radii, width: px(3.) }, "{control}");
        }
        // Uneven sides keep their own widths.
        let border = Edges { top: px(0.), right: px(1.), bottom: px(2.), left: px(0.) };
        let ring = ring_geometry(border, Corners::default(), px(3.));
        assert_eq!(ring.outset, Edges { top: px(3.), right: px(4.), bottom: px(5.), left: px(3.) });
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
        const { assert!(SWITCH_THUMB_SIZE.0 <= SWITCH_TRACK_HEIGHT.0 - 2. * SWITCH_TRACK_BORDER.0 + f32::EPSILON) };
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
    /// the accent pair for the highlight, and separators in the border
    /// colour, like the line under the filter.
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
                    separator: t.border,
                    radius: t.radius_md(),
                    item_radius: t.radius_sm(),
                }
            );
        }
        // max-h-[300px] at the 16px root size, and h-px.
        assert_eq!(COMMAND_LIST_MAX_HEIGHT, rems(18.75));
        assert_eq!(COMMAND_SEPARATOR_HEIGHT, px(1.));
    }

    /// Every row sits inside its group's `p-1` at the sides; only the
    /// group's first row has it above and only its last below, so groups
    /// are 2 x p-1 apart and the list's ends get one.
    #[test]
    fn command_rows_take_their_groups_padding() {
        let p = rems(0.25);
        let none = rems(0.);
        // (first in group, last in group, top, bottom)
        let cases = [
            (true, false, p, none),
            (false, false, none, none),
            (false, true, none, p),
            (true, true, p, p),
        ];
        for (first, last, top, bottom) in cases {
            let insets = command_row_insets(first, last);
            assert_eq!(insets, CommandRowInsets { top, bottom, x: p }, "first={first} last={last}");
        }
        assert_eq!(COMMAND_GROUP_PADDING, rems(0.25));
    }

    #[test]
    fn tables_cover_every_variant_and_size() {
        for dark in [false, true] {
            let rows: Vec<_> = variant_rows(dark).iter().map(|r| r.variant).collect();
            assert_eq!(rows, ButtonVariant::ALL);
        }
        let sizes: Vec<_> = SIZE_ROWS.iter().map(|r| r.0).collect();
        assert_eq!(sizes, ButtonSize::ALL);
    }

    /// (size, height = min width, px), in rems: shadcn's toggle sizes.
    const TOGGLE_SIZE_ROWS: [(ToggleSize, f32, f32); 3] =
        [(ToggleSize::Default, 2.25, 0.5), (ToggleSize::Sm, 2.0, 0.375), (ToggleSize::Lg, 2.5, 0.625)];

    #[test]
    fn toggle_sizes_are_shadcns_and_cover_every_size() {
        let t = Tokens::neutral_light();
        let sizes: Vec<_> = TOGGLE_SIZE_ROWS.iter().map(|r| r.0).collect();
        assert_eq!(sizes, ToggleSize::ALL);
        for (size, height, px) in TOGGLE_SIZE_ROWS {
            for variant in ToggleVariant::ALL {
                for pressed in [false, true] {
                    let s = toggle_style(variant, size, pressed, &t);
                    assert_eq!((s.height, s.min_width, s.padding_x), (rems(height), rems(height), rems(px)), "{size:?}");
                    assert_eq!((s.gap, s.icon_size, s.text_size, s.radius), (rems(0.5), rems(1.), rems(0.875), t.radius_md()));
                }
            }
        }
    }

    /// Unpressed, the default toggle is a transparent control in the
    /// foreground that hovers muted; the outline one has an input border and
    /// hovers in accent. Pressed, both are the accent pair, under the
    /// pointer too.
    #[test]
    fn toggle_colours_for_each_variant_and_state() {
        for t in [Tokens::neutral_light(), Tokens::neutral_dark()] {
            let s = |variant, pressed| toggle_style(variant, ToggleSize::Default, pressed, &t);

            let off = s(ToggleVariant::Default, false);
            assert_eq!((off.background, off.foreground, off.border), (None, t.foreground, None));
            assert_eq!((off.hover_background, off.hover_foreground), (t.muted, t.muted_foreground));
            assert_eq!(off.focus_border, None);

            let off = s(ToggleVariant::Outline, false);
            assert_eq!((off.background, off.foreground, off.border), (None, t.foreground, Some(t.input)));
            assert_eq!((off.hover_background, off.hover_foreground), (t.accent, t.accent_foreground));
            assert_eq!(off.focus_border, Some(t.ring));

            for variant in ToggleVariant::ALL {
                let on = s(variant, true);
                assert_eq!((on.background, on.foreground), (Some(t.accent), t.accent_foreground), "{variant:?}");
                assert_eq!((on.hover_background, on.hover_foreground), (t.accent, t.accent_foreground), "{variant:?}");
                assert_eq!(on.ring, focus_ring(&t));
                // Pressing doesn't change the frame.
                assert_eq!(on.border, s(variant, false).border);
            }
        }
    }

    /// The tooltip is drawn inverted: the foreground token behind the
    /// background token, its keycaps in a faint wash of the background.
    #[test]
    fn tooltip_colours_come_from_the_tokens_inverted() {
        for (t, kbd_opacity) in [(Tokens::neutral_light(), 0.2), (Tokens::neutral_dark(), 0.1)] {
            let s = tooltip_style(&t);
            assert_eq!((s.background, s.foreground), (t.foreground, t.background));
            assert_eq!((s.kbd_background, s.kbd_foreground), (t.background.opacity(kbd_opacity), t.background));
            assert_eq!((s.padding_x, s.padding_y, s.padding_right_with_shortcut), (rems(0.75), rems(0.375), rems(0.375)));
            assert_eq!((s.gap, s.text_size, s.radius), (rems(0.5), rems(0.75), t.radius_md()));
        }
        // A theme of its own: the style follows the tokens, not shadcn's
        // neutral values.
        let mut t = Tokens::neutral_light();
        t.foreground = hsla(0.6, 0.5, 0.2, 1.);
        t.background = hsla(0.1, 0.3, 0.95, 1.);
        let s = tooltip_style(&t);
        assert_eq!((s.background, s.foreground), (t.foreground, t.background));
    }

    /// A menu is a popover-coloured panel with an accent highlight, muted
    /// shortcuts and descriptions, and border-coloured separators.
    #[test]
    fn menu_colours_come_from_the_tokens() {
        for t in [Tokens::neutral_light(), Tokens::neutral_dark()] {
            let s = menu_style(&t);
            assert_eq!((s.background, s.foreground, s.border), (t.popover, t.popover_foreground, t.border));
            assert_eq!((s.highlight, s.highlight_foreground), (t.accent, t.accent_foreground));
            assert_eq!((s.muted, s.separator), (t.muted_foreground, t.border));
            assert_eq!((s.radius, s.item_radius), (t.radius_md(), t.radius_sm()));
            assert_eq!((s.text_size, s.description_size), (rems(0.875), rems(0.75)));
        }
        let mut t = Tokens::neutral_light();
        t.popover = hsla(0.6, 0.5, 0.2, 1.);
        t.accent = hsla(0.1, 0.3, 0.95, 1.);
        let s = menu_style(&t);
        assert_eq!((s.background, s.highlight), (t.popover, t.accent));
        assert_eq!(MENU_MIN_WIDTH, Rems(8.));
    }

    /// The spec's owner colours (`#1a1b26` on `#c0caf5`, line `#7aa2f7`).
    fn owner_colors() -> TabColors {
        TabColors {
            surface: Some(gpui::rgb(0x1a1b26).into()),
            text: Some(gpui::rgb(0xc0caf5).into()),
            line: Some(gpui::rgb(0x7aa2f7).into()),
        }
    }

    /// A trigger given owner colours is on its surface in its text colour
    /// whether active or not, in both variants: active at full opacity,
    /// inactive at 60% until hovered, then full.
    #[test]
    fn owner_colours_apply_to_active_and_inactive_triggers() {
        for t in [Tokens::neutral_light(), Tokens::neutral_dark()] {
            let own = owner_colors();
            let (surface, text) = (own.surface.unwrap(), own.text.unwrap());
            for variant in TabVariant::ALL {
                let on = tab_style(variant, true, own, &t);
                assert_eq!((on.background, on.foreground, on.hover_foreground), (Some(surface), text, text), "{variant:?}");
                let off = tab_style(variant, false, own, &t);
                assert_eq!(off.background, Some(surface), "{variant:?}");
                assert_eq!(off.foreground, text.opacity(0.6), "{variant:?}");
                assert_eq!(off.hover_foreground, text, "{variant:?}");
                assert_eq!(off.hover_background, None, "{variant:?}");
                assert_eq!(off.top_line, None, "{variant:?}");
            }
            // One colour alone: the others come from the tokens.
            let surface_only = TabColors { surface: own.surface, ..TabColors::default() };
            let s = tab_style(TabVariant::Line, false, surface_only, &t);
            let plain = tab_style(TabVariant::Line, false, TabColors::default(), &t);
            assert_eq!((s.background, s.foreground, s.hover_foreground), (Some(surface), plain.foreground, t.foreground));
            let text_only = TabColors { text: own.text, ..TabColors::default() };
            let s = tab_style(TabVariant::Default, true, text_only, &t);
            let plain = tab_style(TabVariant::Default, true, TabColors::default(), &t);
            assert_eq!((s.background, s.foreground), (plain.background, text));
        }
    }

    /// A list without owner colours renders as shadcn's: nothing in the
    /// style depends on `TabColors::default()`.
    #[test]
    fn no_owner_colours_leave_the_shadcn_look() {
        for t in [Tokens::neutral_light(), Tokens::neutral_dark()] {
            let inactive = if t.dark { t.muted_foreground } else { t.foreground.opacity(0.6) };
            for variant in TabVariant::ALL {
                let off = tab_style(variant, false, TabColors::default(), &t);
                assert_eq!((off.background, off.foreground, off.top_line), (None, inactive, None), "{variant:?}");
                assert_eq!((off.hover_background, off.hover_foreground), (None, t.foreground), "{variant:?}");
                let on = tab_style(variant, true, TabColors::default(), &t);
                assert_eq!(on.foreground, t.foreground, "{variant:?}");
                assert_eq!(on.top_line, (variant == TabVariant::Line).then_some(t.primary), "{variant:?}");
            }
        }
    }

    /// The active `Line` trigger's top line is the owner's `line` colour
    /// when given, else `primary`; other triggers draw no line.
    #[test]
    fn line_colour_overrides_primary_on_the_active_line_trigger() {
        for t in [Tokens::neutral_light(), Tokens::neutral_dark()] {
            let own = owner_colors();
            assert_eq!(tab_style(TabVariant::Line, true, own, &t).top_line, own.line);
            let no_line = TabColors { line: None, ..own };
            assert_eq!(tab_style(TabVariant::Line, true, no_line, &t).top_line, Some(t.primary));
            assert_eq!(tab_style(TabVariant::Line, false, own, &t).top_line, None);
            assert_eq!(tab_style(TabVariant::Default, true, own, &t).top_line, None);
            let s = tab_style(TabVariant::Line, true, TabColors::default(), &t);
            assert_eq!((s.top_line, s.background, s.foreground), (Some(t.primary), None, t.foreground));
            assert!(!s.shadow);
            assert_eq!(s.border, Some(transparent_black()));
        }
    }

    /// The pill: inactive text at 60% (muted in a dark set), the active
    /// trigger raised on the background with a shadow (`dark:bg-input/30
    /// dark:border-input`), and the ring's border colour when focused.
    #[test]
    fn default_pill_follows_shadcn_in_both_sets() {
        let t = Tokens::neutral_light();
        let off = tab_style(TabVariant::Default, false, TabColors::default(), &t);
        assert_eq!((off.background, off.foreground), (None, t.foreground.opacity(0.6)));
        let on = tab_style(TabVariant::Default, true, TabColors::default(), &t);
        assert_eq!((on.background, on.foreground, on.border), (Some(t.background), t.foreground, Some(transparent_black())));
        assert!(on.shadow && !off.shadow);
        let t = Tokens::neutral_dark();
        let off = tab_style(TabVariant::Default, false, TabColors::default(), &t);
        assert_eq!(off.foreground, t.muted_foreground);
        let on = tab_style(TabVariant::Default, true, TabColors::default(), &t);
        assert_eq!((on.background, on.border), (Some(t.input.opacity(0.3)), Some(t.input)));
        for variant in TabVariant::ALL {
            let s = tab_style(variant, true, TabColors::default(), &t);
            assert_eq!(s.ring, focus_ring(&t));
            assert_eq!(s.focus_border, Some(t.ring), "{variant:?}");
            assert_eq!((s.text_size, s.gap), (rems(0.875), rems(0.375)));
        }
    }

    #[test]
    fn tab_lists_are_shadcns_pill_and_line_() {
        let t = Tokens::neutral_light();
        let pill = tab_list_style(TabVariant::Default, &t);
        assert_eq!((pill.background, pill.height, pill.radius), (Some(t.muted), rems(2.25), t.radius_lg()));
        assert_eq!(pill.padding, Edges::all(px(3.)));
        let line = tab_list_style(TabVariant::Line, &t);
        assert_eq!((line.background, line.height, line.gap, line.radius), (None, rems(2.25), rems(0.25), rems(0.)));
        // The padding is room for the focus ring.
        assert_eq!(TAB_LIST_PADDING, crate::components::RING_WIDTH);
    }

    /// A trigger keeps its natural width between the limits, grows to the
    /// minimum and stops at the maximum; limits that cross keep the minimum,
    /// as CSS does, and never panic.
    #[test]
    fn tab_width_clamps_between_min_and_max() {
        let width = TabWidth::new(rems(5.), rems(15.));
        assert_eq!(width.clamp(rems(8.)), rems(8.));
        assert_eq!(width.clamp(rems(2.)), rems(5.));
        assert_eq!(width.clamp(rems(40.)), rems(15.));
        assert_eq!((width.clamp(rems(5.)), width.clamp(rems(15.))), (rems(5.), rems(15.)));
        let crossed = TabWidth::new(rems(10.), rems(4.));
        assert_eq!((crossed.min(), crossed.max()), (rems(10.), rems(10.)));
        for natural in [0., 4., 10., 30.] {
            assert_eq!(crossed.clamp(rems(natural)), rems(10.), "natural {natural}");
        }
    }
}
