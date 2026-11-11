//! The gallery: every component in every variant and size, light or dark,
//! with the dialog, popover, select and command cases of the spec.
//!
//! ```sh
//! cargo run -p brindle-ui --example gallery              # for a person
//! cargo run -p brindle-ui --example gallery -- --check   # drives the checks
//! ```
//!
//! `--check` runs the scenarios of the `ui-components` spec from inside the
//! app, the way Brindle's `--action` and `--dump-screen-after` hooks do:
//! key presses go through `Window::dispatch_keystroke` (gpui's own key
//! dispatch, bindings and all), and the state is read back from the
//! components' entities and focus handles. Key releases and pointer presses
//! have no such hook (`Window::dispatch_event` returns a type private to
//! gpui), so on Windows they are posted as `WM_KEYUP` and mouse messages to
//! the gallery's own window, at the bounds the components painted: messages
//! to one window, not input to the desktop (the rule Brindle's e2e probes
//! follow). Where a check is about what is drawn (the focus ring), the
//! gallery renders its own window with `PrintWindow`, also on Windows only.
//! Elsewhere those checks are skipped. It prints one `ok NAME`,
//! `FAIL NAME: why` or `SKIP NAME: why` line per check and exits non-zero
//! when any check failed or the app panicked.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use brindle_ui::{
    ActiveTheme, Button, ButtonSize, ButtonVariant, Card, CardColors, CardHeader, Checkbox, Command, CommandEvent, CommandGroup, CommandItem,
    ContextMenu, ContextMenuTarget, Dialog, Dismiss, DropdownMenu, FocusScope, FocusStops, Icon, Input, Kbd, MenuEntry,
    MenuEvent, MenuItem, Modality, Overlay, Popover, PopoverContent, Select, SelectEvent, SelectOption, Separator,
    Switch, TabActivate, TabContextMenu, TabList, TabMove, TabTrigger, TabWidth, TextInput, Toggle, ToggleSize,
    ToggleVariant, Tokens, Tooltip, TooltipFor, UiTheme,
};
use gpui::{
    App, Application, Bounds, ClickEvent, Context, DismissEvent, Div, Entity, EventEmitter, FocusHandle, Focusable, Hsla,
    MouseDownEvent, Pixels, Point, ScrollHandle, SharedString, Subscription, TitlebarOptions, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, rems, rgb, size,
};

/// Bounds the gallery records as components paint, by name, for the checks
/// (and the popovers' triggers).
type Records = Rc<RefCell<HashMap<String, Bounds<Pixels>>>>;

/// Click counts by control name.
type Counts = Rc<RefCell<HashMap<String, usize>>>;

/// What the command hosts (dialog and popover) confirmed, in order.
type Confirmed = Rc<RefCell<Vec<String>>>;

/// The icon-only button sizes, with their side in rems.
const ICON_SIZES: [ButtonSize; 3] = [ButtonSize::Icon, ButtonSize::IconSm, ButtonSize::IconXs];

/// The window's size: fixed, so the edge cases know where the edges are.
const WINDOW_SIZE: gpui::Size<Pixels> = size(px(1000.), px(700.));

fn main() {
    let check = std::env::args().skip(1).any(|arg| arg == "--check");
    if check {
        check::install_guards();
    }
    // A title of its own per check run, which is how the checks find the
    // window to post messages to.
    let title: SharedString =
        if check { format!("brindle-ui gallery --check {}", std::process::id()).into() } else { "brindle-ui gallery".into() };
    Application::new().run(move |cx: &mut App| {
        brindle_ui::init(cx);
        cx.bind_keys(brindle_ui::default_bindings());
        UiTheme::set(Tokens::neutral_light(), cx);
        let bounds = Bounds::centered(None, WINDOW_SIZE, cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions { title: Some(title.clone()), ..Default::default() }),
                    window_min_size: Some(WINDOW_SIZE),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| Gallery::new(window, cx)),
            )
            .expect("the gallery window opens");
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                // Not inside the close: the platform's window state is still
                // borrowed there (as in Brindle's main).
                cx.spawn(async |cx| cx.update(|cx| cx.quit()).ok()).detach();
            }
        })
        .detach();
        cx.activate(true);
        if check {
            cx.spawn(async move |cx| check::run(window, title, cx.clone()).await).detach();
        }
    });
}

/// The command list the gallery shows, inline and in its hosts. Items are
/// numbered across the groups: New Tab 0, Close Tab 1, Split Right 2, Split
/// Down 3, Attach 4 (disabled), Reload Config 5. A separator sits between
/// each two groups, as shadcn's demo puts `<CommandSeparator />` there.
fn command_groups() -> Vec<CommandGroup> {
    vec![
        CommandGroup::new(
            "Tabs",
            [CommandItem::new("New Tab").shortcut("ctrl-shift-t"), CommandItem::new("Close Tab").shortcut("ctrl-shift-w")],
        ),
        CommandGroup::new(
            "Panes",
            [
                CommandItem::new("Split Right").detail("a zone to the right"),
                CommandItem::new("Split Down").detail("a zone below"),
                CommandItem::new("Attach").detail("no detached session").disabled(true),
            ],
        )
        .separator(true),
        CommandGroup::unnamed(["Reload Config"]).separator(true),
    ]
}

/// The shells the gallery's select offers: `zsh` is disabled.
fn shells() -> Vec<SelectOption> {
    vec!["bash".into(), "fish".into(), "foot".into(), SelectOption::new("zsh").disabled(true), "nu".into()]
}

/// The context menu's entries: a tab's menu, as Brindle's (Move Tab Left
/// disabled, for the first tab). Enabled items are 0, 2 and 4.
fn context_entries() -> Vec<MenuEntry> {
    vec![
        MenuItem::new("Rename Tab").shortcut("ctrl-shift-r").into(),
        MenuItem::new("Move Tab Left").disabled(true).into(),
        MenuItem::new("Move Tab Right").into(),
        MenuEntry::Separator,
        MenuItem::new("Close Tab").shortcut("ctrl-shift-w").into(),
    ]
}

/// The dropdown's entries: a profile menu, with descriptions under some
/// labels and `zsh` disabled.
fn dropdown_entries() -> Vec<MenuEntry> {
    vec![
        MenuItem::new("bash").description("the default shell").shortcut("ctrl-alt-1").into(),
        MenuItem::new("fish").shortcut("ctrl-alt-2").into(),
        MenuItem::new("tmux").description("tmux control mode · main").shortcut("ctrl-alt-3").into(),
        MenuItem::new("zsh").description("not installed").disabled(true).into(),
        MenuEntry::Separator,
        MenuItem::new("Settings…").into(),
    ]
}

/// A menu taller than the window: forty profiles, the first nine with keys.
fn long_entries() -> Vec<MenuEntry> {
    (1..=40)
        .map(|n| {
            let item = MenuItem::new(format!("Profile {n}"));
            if n <= 9 { item.shortcut(format!("ctrl-alt-{n}")) } else { item }.into()
        })
        .collect()
}

/// A tab in one of the gallery's tab lists.
#[derive(Clone, Debug)]
pub struct DemoTab {
    /// Its trigger's element id, kept when it moves.
    pub id: usize,
    pub label: SharedString,
    disabled: bool,
    /// The surface and text colours it is drawn in, active or not, as a
    /// terminal's background and foreground.
    colors: Option<(u32, u32)>,
    /// A leading icon, as a tmux window's marker.
    icon: bool,
    /// A status dot in this colour, and a bell dot.
    status: Option<u32>,
    bell: bool,
}

impl DemoTab {
    fn new(id: usize, label: impl Into<SharedString>) -> Self {
        Self { id, label: label.into(), disabled: false, colors: None, icon: false, status: None, bell: false }
    }
}

/// One of the gallery's tab lists: its tabs and the active one.
#[derive(Clone, Debug)]
pub struct TabsDemo {
    pub tabs: Vec<DemoTab>,
    pub active: usize,
}

/// The tab lists as they start, by name:
/// - `default` (pill): Account, Password (active), Billing (disabled),
///   Settings;
/// - `plain`: Overview, Analytics (active), Reports, Notifications;
/// - `strip`: a tab strip whose tabs close, move and show a tooltip: bash
///   (active, the spec's `#c0caf5` on `#1a1b26`), a long nvim title
///   (Solarized light colours, inactive but on its own surface), tmux (a
///   leading icon, the tokens' colours) and a build with a status dot and
///   a bell;
/// - `crowded`: six sessions in a list too narrow for them.
fn initial_tabs() -> HashMap<&'static str, TabsDemo> {
    let labelled = |labels: &[&'static str]| -> Vec<DemoTab> {
        labels.iter().enumerate().map(|(id, label)| DemoTab::new(id, *label)).collect()
    };
    let mut pill = labelled(&["Account", "Password", "Billing", "Settings"]);
    pill[2].disabled = true;
    let plain = labelled(&["Overview", "Analytics", "Reports", "Notifications"]);
    let strip = vec![
        DemoTab { colors: Some((0x1a1b26, 0xc0caf5)), ..DemoTab::new(0, "bash") },
        DemoTab { colors: Some((0xfdf6e3, 0x657b83)), ..DemoTab::new(1, "nvim ~/src/brindle/ui/src/components/tabs.rs") },
        DemoTab { icon: true, ..DemoTab::new(2, "tmux: main") },
        DemoTab { status: Some(0xe0af68), bell: true, ..DemoTab::new(3, "cargo build") },
    ];
    let crowded = (0..6).map(|n| DemoTab::new(n, format!("Session {}", n + 1))).collect();
    HashMap::from([
        ("default", TabsDemo { tabs: pill, active: 1 }),
        ("plain", TabsDemo { tabs: plain, active: 1 }),
        ("strip", TabsDemo { tabs: strip, active: 0 }),
        ("crowded", TabsDemo { tabs: crowded, active: 0 }),
    ])
}

/// The strip and crowded lists' trigger limits: 5rem to 15rem.
fn demo_tab_width() -> TabWidth {
    TabWidth::new(rems(5.), rems(15.))
}

/// A 6px dot, as a tab's status and bell adornments.
/// The demo cards' size.
const CARD_DEMO_WIDTH: f32 = 240.;
const CARD_DEMO_HEIGHT: f32 = 120.;

/// The headerless card's content: a square background, painted to its
/// box's corners.
const CARD_SQUARE: u32 = 0xd946ef;

/// What the owner-coloured cards sit on.
const OWNER_CARD_SURROUND: u32 = 0x16161e;

/// A terminal's colours, as an app gives them to a card: tokyo-night's
/// background and foreground, a muted mix and the 12% border mix, on a
/// darker surround.
fn owner_card_colors() -> CardColors {
    CardColors {
        surface: Some(rgb(0x1a1b26).into()),
        text: Some(rgb(0xc0caf5).into()),
        muted: Some(rgb(0x737aa2).into()),
        border: Some(rgb(0x2e303f).into()),
        surround: Some(rgb(OWNER_CARD_SURROUND).into()),
    }
}

fn dot(color: Hsla) -> Div {
    div().flex_none().size(px(6.)).rounded_full().bg(color)
}

/// The label of entry `ix` of `entries`, for the menu log.
fn entry_label(entries: &[MenuEntry], ix: usize) -> String {
    entries.get(ix).map_or_else(|| format!("#{ix}"), |entry| entry.label().to_string())
}

/// A `div` around `element` that records the element's painted bounds under
/// `name`, in window coordinates.
fn recorded(element: impl IntoElement, name: &str, records: &Records) -> Div {
    let (name, records) = (name.to_string(), records.clone());
    div().child(element).on_children_prepainted(move |bounds, _, _| {
        if let Some(first) = bounds.first() {
            records.borrow_mut().insert(name.clone(), *first);
        }
    })
}

/// A section heading.
fn heading(text: &'static str, cx: &App) -> Div {
    div().text_size(px(12.)).font_weight(gpui::FontWeight::MEDIUM).text_color(cx.ui_theme().muted_foreground).child(text)
}

/// A label beside a control.
fn label(text: impl Into<SharedString>, cx: &App) -> Div {
    div().w(px(96.)).flex_none().text_color(cx.ui_theme().muted_foreground).child(text.into())
}

// ---- the gallery ---------------------------------------------------------------

/// The window's root view: every component, and the slots of its overlays.
/// Its root is a focus scope over every enabled control ([`FocusStops`]),
/// so Tab and shift-Tab move through them as on a web page.
pub struct Gallery {
    focus: FocusHandle,
    dark: bool,
    handles: HashMap<String, FocusHandle>,
    clicks: Counts,
    records: Records,
    checked: bool,
    switched: bool,
    /// Each toggle's pressed state, by name (unpressed when missing).
    pressed: HashMap<String, bool>,
    name: Entity<TextInput>,
    email: Entity<TextInput>,
    shell: Entity<Select>,
    /// Every `SelectEvent::Changed` from `shell`, in order.
    shell_changes: Vec<usize>,
    editor: Entity<Select>,
    off: Entity<Select>,
    command: Entity<Command>,
    /// What the inline command list reported, in order.
    command_log: Vec<String>,
    dialog: Overlay<DemoDialog>,
    /// A dialog whose slot ignores presses on the backdrop.
    sticky_dialog: Overlay<DemoDialog>,
    command_dialog: Overlay<CommandHost>,
    confirmed: Confirmed,
    context_menu: Entity<ContextMenu>,
    dropdown: Entity<DropdownMenu>,
    long_dropdown: Entity<DropdownMenu>,
    /// What the menus chose, in order, as `menu:label`.
    menu_log: Vec<String>,
    /// The tab lists' tabs and active indices, by list name.
    tabs: HashMap<&'static str, TabsDemo>,
    /// What the tab lists asked for, in order: `list:activate:N:pointer`,
    /// `list:close:N`, `list:move:A->B`, `list:menu:N`.
    tab_log: Vec<String>,
    /// Each corner popover's panel bounds while it is open.
    corners: [Rc<Cell<Option<Bounds<Pixels>>>>; 4],
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl Gallery {
    /// The names of the focusable controls the gallery owns handles for.
    fn handle_names() -> Vec<String> {
        let mut names: Vec<String> = ButtonVariant::ALL
            .iter()
            .flat_map(|variant| ButtonSize::ALL.iter().map(move |size| Self::button_name(*variant, *size)))
            .collect();
        names.extend(
            [
                "disabled", "icon-label", "behind", "open-dialog", "open-stubborn", "open-sticky", "open-command-dialog",
                "command-popover", "corner-0", "corner-1", "corner-2", "corner-3", "checkbox", "checkbox-off", "switch",
                "switch-off", "theme", "toggle", "toggle-off", "dropdown-trigger", "long-dropdown-trigger", "tabs-default",
                "tabs-plain", "tabs-strip",
            ]
            .map(String::from),
        );
        names
    }

    fn button_name(variant: ButtonVariant, size: ButtonSize) -> String {
        format!("button:{variant:?}:{size:?}")
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let handles: HashMap<String, FocusHandle> =
            Self::handle_names().into_iter().map(|name| (name, cx.focus_handle())).collect();
        let context_menu = cx.new(|_| ContextMenu::new());
        // Triggers with a focus handle, for the keys, and no `on_click`:
        // the dropdown takes the press and the keys itself.
        let trigger = |id: &'static str, text: &'static str, handle: FocusHandle| {
            move |_open: bool, _: &mut Window, _: &mut App| {
                Button::new(id, text).variant(ButtonVariant::Outline).icon(Icon::ChevronDown).focus(&handle)
            }
        };
        let dropdown = cx.new(|_| {
            DropdownMenu::new(trigger("dropdown-trigger", "Profiles", handles["dropdown-trigger"].clone()), dropdown_entries())
        });
        let long_dropdown = cx.new(|_| {
            DropdownMenu::new(
                trigger("long-dropdown-trigger", "Forty profiles", handles["long-dropdown-trigger"].clone()),
                long_entries(),
            )
        });
        let name = cx.new(|cx| TextInput::new("", cx).with_placeholder("Name"));
        let email = cx.new(|cx| TextInput::new("not an address", cx).with_placeholder("Email"));
        let shell = cx.new(|cx| Select::new(shells(), Some(0), cx));
        let editor = cx.new(|cx| Select::new(["vim", "emacs", "nano"], None, cx).with_placeholder("Pick an editor…"));
        let off = cx.new(|cx| Select::new(["unavailable"], Some(0), cx));
        off.update(cx, |select, cx| select.set_disabled(true, window, cx));
        let command = cx.new(|cx| Command::new(command_groups(), cx));
        // Focus starts on the gallery itself, inside its focus scope, so the
        // first Tab reaches the first control.
        let focus = cx.focus_handle();
        window.focus(&focus);
        let subscriptions = vec![
            cx.subscribe(&shell, |this, _, event: &SelectEvent, cx| {
                let SelectEvent::Changed(ix) = *event;
                this.shell_changes.push(ix);
                cx.notify();
            }),
            cx.subscribe(&command, |this, command, event: &CommandEvent, cx| {
                let entry = match event {
                    CommandEvent::Confirmed(ix) => {
                        let label = command.read(cx).item(*ix).map(|item| item.label.to_string()).unwrap_or_default();
                        format!("confirmed:{label}")
                    }
                    CommandEvent::Cancelled => "cancelled".to_string(),
                };
                this.command_log.push(entry);
                cx.notify();
            }),
            cx.subscribe(&context_menu, |this, _, event: &MenuEvent, cx| {
                let MenuEvent::Selected(ix) = *event;
                this.menu_log.push(format!("context:{}", entry_label(&context_entries(), ix)));
                cx.notify();
            }),
            cx.subscribe(&dropdown, |this, dropdown, event: &MenuEvent, cx| {
                let MenuEvent::Selected(ix) = *event;
                let label = entry_label(dropdown.read(cx).entries(), ix);
                this.menu_log.push(format!("dropdown:{label}"));
                cx.notify();
            }),
            cx.subscribe(&long_dropdown, |this, dropdown, event: &MenuEvent, cx| {
                let MenuEvent::Selected(ix) = *event;
                let label = entry_label(dropdown.read(cx).entries(), ix);
                this.menu_log.push(format!("long:{label}"));
                cx.notify();
            }),
        ];
        Self {
            focus,
            dark: false,
            handles,
            clicks: Counts::default(),
            records: Records::default(),
            checked: false,
            switched: true,
            pressed: HashMap::new(),
            name,
            email,
            shell,
            shell_changes: Vec::new(),
            editor,
            off,
            command,
            command_log: Vec::new(),
            dialog: Overlay::modal(),
            sticky_dialog: Overlay::modal().ignore_outside_press(),
            command_dialog: Overlay::modal(),
            confirmed: Confirmed::default(),
            context_menu,
            dropdown,
            long_dropdown,
            menu_log: Vec::new(),
            tabs: initial_tabs(),
            tab_log: Vec::new(),
            corners: Default::default(),
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    /// The focus handle of the control called `name`.
    pub fn handle(&self, name: &str) -> FocusHandle {
        self.handles.get(name).unwrap_or_else(|| panic!("no control named {name:?}")).clone()
    }

    pub fn clicks(&self, name: &str) -> usize {
        self.clicks.borrow().get(name).copied().unwrap_or(0)
    }

    pub fn recorded(&self, name: &str) -> Option<Bounds<Pixels>> {
        self.records.borrow().get(name).copied()
    }

    /// A click handler that counts clicks on `name`.
    fn count(&self, name: &str) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        let (name, clicks) = (name.to_string(), self.clicks.clone());
        move |_, _, _| *clicks.borrow_mut().entry(name.clone()).or_default() += 1
    }

    /// A toggle handler that counts the requests made of `name`.
    fn count_toggle(&self, name: &str) -> impl Fn(&bool, &mut Window, &mut App) + 'static {
        let (name, clicks) = (name.to_string(), self.clicks.clone());
        move |_, _, _| *clicks.borrow_mut().entry(name.clone()).or_default() += 1
    }

    pub fn set_dark(&mut self, dark: bool, cx: &mut Context<Self>) {
        self.dark = dark;
        UiTheme::set(if dark { Tokens::neutral_dark() } else { Tokens::neutral_light() }, cx);
        cx.notify();
    }

    /// Opens the demo dialog: the plain one, or the one that refuses to be
    /// dismissed until its changes are discarded.
    pub fn open_dialog(&mut self, stubborn: bool, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.new(|cx| DemoDialog::new(stubborn, cx));
        self.dialog.open(view, |this| &mut this.dialog, window, cx);
    }

    /// Opens the plain demo dialog in the slot that ignores backdrop presses.
    pub fn open_sticky_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.new(|cx| DemoDialog::new(false, cx));
        self.sticky_dialog.open(view, |this| &mut this.sticky_dialog, window, cx);
    }

    pub fn open_command_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let confirmed = self.confirmed.clone();
        let view = cx.new(|cx| CommandHost::new(Chrome::Dialog, confirmed, cx));
        self.command_dialog.open(view, |this| &mut this.command_dialog, window, cx);
    }

    /// Closes what the gallery owns a slot for (the checks' reset).
    pub fn close_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dialog.close(window, cx);
        self.sticky_dialog.close(window, cx);
        self.command_dialog.close(window, cx);
        self.shell.update(cx, |select, cx| select.close(window, cx));
        self.editor.update(cx, |select, cx| select.close(window, cx));
        self.context_menu.update(cx, |menu, cx| menu.close(window, cx));
        self.dropdown.update(cx, |menu, cx| menu.close(window, cx));
        self.long_dropdown.update(cx, |menu, cx| menu.close(window, cx));
        cx.notify();
    }

    /// Opens the context menu with its corner at `at`, as a key or code
    /// would (no pointer gesture).
    pub fn open_context_menu(&mut self, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        self.context_menu.update(cx, |menu, cx| menu.open(at, context_entries(), window, cx));
    }

    /// The context menu, both dropdowns and what they chose.
    fn menus_section(&self, cx: &mut Context<Self>) -> Div {
        let t = *cx.ui_theme();
        let target = div()
            .id("context-target")
            .flex()
            .items_center()
            .justify_center()
            .w(px(240.))
            .h(px(64.))
            .rounded(t.radius_md())
            .border_1()
            .border_dashed()
            .border_color(t.border)
            .text_color(t.muted_foreground)
            .child("Right-click here")
            .on_context_menu(cx.listener(|this, event: &MouseDownEvent, window, cx| {
                this.context_menu.update(cx, |menu, cx| menu.open_on_press(event, context_entries(), window, cx));
            }));
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(heading("ContextMenu and DropdownMenu", cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(recorded(target, "context-target", &self.records))
                    .child(recorded(self.dropdown.clone(), "dropdown", &self.records))
                    .child(recorded(self.long_dropdown.clone(), "long-dropdown", &self.records)),
            )
    }

    fn header(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .items_center()
            .justify_between()
            .child(div().text_size(px(20.)).font_weight(gpui::FontWeight::SEMIBOLD).child("brindle-ui gallery"))
            .child(
                div().flex().items_center().gap_2().child(label("Dark", cx)).child(recorded(
                    Switch::new("theme", self.dark)
                        .focus(&self.handle("theme"))
                        .on_click(cx.listener(|this, dark: &bool, _, cx| this.set_dark(*dark, cx))),
                    "theme",
                    &self.records,
                )),
            )
    }

    fn overlays_section(&mut self, cx: &mut Context<Self>) -> Div {
        let confirmed = self.confirmed.clone();
        let trigger_handle = self.handle("command-popover");
        let count = self.count("command-popover");
        let command_popover = Popover::new(
            "command-popover",
            move |trigger| {
                let toggle = trigger.on_click();
                Button::new("command-popover-trigger", "Command popover")
                    .variant(ButtonVariant::Outline)
                    .focus(&trigger_handle)
                    .on_click(move |event, window, cx| {
                        count(event, window, cx);
                        toggle(event, window, cx);
                    })
            },
            move |_, cx| cx.new(|cx| CommandHost::new(Chrome::Popover, confirmed.clone(), cx)),
        );
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(heading("Overlays", cx))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(recorded(
                        Button::new("open-dialog", "Open dialog")
                            .focus(&self.handle("open-dialog"))
                            .on_click(cx.listener(|this, _, window, cx| this.open_dialog(false, window, cx))),
                        "open-dialog",
                        &self.records,
                    ))
                    .child(
                        Button::new("open-stubborn", "Open stubborn dialog")
                            .variant(ButtonVariant::Secondary)
                            .focus(&self.handle("open-stubborn"))
                            .on_click(cx.listener(|this, _, window, cx| this.open_dialog(true, window, cx))),
                    )
                    .child(
                        Button::new("open-sticky", "Open sticky dialog")
                            .variant(ButtonVariant::Secondary)
                            .focus(&self.handle("open-sticky"))
                            .on_click(cx.listener(|this, _, window, cx| this.open_sticky_dialog(window, cx))),
                    )
                    .child(
                        Button::new("open-command-dialog", "Command dialog")
                            .variant(ButtonVariant::Secondary)
                            .focus(&self.handle("open-command-dialog"))
                            .on_click(cx.listener(|this, _, window, cx| this.open_command_dialog(window, cx))),
                    )
                    .child(recorded(command_popover, "command-popover", &self.records))
                    .child(recorded(
                        Button::new("behind", "Behind the dialog")
                            .variant(ButtonVariant::Ghost)
                            .focus(&self.handle("behind"))
                            .on_click(self.count("behind")),
                        "behind",
                        &self.records,
                    )),
            )
    }

    fn selects_section(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(heading("Select", cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().w(px(220.)).child(recorded(self.shell.clone(), "shell", &self.records)))
                    .child(div().w(px(220.)).child(self.editor.clone()))
                    .child(div().w(px(160.)).child(self.off.clone())),
            )
    }

    fn command_section(&self, cx: &mut Context<Self>) -> Div {
        let t = cx.ui_theme();
        div().flex().flex_col().gap_3().child(heading("Command (inline)", cx)).child(recorded(
            div()
                .w(px(420.))
                .rounded(t.radius_md())
                .border_1()
                .border_color(t.border)
                .shadow_sm()
                .child(self.command.clone()),
            "command",
            &self.records,
        ))
    }

    fn buttons_section(&self, cx: &mut Context<Self>) -> Div {
        let mut section = div().flex().flex_col().gap_3().child(heading("Button", cx));
        for variant in ButtonVariant::ALL {
            let mut row = div().flex().items_center().gap_3().child(label(format!("{variant:?}"), cx));
            for size in ButtonSize::ALL {
                let name = Self::button_name(variant, size);
                let id: SharedString = name.clone().into();
                let button = match size {
                    ButtonSize::Icon | ButtonSize::IconSm | ButtonSize::IconXs => Button::icon_only(id, Icon::Check),
                    _ => Button::new(id, format!("{size:?}")),
                };
                row = row.child(button.variant(variant).size(size).focus(&self.handle(&name)).on_click(self.count(&name)));
            }
            section = section.child(row);
        }
        section.child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(label("States", cx))
                .child(recorded(
                    Button::new("disabled", "Disabled")
                        .disabled(true)
                        .focus(&self.handle("disabled"))
                        .on_click(self.count("disabled")),
                    "disabled",
                    &self.records,
                ))
                .child(
                    Button::new("icon-label", "Save")
                        .icon(Icon::Check)
                        .variant(ButtonVariant::Outline)
                        .focus(&self.handle("icon-label"))
                        .on_click(self.count("icon-label")),
                )
                .child(Button::icon_only("icon-x", Icon::X).variant(ButtonVariant::Ghost))
                .child(Button::new("link", "Learn more").variant(ButtonVariant::Link)),
        )
    }

    /// The icon sizes, every icon, `Separator` and `Kbd`.
    fn chrome_section(&self, cx: &mut Context<Self>) -> Div {
        let t = *cx.ui_theme();
        let row = |name: &'static str| div().flex().items_center().gap_3().child(label(name, cx));
        let mut sizes = row("Icon sizes");
        for size in ICON_SIZES {
            let name = format!("size:{size:?}");
            sizes = sizes
                .child(recorded(
                    Button::icon_only(SharedString::from(name.clone()), Icon::Plus)
                        .variant(ButtonVariant::Outline)
                        .size(size),
                    &name,
                    &self.records,
                ))
                .child(
                    Button::icon_only(SharedString::from(format!("size-ghost:{size:?}")), Icon::ArrowDown)
                        .variant(ButtonVariant::Ghost)
                        .size(size),
                );
        }
        let mut icons = row("Icons");
        for icon in Icon::ALL {
            icons = icons.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .child(icon.size(gpui::rems(1.5)))
                    .child(div().text_size(px(10.)).text_color(t.muted_foreground).child(format!("{icon:?}"))),
            );
        }
        let separators = row("Separator")
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .w(px(160.))
                    .child("Above")
                    .child(Separator::horizontal())
                    .child("Below"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .h(px(20.))
                    .child("Docs")
                    .child(Separator::vertical())
                    .child("Source")
                    .child(Separator::vertical())
                    .child("Releases"),
            );
        let keys = row("Kbd")
            .child(Kbd::new("enter"))
            .child(Kbd::new("ctrl-shift-t"))
            .child(Kbd::new("ctrl-k ctrl-c"))
            .child(Kbd::new("alt-1"))
            .child(Kbd::new("ctrl--"));
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(heading("Icon buttons, icons, Separator and Kbd", cx))
            .child(sizes)
            .child(icons)
            .child(separators)
            .child(keys)
    }

    fn inputs_section(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(heading("Input", cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(label("Name", cx))
                    .child(div().w(px(260.)).child(recorded(Input::new(&self.name), "name-input", &self.records))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(label("Invalid", cx))
                    .child(div().w(px(260.)).child(Input::new(&self.email).invalid(true))),
            )
    }

    fn toggles_section(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(heading("Checkbox and Switch", cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .child(label("Checkbox", cx))
                    .child(recorded(
                        Checkbox::new("checkbox", self.checked)
                            .focus(&self.handle("checkbox"))
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.checked = *checked;
                                cx.notify();
                            })),
                        "checkbox",
                        &self.records,
                    ))
                    .child(recorded(
                        Checkbox::new("checkbox-off", true)
                            .disabled(true)
                            .focus(&self.handle("checkbox-off"))
                            .on_click(self.count_toggle("checkbox-off")),
                        "checkbox-off",
                        &self.records,
                    ))
                    .child(Checkbox::new("checkbox-off-unchecked", false).disabled(true)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .child(label("Switch", cx))
                    .child(
                        Switch::new("switch", self.switched)
                            .focus(&self.handle("switch"))
                            .on_click(cx.listener(|this, on: &bool, _, cx| {
                                this.switched = *on;
                                cx.notify();
                            })),
                    )
                    .child(Switch::new("switch-off", true).disabled(true).focus(&self.handle("switch-off")))
                    .child(Switch::new("switch-off-unchecked", false).disabled(true)),
            )
    }

    /// Whether the toggle called `name` is pressed.
    pub fn is_pressed(&self, name: &str) -> bool {
        self.pressed.get(name).copied().unwrap_or(false)
    }

    /// A toggle handler that stores the state asked for under `name` and
    /// counts the request.
    fn press_toggle(&self, name: &str, cx: &mut Context<Self>) -> impl Fn(&bool, &mut Window, &mut App) + 'static {
        let (name, clicks) = (name.to_string(), self.clicks.clone());
        cx.listener(move |this, pressed: &bool, _, cx| {
            *clicks.borrow_mut().entry(name.clone()).or_default() += 1;
            this.pressed.insert(name.clone(), *pressed);
            cx.notify();
        })
    }

    /// Every toggle variant and size, the states, and buttons and a plain
    /// element with tooltips (shown after a 500 ms hover), plus the
    /// tooltip's look drawn in place.
    fn toggle_tooltip_section(&self, cx: &mut Context<Self>) -> Div {
        let mut section = div().flex().flex_col().gap_3().child(heading("Toggle and Tooltip", cx));
        for variant in ToggleVariant::ALL {
            let mut row = div().flex().items_center().gap_3().child(label(format!("{variant:?}"), cx));
            for size in ToggleSize::ALL {
                // No focus handles here: a press flips them and moves focus
                // nowhere.
                let name = format!("toggle:{variant:?}:{size:?}");
                row = row
                    .child(recorded(
                        Toggle::new(SharedString::from(name.clone()), format!("{size:?}"))
                            .variant(variant)
                            .size(size)
                            .pressed(self.is_pressed(&name))
                            .on_click(self.press_toggle(&name, cx)),
                        &name,
                        &self.records,
                    ))
                    .child({
                        let name = format!("toggle-icon:{variant:?}:{size:?}");
                        Toggle::icon_only(SharedString::from(name.clone()), Icon::Check)
                            .variant(variant)
                            .size(size)
                            .pressed(self.is_pressed(&name))
                            .on_click(self.press_toggle(&name, cx))
                    });
            }
            section = section.child(row);
        }
        let states = div()
            .flex()
            .items_center()
            .gap_3()
            .child(label("States", cx))
            .child(recorded(
                Toggle::new("toggle", "Aa")
                    .size(ToggleSize::Sm)
                    .pressed(self.is_pressed("toggle"))
                    .focus(&self.handle("toggle"))
                    .on_click(self.press_toggle("toggle", cx))
                    .tooltip_for("Match case", Some("alt-c".into())),
                "toggle",
                &self.records,
            ))
            .child(recorded(
                Toggle::new("toggle-off", "Off")
                    .disabled(true)
                    .focus(&self.handle("toggle-off"))
                    .on_click(self.count_toggle("toggle-off"))
                    .tooltip_for("Never shown", None),
                "toggle-off",
                &self.records,
            ))
            .child(Toggle::new("toggle-off-pressed", "On").pressed(true).disabled(true))
            .child(Toggle::new("toggle-icon-label", "Bold").icon(Icon::Check).variant(ToggleVariant::Outline));
        let tooltips = div()
            .flex()
            .items_center()
            .gap_3()
            .child(label("Tooltips", cx))
            .child(recorded(
                // As a tab strip's new-tab button: a legible stroke.
                Button::icon_only("tip-new", Icon::Plus)
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::IconSm)
                    .icon_min_stroke(brindle_ui::LEGIBLE_ICON_STROKE)
                    .on_click(self.count("tip-new"))
                    .tooltip_for("New tab", Some("ctrl-shift-t".into())),
                "tip-new",
                &self.records,
            ))
            .child(recorded(
                // The text is worked out when the tooltip shows.
                Button::icon_only("tip-find", Icon::Search)
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::IconSm)
                    .tooltip_with(|_, _| Tooltip::new("Find").shortcut("ctrl-shift-f")),
                "tip-find",
                &self.records,
            ))
            .child(Button::new("tip-save", "Save").variant(ButtonVariant::Outline).tooltip_for("Save the profile", None))
            .child(recorded(
                Button::new("tip-disabled", "Disabled")
                    .variant(ButtonVariant::Outline)
                    .disabled(true)
                    .tooltip_for("Never shown", None),
                "tip-disabled",
                &self.records,
            ))
            .child(
                div()
                    .id("tip-text")
                    .text_color(cx.ui_theme().muted_foreground)
                    .child("Hover me (a plain element)")
                    .tooltip_for("Any stateful element", None),
            );
        let look = div()
            .flex()
            .items_center()
            .gap_3()
            .child(label("Tooltip look", cx))
            .child(Tooltip::new("New tab").shortcut("ctrl-shift-t"))
            .child(Tooltip::new("Close tab"))
            .child(Tooltip::new("Split").shortcut("ctrl-k ctrl-v"));
        section.child(states).child(tooltips).child(look)
    }

    /// The tab list called `name`, as it is now.
    pub fn tabs_demo(&self, name: &str) -> &TabsDemo {
        self.tabs.get(name).unwrap_or_else(|| panic!("no tab list named {name:?}"))
    }

    /// Makes the trigger at `ix` of `name` the active one, as the checks'
    /// preconditions do.
    pub fn set_tab_active(&mut self, name: &str, ix: usize, cx: &mut Context<Self>) {
        if let Some(demo) = self.tabs.get_mut(name) {
            demo.active = ix;
        }
        cx.notify();
    }

    /// Puts every tab list back as it started (the checks' reset).
    pub fn reset_tabs(&mut self, cx: &mut Context<Self>) {
        self.tabs = initial_tabs();
        cx.notify();
    }

    fn activate_tab(&mut self, name: &str, request: &TabActivate, cx: &mut Context<Self>) {
        let by = match request.by {
            Modality::Pointer => "pointer",
            Modality::Keyboard => "keyboard",
        };
        self.tab_log.push(format!("{name}:activate:{}:{by}", request.index));
        self.set_tab_active(name, request.index, cx);
    }

    /// Closes the tab at `ix`; the active tab stays, or its right-hand
    /// neighbour takes over (the left one at the end).
    fn close_tab(&mut self, name: &str, ix: usize, cx: &mut Context<Self>) {
        self.tab_log.push(format!("{name}:close:{ix}"));
        if let Some(demo) = self.tabs.get_mut(name)
            && ix < demo.tabs.len()
        {
            demo.tabs.remove(ix);
            if ix < demo.active || demo.active >= demo.tabs.len() {
                demo.active = demo.active.saturating_sub(1);
            }
        }
        cx.notify();
    }

    /// Moves the tab at `from` to `to`'s place; the active tab stays active.
    fn move_tab(&mut self, name: &str, request: &TabMove, cx: &mut Context<Self>) {
        let TabMove { from, to } = *request;
        self.tab_log.push(format!("{name}:move:{from}->{to}"));
        if let Some(demo) = self.tabs.get_mut(name)
            && from < demo.tabs.len()
        {
            let active = demo.tabs.get(demo.active).map(|tab| tab.id);
            let tab = demo.tabs.remove(from);
            demo.tabs.insert(to.min(demo.tabs.len()), tab);
            demo.active = demo.tabs.iter().position(|tab| Some(tab.id) == active).unwrap_or(0);
        }
        cx.notify();
    }

    /// The tab list called `name`. The strip and crowded lists' tabs close
    /// (with a tooltip on the button), move, show their title as a tooltip
    /// and keep between 5 and 15rem. Each trigger's bounds are recorded as
    /// `tab:NAME:IX`.
    fn tab_list(&self, name: &'static str, cx: &mut Context<Self>) -> TabList {
        let demo = self.tabs_demo(name);
        let closable = matches!(name, "strip" | "crowded");
        let primary = cx.ui_theme().primary;
        let triggers: Vec<TabTrigger> = demo
            .tabs
            .iter()
            .map(|tab| {
                let mut trigger = TabTrigger::new(("tab", tab.id), tab.label.clone()).disabled(tab.disabled);
                if closable {
                    trigger = trigger
                        .closable(true)
                        .tooltip_for(tab.label.clone(), None)
                        .close_tooltip(|_, _| Tooltip::new("Close tab").shortcut("ctrl-shift-w"));
                }
                if let Some((surface, text)) = tab.colors {
                    trigger = trigger.surface(rgb(surface)).text_color(rgb(text));
                }
                if tab.icon {
                    trigger = trigger.leading(Icon::AppWindow.size(rems(0.875)));
                }
                if let Some(status) = tab.status {
                    trigger = trigger.adornment(dot(rgb(status).into()));
                }
                if tab.bell {
                    trigger = trigger.adornment(dot(primary));
                }
                trigger
            })
            .collect();
        let (records, prefix) = (self.records.clone(), format!("tab:{name}:"));
        let list = TabList::new(SharedString::from(format!("tabs-{name}")))
            .active(demo.active)
            .children(triggers)
            .on_activate(cx.listener(move |this, request: &TabActivate, _, cx| this.activate_tab(name, request, cx)))
            .on_trigger_bounds(move |bounds, _, _| {
                let mut records = records.borrow_mut();
                records.retain(|key, _| !key.starts_with(&prefix));
                for (ix, bounds) in bounds.iter().enumerate() {
                    records.insert(format!("{prefix}{ix}"), *bounds);
                }
            });
        if !closable {
            return list;
        }
        list.trigger_width(demo_tab_width())
            .on_close(cx.listener(move |this, ix: &usize, _, cx| this.close_tab(name, *ix, cx)))
            .on_move(cx.listener(move |this, request: &TabMove, _, cx| this.move_tab(name, request, cx)))
            .on_context_menu(cx.listener(move |this, request: &TabContextMenu, window, cx| {
                this.tab_menu(name, request, window, cx)
            }))
    }

    /// A secondary press on a tab of `name`: the tab menu opens at the
    /// pointer, in the shared context menu (the gallery's demo menu is a tab
    /// menu already). The tab isn't activated, as a strip may choose to.
    fn tab_menu(&mut self, name: &str, request: &TabContextMenu, window: &mut Window, cx: &mut Context<Self>) {
        self.tab_log.push(format!("{name}:menu:{}", request.index));
        self.context_menu.update(cx, |menu, cx| menu.open_on_press(&request.event, context_entries(), window, cx));
        cx.notify();
    }

    /// The tab lists, all shadcn's pill: one with a disabled trigger, a
    /// plain one, the strip above content in the active tab's colours, and
    /// a crowded one whose triggers shrink to their minimum.
    fn tabs_section(&self, cx: &mut Context<Self>) -> Div {
        let t = *cx.ui_theme();
        let demo = self.tabs_demo("strip");
        let current = demo.tabs.get(demo.active);
        let (surface, text): (Hsla, Hsla) = current
            .and_then(|tab| tab.colors)
            .map_or((t.background, t.foreground), |(surface, text)| (rgb(surface).into(), rgb(text).into()));
        let content = current.map_or_else(|| "No tabs left".to_string(), |tab| format!("The content of {}", tab.label));
        let pill = self.tab_list("default", cx).focus(&self.handle("tabs-default"));
        let plain = self.tab_list("plain", cx).focus(&self.handle("tabs-plain"));
        let strip = self.tab_list("strip", cx).focus(&self.handle("tabs-strip"));
        let crowded = self.tab_list("crowded", cx);
        let row = |name: &'static str| div().flex().items_center().gap_3().child(label(name, cx));
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(heading("Tabs", cx))
            .child(row("Default").child(recorded(pill, "tabs:default", &self.records)))
            .child(row("Plain").child(recorded(plain, "tabs:plain", &self.records)))
            .child(
                div().flex().items_start().gap_3().child(label("Strip", cx)).child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .rounded(t.radius_md())
                        .border_1()
                        .border_color(t.border)
                        .overflow_hidden()
                        .child(strip)
                        .child(div().h(px(56.)).px_3().py_2().bg(surface).text_color(text).child(content)),
                ),
            )
            .child(
                row("Crowded").child(
                    div()
                        .w(px(320.))
                        .rounded(t.radius_md())
                        .border_1()
                        .border_color(t.border)
                        .overflow_hidden()
                        .child(crowded),
                ),
            )
    }

    /// Cards: the default look with a header; the owner's colours (a
    /// terminal's) on a surround of their own; a headerless card whose
    /// content paints a square background to its corners; and the same
    /// card without and with the ring.
    fn cards_section(&self, cx: &mut Context<Self>) -> Div {
        let t = *cx.ui_theme();
        let records = &self.records;
        let owner = owner_card_colors();
        let actions = |card: &'static str, muted: Hsla, header: CardHeader| {
            [("split-right", Icon::Columns2), ("split-down", Icon::Rows2), ("close", Icon::X)].into_iter().fold(
                header,
                |header, (name, icon)| {
                    let id = SharedString::from(format!("card-{card}-{name}"));
                    header.action(
                        Button::icon_only(id, icon).variant(ButtonVariant::Ghost).size(ButtonSize::IconXs).text_color(muted),
                    )
                },
            )
        };
        let sized = |card: Card| card.w(px(CARD_DEMO_WIDTH)).h(px(CARD_DEMO_HEIGHT));
        let header_record = |name: &'static str| {
            let records = records.clone();
            move |bounds: Bounds<Pixels>, _: &mut Window, _: &mut App| {
                records.borrow_mut().insert(name.to_string(), bounds);
            }
        };
        let body = |text: &'static str| div().size_full().px_2().py_1().text_size(px(12.)).child(text);

        let default = sized(Card::new("card-default"))
            .header(actions(
                "default",
                t.muted_foreground,
                CardHeader::new("card-default-header").leading(dot(rgb(0x22c55e).into())).title("A long card title cut by an ellipsis"),
            ))
            .on_header_bounds(header_record("card:default:header"))
            .child(body("The default tokens"));
        let owned = sized(Card::new("card-owner"))
            .colors(owner)
            .header(actions(
                "owner",
                owner.muted.unwrap_or(t.muted_foreground),
                CardHeader::new("card-owner-header").leading(dot(rgb(0xe0af68).into())).title("nvim"),
            ))
            .on_header_bounds(header_record("card:owner:header"))
            .child(body("~ $ ls").text_color(owner.text.unwrap_or(t.foreground)));
        let inactive = sized(Card::new("card-owner-inactive"))
            .colors(owner)
            .header(actions(
                "owner-inactive",
                owner.muted.unwrap_or(t.muted_foreground),
                CardHeader::new("card-owner-inactive-header").title("bash (muted title)").title_muted(true),
            ))
            .child(body("~ $").text_color(owner.text.unwrap_or(t.foreground)));
        let square = sized(Card::new("card-plain")).child(
            recorded(div().size_full().bg(rgb(CARD_SQUARE)), "card:plain:content", records).size_full(),
        );
        let ring_off = sized(Card::new("card-ring-off")).child(body("Ring off"));
        let ring_on = sized(Card::new("card-ring-on")).ring(true).child(body("Ring on"));
        let row = |name: &'static str| div().flex().items_start().gap_4().child(label(name, cx));
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(heading("Card", cx))
            .child(row("Default").child(recorded(default, "card:default", records)))
            .child(
                row("Owner colours").child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .p(px(6.))
                        .bg(rgb(OWNER_CARD_SURROUND))
                        .child(recorded(owned, "card:owner", records))
                        .child(recorded(inactive, "card:owner-inactive", records)),
                ),
            )
            .child(row("No header").child(recorded(square, "card:plain", records)))
            .child(
                row("Ring")
                    .child(recorded(ring_off, "card:ring-off", records))
                    .child(recorded(ring_on, "card:ring-on", records)),
            )
    }

    fn status(&self, cx: &mut Context<Self>) -> Div {
        let t = cx.ui_theme();
        let shell = self.shell.read(cx);
        let clicks: Vec<String> = {
            let clicks = self.clicks.borrow();
            let mut entries: Vec<_> = clicks.iter().collect();
            entries.sort();
            entries.iter().map(|(name, count)| format!("{name} ×{count}")).collect()
        };
        div()
            .flex()
            .flex_col()
            .gap_1()
            .text_size(px(12.))
            .text_color(t.muted_foreground)
            .child(format!(
                "Select: {} ({} change{})",
                shell.selected_label().map_or("—", |label| label.as_ref()),
                self.shell_changes.len(),
                if self.shell_changes.len() == 1 { "" } else { "s" }
            ))
            .child(format!("Command: {}", self.command_log.last().map_or("—", String::as_str)))
            .child(format!("Menu: {}", self.menu_log.last().map_or("—", String::as_str)))
            .child(format!("Tabs: {}", self.tab_log.last().map_or("—", String::as_str)))
            .child(format!("Hosts confirmed: {}", self.confirmed.borrow().join(", ")))
            .child(format!("Clicks: {}", if clicks.is_empty() { "—".to_string() } else { clicks.join(", ") }))
    }

    /// The four corner popovers, absolute in the window's corners, so each
    /// opens near two edges.
    fn corner_triggers(&self) -> Vec<Div> {
        (0..4)
            .map(|corner| {
                let name = format!("corner-{corner}");
                let bounds = self.corners[corner].clone();
                let handle = self.handle(&name);
                let popover = Popover::new(
                    ("corner", corner),
                    move |trigger| {
                        Button::new(("corner-trigger", corner), format!("Popover {corner}"))
                            .variant(ButtonVariant::Secondary)
                            .size(ButtonSize::Sm)
                            .focus(&handle)
                            .on_click(trigger.on_click())
                    },
                    move |_, cx| cx.new(|cx| NotePopover::new(corner, bounds.clone(), cx)),
                );
                div()
                    .absolute()
                    .when(corner < 2, |d| d.top_2())
                    .when(corner >= 2, |d| d.bottom_2())
                    .when(corner % 2 == 0, |d| d.left_2())
                    .when(corner % 2 == 1, |d| d.right_2())
                    .child(recorded(popover, &name, &self.records))
            })
            .collect()
    }
}

impl Focusable for Gallery {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl FocusStops for Gallery {
    /// The enabled controls, top to bottom and left to right, then the
    /// corner popovers' triggers.
    fn stops(&self, cx: &App) -> Vec<FocusHandle> {
        let named = |names: &[&str]| names.iter().map(|name| self.handle(name)).collect::<Vec<_>>();
        let mut stops = named(&["theme", "open-dialog", "open-stubborn", "open-sticky", "open-command-dialog", "command-popover", "behind"]);
        stops.push(self.shell.focus_handle(cx));
        stops.push(self.editor.focus_handle(cx));
        stops.extend(named(&["dropdown-trigger", "long-dropdown-trigger"]));
        stops.push(self.command.focus_handle(cx));
        for variant in ButtonVariant::ALL {
            stops.extend(ButtonSize::ALL.iter().map(|size| self.handle(&Self::button_name(variant, *size))));
        }
        stops.extend(named(&["icon-label"]));
        stops.push(self.name.focus_handle(cx));
        stops.push(self.email.focus_handle(cx));
        stops.extend(named(&[
            "checkbox", "switch", "toggle", "tabs-default", "tabs-plain", "tabs-strip", "corner-0", "corner-1", "corner-2",
            "corner-3",
        ]));
        stops
    }
}

impl Render for Gallery {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.ui_theme();
        let content = div()
            .id("gallery-content")
            .flex()
            .flex_col()
            .gap_6()
            .px_8()
            .py_12()
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(self.header(cx))
            .child(self.overlays_section(cx))
            .child(self.selects_section(cx))
            .child(self.menus_section(cx))
            .child(self.command_section(cx))
            .child(self.buttons_section(cx))
            .child(self.chrome_section(cx))
            .child(self.inputs_section(cx))
            .child(self.toggles_section(cx))
            .child(self.toggle_tooltip_section(cx))
            .child(self.tabs_section(cx))
            .child(self.cards_section(cx))
            .child(self.status(cx));
        // The scope's Tab binding is reached only from focus inside it, so
        // the gallery's own handle, focused at start and by the checks'
        // reset, sits inside, as a page's body does.
        FocusScope::new(&cx.entity()).size_full().child(
            div()
                .track_focus(&self.focus)
                .relative()
                .size_full()
                .bg(t.background)
                .text_color(t.foreground)
                .text_size(px(14.))
                .child(content)
                .children(self.corner_triggers())
                // Takes no room; its menu is drawn above the window.
                .child(self.context_menu.clone())
                // Modal layers render last in the root: tree order is their
                // z-order, and nothing lifts them.
                .children(self.dialog.render(window, cx))
                .children(self.sticky_dialog.render(window, cx))
                .children(self.command_dialog.render(window, cx)),
        )
    }
}

// ---- the dialog ---------------------------------------------------------------

/// The demo dialog: an input, a select and two buttons, with a close `X`.
/// The stubborn one answers `Dismiss` itself, so Escape and the backdrop
/// leave it open with a warning, and only Discard (or Save) closes it.
pub struct DemoDialog {
    focus: FocusHandle,
    pub name: Entity<TextInput>,
    pub shell: Entity<Select>,
    pub cancel: FocusHandle,
    pub save: FocusHandle,
    pub close: FocusHandle,
    pub discard: FocusHandle,
    pub stubborn: bool,
    pub warned: bool,
    pub dismiss_requests: usize,
    pub saves: usize,
    /// The select trigger's painted bounds, for the pointer checks.
    pub shell_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl EventEmitter<DismissEvent> for DemoDialog {}

impl Focusable for DemoDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl FocusStops for DemoDialog {
    /// The body, the footer, then the close button, as in shadcn's markup.
    fn stops(&self, cx: &App) -> Vec<FocusHandle> {
        let mut stops = vec![
            self.name.focus_handle(cx),
            self.shell.focus_handle(cx),
            self.cancel.clone(),
        ];
        if self.warned {
            stops.push(self.discard.clone());
        }
        stops.push(self.save.clone());
        stops.push(self.close.clone());
        stops
    }
}

impl DemoDialog {
    fn new(stubborn: bool, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            name: cx.new(|cx| TextInput::new("", cx).with_placeholder("Profile name")),
            shell: cx.new(|cx| Select::new(["bash", "fish", "foot"], Some(0), cx)),
            cancel: cx.focus_handle(),
            save: cx.focus_handle(),
            close: cx.focus_handle(),
            discard: cx.focus_handle(),
            stubborn,
            warned: false,
            dismiss_requests: 0,
            saves: 0,
            shell_bounds: Rc::default(),
        }
    }

    /// What a dismiss request does: the plain dialog closes, the stubborn
    /// one warns and stays.
    fn request_close(&mut self, cx: &mut Context<Self>) {
        self.dismiss_requests += 1;
        if self.stubborn {
            self.warned = true;
            cx.notify();
        } else {
            cx.emit(DismissEvent);
        }
    }
}

impl Render for DemoDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.ui_theme();
        let field = |name: &'static str, control: gpui::AnyElement| {
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(div().w(px(64.)).flex_none().text_color(t.muted_foreground).child(name))
                .child(div().flex_1().child(control))
        };
        let shell_bounds = self.shell_bounds.clone();
        let shell = div().child(self.shell.clone()).on_children_prepainted(move |painted, _, _| {
            if let Some(first) = painted.first() {
                shell_bounds.set(Some(*first));
            }
        });
        div()
            .track_focus(&self.focus)
            // The stubborn dialog answers Dismiss itself; the layer's
            // default (closing) never sees it.
            .when(self.stubborn, |d| d.on_action(cx.listener(|this, _: &Dismiss, _, cx| this.request_close(cx))))
            .child(
                Dialog::new()
                    .title(if self.stubborn { "Unsaved profile" } else { "Edit profile" })
                    .description("The name shows in the tab strip; the shell starts in new tabs.")
                    .close_focus(&self.close)
                    .on_close(cx.listener(|this, _: &ClickEvent, _, cx| this.request_close(cx)))
                    .child(field("Name", Input::new(&self.name).into_any_element()))
                    .child(field("Shell", shell.into_any_element()))
                    .when(self.warned, |d| {
                        d.child(div().text_color(t.destructive).child("Unsaved changes: discard them to close."))
                    })
                    .footer(
                        Button::new("cancel", "Cancel")
                            .variant(ButtonVariant::Outline)
                            .focus(&self.cancel)
                            .on_click(cx.listener(|this, _, _, cx| this.request_close(cx))),
                    )
                    .when(self.warned, |d| {
                        d.footer(
                            Button::new("discard", "Discard")
                                .variant(ButtonVariant::Destructive)
                                .focus(&self.discard)
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                        )
                    })
                    .footer(Button::new("save", "Save").focus(&self.save).on_click(cx.listener(|this, _, _, cx| {
                        this.saves += 1;
                        cx.emit(DismissEvent);
                    }))),
            )
    }
}

// ---- command hosts ------------------------------------------------------------

/// Where a [`CommandHost`] shows its command list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chrome {
    Dialog,
    Popover,
}

/// A managed view around a [`Command`]: in a dialog or in a popover. It
/// turns `Cancelled` into its own dismissal, and on `Confirmed` closes
/// first, then records the item.
pub struct CommandHost {
    focus: FocusHandle,
    pub command: Entity<Command>,
    chrome: Chrome,
    confirmed: Confirmed,
    _subscription: Subscription,
}

impl EventEmitter<DismissEvent> for CommandHost {}

impl Focusable for CommandHost {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl FocusStops for CommandHost {
    fn stops(&self, cx: &App) -> Vec<FocusHandle> {
        vec![self.command.focus_handle(cx)]
    }
}

impl CommandHost {
    fn new(chrome: Chrome, confirmed: Confirmed, cx: &mut Context<Self>) -> Self {
        let command = cx.new(|cx| Command::new(command_groups(), cx));
        let subscription = cx.subscribe(&command, |this, command, event: &CommandEvent, cx| match event {
            CommandEvent::Confirmed(ix) => {
                let label = command.read(cx).item(*ix).map(|item| item.label.to_string());
                // Close before acting on the item.
                cx.emit(DismissEvent);
                if let Some(label) = label {
                    this.confirmed.borrow_mut().push(label);
                }
            }
            CommandEvent::Cancelled => cx.emit(DismissEvent),
        });
        Self { focus: cx.focus_handle(), command, chrome, confirmed, _subscription: subscription }
    }
}

impl Render for CommandHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let panel = match self.chrome {
            Chrome::Dialog => Dialog::new()
                .title("Run a command")
                .description("Type to filter; enter runs the highlighted command.")
                .child(self.command.clone())
                .into_any_element(),
            Chrome::Popover => PopoverContent::new().p_0().child(self.command.clone()).into_any_element(),
        };
        div().track_focus(&self.focus).child(panel)
    }
}

// ---- the corner popovers --------------------------------------------------------

/// A corner popover's content: a note and a Close button. It records its
/// panel's painted bounds for the placement checks.
pub struct NotePopover {
    focus: FocusHandle,
    pub close: FocusHandle,
    corner: usize,
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl EventEmitter<DismissEvent> for NotePopover {}

impl Focusable for NotePopover {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl FocusStops for NotePopover {
    fn stops(&self, _: &App) -> Vec<FocusHandle> {
        vec![self.close.clone()]
    }
}

impl NotePopover {
    fn new(corner: usize, bounds: Rc<Cell<Option<Bounds<Pixels>>>>, cx: &mut Context<Self>) -> Self {
        bounds.set(None);
        Self { focus: cx.focus_handle(), close: cx.focus_handle(), corner, bounds }
    }
}

impl Drop for NotePopover {
    fn drop(&mut self) {
        self.bounds.set(None);
    }
}

impl Render for NotePopover {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds = self.bounds.clone();
        div()
            .track_focus(&self.focus)
            .on_children_prepainted(move |painted, _, _| {
                if let Some(first) = painted.first() {
                    bounds.set(Some(*first));
                }
            })
            .child(
                PopoverContent::new()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().font_weight(gpui::FontWeight::MEDIUM).child(format!("Popover {}", self.corner)))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(cx.ui_theme().muted_foreground)
                            .child("Anchored below its trigger, centred; it flips and stays inside the window."),
                    )
                    .child(
                        Button::new("close", "Close")
                            .variant(ButtonVariant::Outline)
                            .size(ButtonSize::Sm)
                            .focus(&self.close)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                    ),
            )
    }
}

// ---- --check ----------------------------------------------------------------------

/// The checks: the Verify clauses of tasks 4.x and 5.1–5.7, driven from
/// inside the app.
mod check {
    use std::time::Duration;

    use brindle_ui::{ActiveTheme, Menu, MenuAnchor, Modality, modality, primitives::layer};
    use gpui::{
        App, AppContext, AsyncApp, Bounds, Context, Entity, FocusHandle, Focusable, Keystroke, MouseButton, Pixels,
        Point, SharedString, Window, WindowHandle, point, px,
    };

    use super::Gallery;

    /// The window messages the checks post, and the user32 calls for them.
    /// Declared here so the example adds no dependency.
    #[cfg(windows)]
    mod win {
        pub const WM_MOUSEMOVE: u32 = 0x0200;
        pub const WM_LBUTTONDOWN: u32 = 0x0201;
        pub const WM_LBUTTONUP: u32 = 0x0202;
        pub const WM_RBUTTONDOWN: u32 = 0x0204;
        pub const WM_RBUTTONUP: u32 = 0x0205;
        pub const WM_MBUTTONDOWN: u32 = 0x0207;
        pub const WM_MBUTTONUP: u32 = 0x0208;
        pub const WM_KEYUP: u32 = 0x0101;
        pub const MK_LBUTTON: usize = 0x0001;
        pub const MK_RBUTTON: usize = 0x0002;
        pub const MK_MBUTTON: usize = 0x0010;
        /// A key-up's lparam: repeat count 1, previous state down, transition up.
        pub const KEYUP_LPARAM: isize = 0xC000_0001u32 as isize;

        #[link(name = "user32")]
        unsafe extern "system" {
            pub fn FindWindowW(class_name: *const u16, window_name: *const u16) -> isize;
            pub fn GetActiveWindow() -> isize;
            pub fn PostMessageW(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> i32;
        }

        /// The top-level window titled `title`, else this thread's active
        /// window, else 0.
        pub fn find_window(title: &str) -> isize {
            let title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
            match unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) } {
                0 => unsafe { GetActiveWindow() },
                hwnd => hwnd,
            }
        }

        pub fn post(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> bool {
            unsafe { PostMessageW(hwnd, msg, wparam, lparam) != 0 }
        }

        /// What the window shows, as `PrintWindow` renders it: its client
        /// area in physical pixels, BGRA, top row first.
        pub struct Capture {
            pub width: usize,
            pub height: usize,
            bgra: Vec<u8>,
        }

        impl Capture {
            /// The colour at `(x, y)`, as RGB.
            pub fn rgb(&self, x: usize, y: usize) -> Option<[u8; 3]> {
                if x >= self.width || y >= self.height {
                    return None;
                }
                let at = (y * self.width + x) * 4;
                Some([self.bgra[at + 2], self.bgra[at + 1], self.bgra[at]])
            }
        }

        #[repr(C)]
        struct Rect {
            left: i32,
            top: i32,
            right: i32,
            bottom: i32,
        }

        /// A `BITMAPINFO` for 32-bit pixels, with room for the colour masks
        /// `GetDIBits` may write after the header.
        #[repr(C)]
        struct BitmapInfo {
            size: u32,
            width: i32,
            height: i32,
            planes: u16,
            bit_count: u16,
            compression: u32,
            size_image: u32,
            x_pels_per_meter: i32,
            y_pels_per_meter: i32,
            clr_used: u32,
            clr_important: u32,
            colors: [u32; 3],
        }

        /// `PW_CLIENTONLY | PW_RENDERFULLCONTENT`: the client area, with
        /// what DirectComposition shows (gpui's swap chain) included.
        const PRINT_FLAGS: u32 = 0x1 | 0x2;

        #[link(name = "user32")]
        unsafe extern "system" {
            fn GetClientRect(hwnd: isize, rect: *mut Rect) -> i32;
            fn GetDC(hwnd: isize) -> isize;
            fn ReleaseDC(hwnd: isize, dc: isize) -> i32;
            fn PrintWindow(hwnd: isize, dc: isize, flags: u32) -> i32;
        }

        #[link(name = "gdi32")]
        unsafe extern "system" {
            fn CreateCompatibleDC(dc: isize) -> isize;
            fn CreateCompatibleBitmap(dc: isize, width: i32, height: i32) -> isize;
            fn SelectObject(dc: isize, object: isize) -> isize;
            fn GetDIBits(dc: isize, bitmap: isize, start: u32, lines: u32, bits: *mut u8, info: *mut BitmapInfo, usage: u32) -> i32;
            fn DeleteObject(object: isize) -> i32;
            fn DeleteDC(dc: isize) -> i32;
        }

        /// Renders the window `hwnd` into memory: the gallery reading its
        /// own pixels. Call it off the window's thread, which must keep
        /// handling messages meanwhile.
        pub fn capture(hwnd: isize) -> Option<Capture> {
            let mut rect = Rect { left: 0, top: 0, right: 0, bottom: 0 };
            if unsafe { GetClientRect(hwnd, &mut rect) } == 0 {
                return None;
            }
            let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
            if width <= 0 || height <= 0 {
                return None;
            }
            unsafe {
                let screen = GetDC(0);
                let dc = CreateCompatibleDC(screen);
                let bitmap = CreateCompatibleBitmap(screen, width, height);
                let previous = SelectObject(dc, bitmap);
                let printed = PrintWindow(hwnd, dc, PRINT_FLAGS) != 0;
                SelectObject(dc, previous);
                let mut info = BitmapInfo {
                    size: 40,
                    width,
                    // Negative: top row first.
                    height: -height,
                    planes: 1,
                    bit_count: 32,
                    compression: 0,
                    size_image: 0,
                    x_pels_per_meter: 0,
                    y_pels_per_meter: 0,
                    clr_used: 0,
                    clr_important: 0,
                    colors: [0; 3],
                };
                let mut bgra = vec![0u8; width as usize * height as usize * 4];
                let lines = GetDIBits(dc, bitmap, 0, height as u32, bgra.as_mut_ptr(), &mut info, 0);
                DeleteObject(bitmap);
                DeleteDC(dc);
                ReleaseDC(0, screen);
                (printed && lines == height).then_some(Capture { width: width as usize, height: height as usize, bgra })
            }
        }

        /// The virtual-key code of a key the checks release.
        pub fn virtual_key(key: &str) -> Option<usize> {
            Some(match key {
                "enter" => 0x0D,
                "space" => 0x20,
                "escape" => 0x1B,
                "tab" => 0x09,
                _ => return None,
            })
        }
    }

    /// A mouse button's down and up messages, and its flag in a message's
    /// wparam while it is held.
    #[cfg(windows)]
    fn button_messages(button: MouseButton) -> (u32, u32, usize) {
        match button {
            MouseButton::Right => (win::WM_RBUTTONDOWN, win::WM_RBUTTONUP, win::MK_RBUTTON),
            MouseButton::Middle => (win::WM_MBUTTONDOWN, win::WM_MBUTTONUP, win::MK_MBUTTON),
            _ => (win::WM_LBUTTONDOWN, win::WM_LBUTTONUP, win::MK_LBUTTON),
        }
    }

    /// Why the pointer checks can't run on this platform.
    #[cfg(not(windows))]
    const NO_MESSAGES: &str = "needs a real pointer: the gallery posts presses as window messages on Windows only";

    /// Why a check of what the window shows can't run on this platform.
    #[cfg(not(windows))]
    const NO_PIXELS: &str = "reads the window's pixels, which the gallery does on Windows only";

    /// Why a check about a key's release can't run on this platform.
    const NO_KEY_UP: &str = "needs a real key release: the gallery posts those as window messages on Windows only";

    /// Why a check didn't pass.
    pub enum Failure {
        Fail(String),
        Skip(String),
    }

    type Check = Result<(), Failure>;

    macro_rules! fail {
        ($($arg:tt)*) => { return Err(Failure::Fail(format!($($arg)*))) };
    }

    macro_rules! ensure {
        ($cond:expr, $($arg:tt)*) => { if !$cond { fail!($($arg)*) } };
    }

    /// A frame's worth of waiting: gpui draws dirty windows on the next
    /// vsync, and deferred effects run between.
    const TICK: Duration = Duration::from_millis(60);

    /// Long enough for a posted mouse move to be handled.
    const HOVER_TICK: Duration = Duration::from_millis(12);

    /// Longer than the typeahead's accumulation window.
    const TYPEAHEAD_GAP: Duration = Duration::from_millis(1200);

    /// The run can't hang: the process dies after this.
    const DEADLINE: Duration = Duration::from_secs(150);

    /// A panic anywhere ends the run as a failure, and a hung run ends.
    pub fn install_guards() {
        std::panic::set_hook(Box::new(|info| {
            println!("FAIL panic: {info}");
            println!("{}", std::backtrace::Backtrace::force_capture());
            let _ = std::io::stdout().flush();
            std::process::exit(2);
        }));
        std::thread::spawn(|| {
            std::thread::sleep(DEADLINE);
            println!("FAIL timeout: the checks didn't finish within {}s", DEADLINE.as_secs());
            let _ = std::io::stdout().flush();
            std::process::exit(3);
        });
    }

    use std::io::Write as _;

    pub struct Checker {
        window: WindowHandle<Gallery>,
        cx: AsyncApp,
        /// The platform window the messages go to (Windows), else 0.
        #[cfg_attr(not(windows), allow(dead_code))]
        hwnd: isize,
        /// Physical pixels per logical pixel, for the messages' coordinates.
        scale: f32,
        ok: usize,
        skipped: usize,
        failed: usize,
    }

    impl Checker {
        async fn tick(&self) {
            self.cx.background_executor().timer(TICK).await;
        }

        /// Posts a message to the gallery's window. The platform handles it
        /// on the main thread before the next tick's continuation runs.
        #[cfg(windows)]
        fn post(&mut self, msg: u32, wparam: usize, lparam: isize) -> Result<(), Failure> {
            if self.hwnd == 0 {
                return Err(Failure::Skip("the gallery window wasn't found by its title".to_string()));
            }
            if win::post(self.hwnd, msg, wparam, lparam) { Ok(()) } else { fail!("PostMessageW failed") }
        }

        #[cfg(not(windows))]
        fn post(&mut self, _msg: u32, _wparam: usize, _lparam: isize) -> Result<(), Failure> {
            Err(Failure::Skip(NO_MESSAGES.to_string()))
        }

        /// A point in the window as a mouse message's lparam: client
        /// coordinates in physical pixels, y in the high word.
        fn mouse_lparam(&self, at: Point<Pixels>) -> isize {
            let x = (f32::from(at.x) * self.scale).round() as i32;
            let y = (f32::from(at.y) * self.scale).round() as i32;
            (((y as u32) << 16) | (x as u32 & 0xffff)) as i32 as isize
        }

        /// Reads or changes the gallery. Events are never dispatched from
        /// here: a key the gallery itself handles would re-enter it.
        fn with<R>(
            &mut self,
            f: impl FnOnce(&mut Gallery, &mut Window, &mut Context<Gallery>) -> R,
        ) -> Result<R, Failure> {
            self.window.update(&mut self.cx, f).map_err(|err| Failure::Fail(format!("the window is gone: {err}")))
        }

        /// Reads an entity.
        fn read<T: 'static, R>(&mut self, entity: &Entity<T>, f: impl FnOnce(&T, &App) -> R) -> Result<R, Failure> {
            let entity = entity.clone();
            self.with(move |_, _, cx| f(entity.read(cx), cx))
        }

        /// Dispatches an event through the window, holding no entity.
        fn dispatch(&mut self, f: impl FnOnce(&mut Window, &mut App)) -> Result<(), Failure> {
            self.cx
                .update_window(self.window.into(), |_, window, cx| f(window, cx))
                .map_err(|err| Failure::Fail(format!("the window is gone: {err}")))
        }

        /// A key press, through gpui's own key dispatch (bindings, key
        /// listeners, then the input handler for a character), then a frame.
        async fn key(&mut self, key: &str) -> Result<(), Failure> {
            let keystroke = Keystroke::parse(key).map_err(|err| Failure::Fail(format!("bad keystroke {key:?}: {err:?}")))?;
            self.dispatch(|window, cx| {
                window.dispatch_keystroke(keystroke, cx);
            })?;
            self.tick().await;
            Ok(())
        }

        /// The release of `key`, which gpui turns into a keyboard click on
        /// the focused control: a `WM_KEYUP` to the window. Returns whether
        /// a release could be sent at all: where it can't, a check that
        /// needs the release itself skips, and the others go on without it
        /// (the controls act on the key-down).
        async fn key_up(&mut self, key: &str) -> Result<bool, Failure> {
            #[cfg(windows)]
            {
                let Some(vk) = win::virtual_key(key) else { fail!("no virtual key for {key:?}") };
                self.post(win::WM_KEYUP, vk, win::KEYUP_LPARAM)?;
                self.tick().await;
                Ok(true)
            }
            #[cfg(not(windows))]
            {
                let _ = key;
                Ok(false)
            }
        }

        /// A press and release of the primary button at `at`, a frame apart.
        async fn press(&mut self, at: Point<Pixels>) -> Result<(), Failure> {
            let lparam = self.mouse_lparam(at);
            #[cfg(windows)]
            {
                self.post(win::WM_LBUTTONDOWN, win::MK_LBUTTON, lparam)?;
                self.tick().await;
                self.post(win::WM_LBUTTONUP, 0, lparam)?;
            }
            #[cfg(not(windows))]
            self.post(0, 0, lparam)?;
            self.tick().await;
            Ok(())
        }

        /// One press-drag-release gesture: the primary button goes down at
        /// `from`, the pointer moves to `to` with it held, and it comes up
        /// there, a frame (and a hover tick) apart.
        async fn drag(&mut self, from: Point<Pixels>, to: Point<Pixels>) -> Result<(), Failure> {
            let (down, up) = (self.mouse_lparam(from), self.mouse_lparam(to));
            #[cfg(windows)]
            {
                self.post(win::WM_LBUTTONDOWN, win::MK_LBUTTON, down)?;
                self.tick().await;
                self.post(win::WM_MOUSEMOVE, win::MK_LBUTTON, up)?;
                self.cx.background_executor().timer(HOVER_TICK).await;
                self.tick().await;
                self.post(win::WM_LBUTTONUP, 0, up)?;
            }
            #[cfg(not(windows))]
            {
                let _ = up;
                self.post(0, 0, down)?;
            }
            self.tick().await;
            Ok(())
        }

        /// Moves the pointer to `at`, and gives the message a moment.
        async fn hover(&mut self, at: Point<Pixels>) -> Result<(), Failure> {
            let lparam = self.mouse_lparam(at);
            #[cfg(windows)]
            self.post(win::WM_MOUSEMOVE, 0, lparam)?;
            #[cfg(not(windows))]
            self.post(0, 0, lparam)?;
            self.cx.background_executor().timer(HOVER_TICK).await;
            Ok(())
        }

        /// `button` goes down at `at` (and stays down), then a frame.
        async fn button_down(&mut self, button: MouseButton, at: Point<Pixels>) -> Result<(), Failure> {
            let lparam = self.mouse_lparam(at);
            #[cfg(windows)]
            {
                let (down, _, held) = button_messages(button);
                self.post(down, held, lparam)?;
            }
            #[cfg(not(windows))]
            {
                let _ = button;
                self.post(0, 0, lparam)?;
            }
            self.tick().await;
            Ok(())
        }

        /// The pointer moves to `at` with `button` held, then a frame.
        async fn button_move(&mut self, button: MouseButton, at: Point<Pixels>) -> Result<(), Failure> {
            let lparam = self.mouse_lparam(at);
            #[cfg(windows)]
            {
                let (_, _, held) = button_messages(button);
                self.post(win::WM_MOUSEMOVE, held, lparam)?;
            }
            #[cfg(not(windows))]
            {
                let _ = button;
                self.post(0, 0, lparam)?;
            }
            self.cx.background_executor().timer(HOVER_TICK).await;
            self.tick().await;
            Ok(())
        }

        /// `button` comes up at `at`, then a frame.
        async fn button_up(&mut self, button: MouseButton, at: Point<Pixels>) -> Result<(), Failure> {
            let lparam = self.mouse_lparam(at);
            #[cfg(windows)]
            {
                let (_, up, _) = button_messages(button);
                self.post(up, 0, lparam)?;
            }
            #[cfg(not(windows))]
            {
                let _ = button;
                self.post(0, 0, lparam)?;
            }
            self.tick().await;
            Ok(())
        }

        /// Focuses the control called `name`, as Tab would land on it.
        async fn focus(&mut self, name: &str) -> Result<(), Failure> {
            self.with(|gallery, window, _| window.focus(&gallery.handle(name)))?;
            self.tick().await;
            Ok(())
        }

        async fn focus_handle(&mut self, handle: &FocusHandle) -> Result<(), Failure> {
            let handle = handle.clone();
            self.with(move |_, window, _| window.focus(&handle))?;
            self.tick().await;
            Ok(())
        }

        fn focused(&mut self, handle: &FocusHandle) -> Result<bool, Failure> {
            let handle = handle.clone();
            self.with(move |_, window, _| handle.is_focused(window))
        }

        fn focused_name(&mut self, name: &str) -> Result<bool, Failure> {
            let name = name.to_string();
            self.with(move |gallery, window, _| gallery.handle(&name).is_focused(window))
        }

        fn layers(&mut self) -> Result<usize, Failure> {
            self.with(|_, window, cx| layer::stack(window, cx).map_or(0, |stack| stack.len()))
        }

        fn modality(&mut self) -> Result<Modality, Failure> {
            self.with(|_, window, cx| modality(window, cx))
        }

        fn clicks(&mut self, name: &str) -> Result<usize, Failure> {
            let name = name.to_string();
            self.with(move |gallery, _, _| gallery.clicks(&name))
        }

        fn recorded(&mut self, name: &str) -> Result<Bounds<Pixels>, Failure> {
            let key = name.to_string();
            self.with(move |gallery, _, _| gallery.recorded(&key))?
                .ok_or_else(|| Failure::Fail(format!("no painted bounds recorded for {name:?}")))
        }

        /// The bounds of the control called `name` once it is in view: the
        /// gallery is taller than the window, so a control below the fold
        /// is scrolled to first. A press needs the control on screen; a
        /// press below the window would hit nothing and pass for nothing.
        async fn reveal(&mut self, name: &str) -> Result<Bounds<Pixels>, Failure> {
            let in_view = |bounds: Bounds<Pixels>, viewport: gpui::Size<Pixels>| {
                bounds.top() >= px(0.) && bounds.bottom() <= viewport.height
            };
            let bounds = self.recorded(name)?;
            let viewport = self.viewport()?;
            if in_view(bounds, viewport) {
                return Ok(bounds);
            }
            self.with(move |gallery, window, _| {
                let offset = gallery.scroll.offset();
                gallery.scroll.set_offset(point(offset.x, offset.y - (bounds.top() - px(120.))));
                window.refresh();
            })?;
            self.tick().await;
            self.tick().await;
            let bounds = self.recorded(name)?;
            ensure!(in_view(bounds, viewport), "{name} is still out of view at {bounds:?} after scrolling");
            Ok(bounds)
        }

        /// The colour the window shows at `at`, once the next frame is
        /// drawn: the gallery renders its own window with `PrintWindow`, off
        /// the main thread, which keeps handling messages meanwhile.
        async fn pixel(&mut self, at: Point<Pixels>) -> Result<[u8; 3], Failure> {
            Ok(self.pixels(&[at]).await?[0])
        }

        /// The colours at each of `points`, from one rendering of the window.
        #[cfg(windows)]
        async fn pixels(&mut self, points: &[Point<Pixels>]) -> Result<Vec<[u8; 3]>, Failure> {
            if self.hwnd == 0 {
                return Err(Failure::Skip("the gallery window wasn't found by its title".to_string()));
            }
            self.tick().await;
            let hwnd = self.hwnd;
            let capture = self
                .cx
                .background_executor()
                .spawn(async move { win::capture(hwnd) })
                .await
                .ok_or_else(|| Failure::Skip("PrintWindow couldn't render the window".to_string()))?;
            let scale = self.scale;
            points
                .iter()
                .map(|at| {
                    let x = (f32::from(at.x) * scale).floor() as usize;
                    let y = (f32::from(at.y) * scale).floor() as usize;
                    capture.rgb(x, y).ok_or_else(|| Failure::Fail(format!("{at:?} is outside the window")))
                })
                .collect()
        }

        #[cfg(not(windows))]
        async fn pixels(&mut self, _points: &[Point<Pixels>]) -> Result<Vec<[u8; 3]>, Failure> {
            Err(Failure::Skip(NO_PIXELS.to_string()))
        }

        fn viewport(&mut self) -> Result<gpui::Size<Pixels>, Failure> {
            self.with(|_, window, _| window.viewport_size())
        }

        fn handle(&mut self, name: &str) -> Result<FocusHandle, Failure> {
            let name = name.to_string();
            self.with(move |gallery, _, _| gallery.handle(&name))
        }

        /// Finds a point on the row of option `target` in `select`'s open
        /// menu by moving the pointer down the menu (and up above the
        /// trigger, for a flipped menu) until the highlight lands on it.
        /// The highlight is first moved off the target with the keys, so
        /// only the pointer can bring it there.
        async fn option_row(
            &mut self,
            select: &Entity<Select>,
            trigger: Bounds<Pixels>,
            target: usize,
        ) -> Result<Point<Pixels>, Failure> {
            let highlighted = self.read(select, |select, cx| select.highlighted(cx))?;
            if highlighted == Some(target) {
                self.key(if target == 0 { "end" } else { "home" }).await?;
            }
            let x = trigger.left() + px(24.);
            let below = (0..70).map(|step| trigger.bottom() + px(6. + 6. * step as f32));
            let above = (0..70).map(|step| trigger.top() - px(6. + 6. * step as f32));
            for y in below.chain(above) {
                let at = point(x, y);
                self.hover(at).await?;
                let select = select.clone();
                if self.with(move |_, _, cx| select.read(cx).highlighted(cx))? == Some(target) {
                    return Ok(at);
                }
            }
            Err(Failure::Skip(format!("the pointer found no menu row for option {target}")))
        }

        /// Closes whatever a check left open and puts focus back on the
        /// gallery itself, on no control.
        async fn reset(&mut self) -> Result<(), Failure> {
            for _ in 0..4 {
                if self.layers()? == 0 {
                    break;
                }
                self.key("escape").await?;
            }
            self.with(|gallery, window, cx| {
                gallery.close_all(window, cx);
                gallery.reset_tabs(cx);
                gallery.scroll.set_offset(point(px(0.), px(0.)));
                // Back to the gallery itself, as a page's body.
                window.focus(&gallery.focus);
                window.refresh();
            })?;
            self.tick().await;
            self.tick().await;
            Ok(())
        }

        async fn run_check(&mut self, name: &str, check: Check) {
            match check {
                Ok(()) => {
                    self.ok += 1;
                    println!("ok {name}");
                }
                Err(Failure::Skip(why)) => {
                    self.skipped += 1;
                    println!("SKIP {name}: {why}");
                }
                Err(Failure::Fail(why)) => {
                    self.failed += 1;
                    println!("FAIL {name}: {why}");
                }
            }
            let _ = std::io::stdout().flush();
            if let Err(Failure::Fail(why)) = self.reset().await {
                println!("FAIL reset after {name}: {why}");
                self.failed += 1;
            }
        }
    }

    use brindle_ui::Select;

    /// Runs every check and exits with 0 when all passed or were skipped.
    pub async fn run(window: WindowHandle<Gallery>, title: SharedString, cx: AsyncApp) {
        let mut c = Checker { window, cx, hwnd: 0, scale: 1., ok: 0, skipped: 0, failed: 0 };
        // Let the first frames paint, so every control has recorded bounds.
        for _ in 0..5 {
            c.tick().await;
        }
        #[cfg(windows)]
        {
            c.hwnd = win::find_window(&title);
            if c.hwnd == 0 {
                println!("FAIL setup: no window titled {title:?}");
                std::process::exit(1);
            }
        }
        #[cfg(not(windows))]
        let _ = title;
        c.scale = match c.with(|_, window, _| window.scale_factor()) {
            Ok(scale) => scale,
            Err(_) => {
                println!("FAIL setup: the window is gone");
                std::process::exit(1);
            }
        };
        // First, as the window opened: Tab must work from the start.
        let check = tab_moves_focus(&mut c).await;
        c.run_check("tab-moves-focus-through-the-gallery-and-wraps", check).await;
        let check = theme_switch(&mut c).await;
        c.run_check("theme-switch-redraws-in-dark-tokens", check).await;
        let check = button_mouse_click(&mut c).await;
        c.run_check("button-click-counts-focuses-without-ring", check).await;
        let check = button_space_activates_once(&mut c).await;
        c.run_check("button-space-and-enter-activate-once", check).await;
        let check = button_disabled_inert(&mut c).await;
        c.run_check("button-disabled-runs-nothing-keeps-focus", check).await;
        let check = icon_button_sizes(&mut c).await;
        c.run_check("icon-button-sizes-are-two-and-a-quarter-two-and-one-and-a-half-rem", check).await;
        let check = focus_visible(&mut c).await;
        c.run_check("focus-visible-pointer-then-keyboard", check).await;
        let check = ring_paints(&mut c).await;
        c.run_check("focus-ring-paints-outside-each-tabbed-control", check).await;
        let check = input_frame_press(&mut c).await;
        c.run_check("input-press-in-frame-focuses-text-input", check).await;
        let check = checkbox_space(&mut c).await;
        c.run_check("checkbox-space-toggles-enter-does-not", check).await;
        let check = switch_space_and_enter(&mut c).await;
        c.run_check("switch-space-and-enter-toggle", check).await;
        let check = checkbox_disabled(&mut c).await;
        c.run_check("checkbox-disabled-asks-nothing", check).await;
        let check = toggle_space_and_enter(&mut c).await;
        c.run_check("toggle-space-and-enter-flip-it", check).await;
        let check = toggle_click(&mut c).await;
        c.run_check("toggle-click-flips-it-and-a-handleless-press-keeps-focus", check).await;
        let check = toggle_disabled_keys(&mut c).await;
        c.run_check("toggle-disabled-space-asks-nothing", check).await;
        let check = toggle_disabled_press(&mut c).await;
        c.run_check("toggle-disabled-press-asks-nothing-keeps-focus", check).await;
        // Each moves the pointer away after, passed or not, so no tooltip
        // stays up for the later checks' pixel samples.
        let check = tooltip_hover_and_press(&mut c).await;
        let check = pointer_away_after(&mut c, check).await;
        c.run_check("tooltip-hover-shows-it-press-hides-it-and-clicks", check).await;
        let check = tooltip_escape(&mut c).await;
        let check = pointer_away_after(&mut c, check).await;
        c.run_check("tooltip-escape-hides-it", check).await;
        let check = tooltip_disabled(&mut c).await;
        let check = pointer_away_after(&mut c, check).await;
        c.run_check("tooltip-not-shown-on-a-disabled-control", check).await;
        let check = dialog_opens_focusing_input(&mut c).await;
        c.run_check("dialog-opens-with-input-focused-and-typing-goes-there", check).await;
        let check = dialog_tab_cycles(&mut c).await;
        c.run_check("dialog-tab-cycles-every-stop-and-wraps", check).await;
        let check = dialog_escape_returns_focus(&mut c).await;
        c.run_check("dialog-escape-closes-and-focus-returns", check).await;
        let check = dialog_backdrop_returns_focus(&mut c).await;
        c.run_check("dialog-backdrop-press-closes-and-focus-returns", check).await;
        let check = dialog_blocks_press_beneath(&mut c).await;
        c.run_check("dialog-blocks-the-button-beneath-and-asks-to-dismiss", check).await;
        let check = dialog_stubborn(&mut c).await;
        c.run_check("dialog-handling-dismiss-stays-open-until-discard", check).await;
        let check = sticky_backdrop_press_keeps_it_open(&mut c).await;
        c.run_check("sticky-dialog-backdrop-press-keeps-it-open-and-blocks-beneath", check).await;
        let check = sticky_escape_still_closes(&mut c).await;
        c.run_check("sticky-dialog-escape-still-closes", check).await;
        let check = sticky_menu_outside_press_closes_menu_only(&mut c).await;
        c.run_check("sticky-dialog-menu-outside-press-closes-menu-only", check).await;
        let check = select_opens_on_down(&mut c).await;
        c.run_check("select-down-opens-highlighting-the-selection", check).await;
        let check = select_closed_trigger_keys(&mut c).await;
        c.run_check("select-closed-trigger-keys-match-radix", check).await;
        let check = select_press_opens_and_release_on_trigger_stays(&mut c).await;
        c.run_check("select-press-opens-release-on-trigger-stays-open", check).await;
        let check = select_press_drag_release_picks(&mut c).await;
        c.run_check("select-press-drag-release-picks-foot", check).await;
        let check = select_drag_off_and_release_closes(&mut c).await;
        c.run_check("select-dragged-release-outside-or-on-trigger-closes", check).await;
        let check = select_outside_press_reaches_nothing(&mut c).await;
        c.run_check("select-outside-press-closes-menu-only-reaches-nothing", check).await;
        let check = select_nav_skips_disabled(&mut c).await;
        c.run_check("select-nav-skips-disabled-and-does-not-wrap", check).await;
        let check = select_enter_picks(&mut c).await;
        c.run_check("select-enter-picks-fish-tells-owner-refocuses-trigger", check).await;
        let check = select_enter_keyup(&mut c).await;
        c.run_check("select-enter-key-up-does-not-reopen", check).await;
        let check = select_rechoose_same(&mut c).await;
        c.run_check("select-clicking-the-selected-option-tells-nobody", check).await;
        let check = select_escape(&mut c).await;
        c.run_check("select-escape-closes-without-change-refocuses-trigger", check).await;
        let check = select_space_keyboard_modality(&mut c).await;
        c.run_check("select-space-opens-and-picks-as-keyboard-input", check).await;
        let check = select_typeahead_open(&mut c).await;
        c.run_check("select-open-typeahead-f-f-cycles-to-foot", check).await;
        let check = select_typeahead_closed(&mut c).await;
        c.run_check("select-closed-typeahead-moves-the-selection", check).await;
        let check = select_in_dialog_escape(&mut c).await;
        c.run_check("select-in-dialog-escape-closes-menu-only", check).await;
        let check = select_in_dialog_backdrop(&mut c).await;
        c.run_check("select-in-dialog-backdrop-closes-menu-only", check).await;
        let check = select_in_dialog_option_press(&mut c).await;
        c.run_check("select-in-dialog-option-press-picks-and-dialog-stays", check).await;
        for corner in 0..4 {
            let check = popover_corner(&mut c, corner).await;
            c.run_check(&format!("popover-corner-{corner}-fits-and-focus-returns"), check).await;
        }
        let check = popover_outside_press(&mut c).await;
        c.run_check("popover-outside-press-closes-clicks-run-focus-stays", check).await;
        let check = popover_trigger_toggles(&mut c).await;
        c.run_check("popover-trigger-press-toggles", check).await;
        let check = popover_close_button(&mut c).await;
        c.run_check("popover-content-closes-itself", check).await;
        let check = command_filter(&mut c).await;
        c.run_check("command-spr-lists-split-right-highlighted", check).await;
        let check = command_nothing_matches(&mut c).await;
        c.run_check("command-nothing-matches-hides-headings", check).await;
        let check = command_wraps(&mut c).await;
        c.run_check("command-navigation-skips-disabled-and-wraps", check).await;
        let check = command_rows_fill_the_width(&mut c).await;
        c.run_check("command-rows-fill-the-group-width-under-a-filter-border", check).await;
        let check = command_separator_spans_the_list(&mut c).await;
        c.run_check("command-separator-spans-the-list-between-groups", check).await;
        let check = command_enter_and_escape_inline(&mut c).await;
        c.run_check("command-inline-enter-confirms-escape-cancels", check).await;
        let check = command_dialog(&mut c).await;
        c.run_check("command-in-dialog-escape-and-enter", check).await;
        let check = command_popover_hazard(&mut c).await;
        c.run_check("command-in-popover-enter-key-up-does-not-reopen", check).await;
        // Menus: everywhere, opened by key or from code.
        let check = context_menu_corner(&mut c).await;
        c.run_check("context-menu-opens-with-its-corner-at-200-40-highlighting-nothing", check).await;
        let check = context_menu_edges(&mut c).await;
        c.run_check("context-menu-near-the-edges-opens-left-of-the-pointer-and-stays-inside", check).await;
        let check = context_menu_keys(&mut c).await;
        c.run_check("context-menu-keys-skip-disabled-and-separators-without-wrapping", check).await;
        let check = context_menu_enter_selects(&mut c).await;
        c.run_check("context-menu-down-down-enter-selects-the-second-enabled-item", check).await;
        let check = context_menu_space_selects(&mut c).await;
        c.run_check("context-menu-space-selects-and-continues-a-typeahead-search", check).await;
        let check = context_menu_escape(&mut c).await;
        c.run_check("context-menu-escape-closes-and-focus-returns", check).await;
        let check = dropdown_keys(&mut c).await;
        c.run_check("dropdown-enter-space-down-open-below-the-trigger-other-keys-do-not", check).await;
        let check = dropdown_description(&mut c).await;
        c.run_check("dropdown-description-under-the-label-and-t-highlights-tmux", check).await;
        let check = dropdown_long_menu(&mut c).await;
        c.run_check("dropdown-long-menu-fits-the-window-scrolls-and-keeps-the-highlight-in-view", check).await;
        // Menus: the pointer (Windows only).
        let check = context_menu_press_and_near_release(&mut c).await;
        c.run_check("context-menu-secondary-press-opens-at-the-pointer-release-within-10px-stays", check).await;
        let check = context_menu_one_gesture(&mut c).await;
        c.run_check("context-menu-press-drag-release-selects-the-item", check).await;
        let check = context_menu_hover_and_click(&mut c).await;
        c.run_check("context-menu-hover-highlights-click-selects", check).await;
        let check = context_menu_disabled_click(&mut c).await;
        c.run_check("context-menu-disabled-item-click-selects-nothing-stays-open", check).await;
        let check = context_menu_outside_press(&mut c).await;
        c.run_check("context-menu-outside-press-closes-reaches-nothing-focus-returns", check).await;
        let check = dropdown_trigger_toggles(&mut c).await;
        c.run_check("dropdown-trigger-press-opens-second-press-closes", check).await;
        let check = dropdown_drag_selects(&mut c).await;
        c.run_check("dropdown-press-drag-release-selects-the-third-item", check).await;
        let check = dropdown_enter_keyup(&mut c).await;
        c.run_check("dropdown-enter-key-up-does-not-reopen", check).await;
        // Tabs: the keys, everywhere.
        let check = tabs_arrow_activates(&mut c).await;
        c.run_check("tabs-right-on-the-second-of-four-activates-the-third-and-focus-follows", check).await;
        let check = tabs_wrap(&mut c).await;
        c.run_check("tabs-right-on-the-last-wraps-to-the-first-and-left-back", check).await;
        let check = tabs_disabled_skipped(&mut c).await;
        c.run_check("tabs-right-skips-a-disabled-trigger", check).await;
        let check = tabs_home_end(&mut c).await;
        c.run_check("tabs-home-end-pageup-pagedown-go-to-the-ends-other-keys-do-nothing", check).await;
        let check = tabs_one_stop(&mut c).await;
        c.run_check("tabs-each-list-is-one-tab-stop", check).await;
        let check = tabs_widths(&mut c).await;
        c.run_check("tabs-triggers-keep-between-their-min-and-max-width", check).await;
        // Tabs: what is drawn and the pointer (Windows only).
        let check = tabs_ring(&mut c).await;
        c.run_check("tabs-ring-paints-around-the-active-trigger-only", check).await;
        let check = tabs_owner_colours(&mut c).await;
        let check = pointer_away_after(&mut c, check).await;
        c.run_check("tabs-owner-colours-apply-to-active-and-inactive-triggers", check).await;
        let check = tabs_press_activates(&mut c).await;
        c.run_check("tabs-press-activates-and-moves-no-focus", check).await;
        let check = tabs_disabled_press(&mut c).await;
        c.run_check("tabs-press-on-a-disabled-trigger-does-nothing", check).await;
        let check = tabs_close_button(&mut c).await;
        let check = pointer_away_after(&mut c, check).await;
        c.run_check("tabs-close-button-closes-without-activating", check).await;
        let check = tabs_middle_click(&mut c).await;
        c.run_check("tabs-middle-press-asks-to-close-a-closable-trigger", check).await;
        let check = tabs_context_menu(&mut c).await;
        c.run_check("tabs-secondary-press-asks-for-the-trigger-menu", check).await;
        let check = tabs_drag(&mut c).await;
        let check = pointer_away_after(&mut c, check).await;
        c.run_check("tabs-drag-first-onto-third-moves-it-with-a-drop-bar", check).await;
        let check = tabs_tooltip(&mut c).await;
        let check = pointer_away_after(&mut c, check).await;
        c.run_check("tabs-trigger-tooltip-shows-on-hover", check).await;
        // Cards: the layout everywhere, what is drawn on Windows only.
        let check = card_headerless_fills(&mut c).await;
        c.run_check("card-without-a-header-content-fills-inside-the-border", check).await;
        let check = card_header_above_content(&mut c).await;
        c.run_check("card-header-row-along-the-top-inside-the-border", check).await;
        let check = card_default_colours(&mut c).await;
        c.run_check("card-default-colours-are-the-card-foreground-and-border-tokens", check).await;
        let check = card_owner_colours(&mut c).await;
        c.run_check("card-owner-colours-draw-card-and-header-whatever-the-tokens", check).await;
        let check = card_ring(&mut c).await;
        c.run_check("card-ring-is-a-band-outside-the-border-taking-no-room", check).await;
        let check = card_corners(&mut c).await;
        c.run_check("card-corners-show-the-surround-over-square-content", check).await;

        println!("--- {} ok, {} skipped, {} failed", c.ok, c.skipped, c.failed);
        let _ = std::io::stdout().flush();
        std::process::exit(if c.failed > 0 { 1 } else { 0 });
    }

    fn center(bounds: Bounds<Pixels>) -> Point<Pixels> {
        bounds.center()
    }

    /// A point on the backdrop above the centred dialog.
    fn backdrop_point(viewport: gpui::Size<Pixels>) -> Point<Pixels> {
        point(viewport.width / 2., px(24.))
    }

    // ---- tokens and controls ----

    async fn theme_switch(c: &mut Checker) -> Check {
        c.focus("theme").await?;
        c.key("space").await?;
        c.key_up("space").await?;
        let dark = c.with(|_, _, cx| cx.ui_theme().dark)?;
        ensure!(dark, "space on the Dark switch didn't make the dark set active");
        let primary = c.with(|_, _, cx| cx.ui_theme().primary)?;
        ensure!(primary == Tokens::neutral_dark().primary, "the dark set's primary isn't active");
        c.key("space").await?;
        c.key_up("space").await?;
        let dark = c.with(|_, _, cx| cx.ui_theme().dark)?;
        ensure!(!dark, "space again didn't bring the light set back");
        Ok(())
    }

    use brindle_ui::Tokens;

    async fn button_mouse_click(c: &mut Checker) -> Check {
        let before = c.clicks("behind")?;
        let at = center(c.recorded("behind")?);
        c.press(at).await?;
        let after = c.clicks("behind")?;
        ensure!(after == before + 1, "a press and release ran the click {} times", after - before);
        ensure!(c.focused_name("behind")?, "the clicked button isn't focused");
        ensure!(c.modality()? == Modality::Pointer, "a pointer press left the modality on the keyboard (a ring would show)");
        Ok(())
    }

    async fn button_space_activates_once(c: &mut Checker) -> Check {
        // A pointer press first, so the key has a modality to change.
        let at = center(c.recorded("behind")?);
        c.press(at).await?;
        ensure!(c.modality()? == Modality::Pointer, "the press didn't set the pointer modality");
        let name = Gallery::button_name(ButtonVariant::Default, ButtonSize::Default);
        c.focus(&name).await?;
        let before = c.clicks(&name)?;
        c.key("space").await?;
        c.key_up("space").await?;
        let after = c.clicks(&name)?;
        ensure!(after == before + 1, "space ran the action {} times", after - before);
        ensure!(c.modality()? == Modality::Keyboard, "space on a button didn't count as keyboard input (no ring would show)");
        c.key("enter").await?;
        c.key_up("enter").await?;
        let after_enter = c.clicks(&name)?;
        ensure!(after_enter == after + 1, "enter ran the action {} times", after_enter - after);
        Ok(())
    }

    /// Space on a select's trigger and in its menu goes through the
    /// select's own key handling, which must count as keyboard input too.
    async fn select_space_keyboard_modality(c: &mut Checker) -> Check {
        let at = center(c.recorded("behind")?);
        c.press(at).await?;
        let shell = shell_focused(c).await?;
        c.key("space").await?;
        c.key_up("space").await?;
        ensure!(is_open(c, &shell)?, "space didn't open the menu");
        ensure!(c.modality()? == Modality::Keyboard, "space on the trigger didn't count as keyboard input");
        c.press(at).await?;
        ensure!(!is_open(c, &shell)?, "the outside press didn't close the menu");
        let trigger = trigger_handle(c, &shell)?;
        ensure!(c.focused(&trigger)?, "focus didn't return to the trigger");
        ensure!(c.modality()? == Modality::Pointer, "the outside press didn't set the pointer modality");
        c.key("down").await?;
        c.press(at).await?;
        // Opened by key, closed by the pointer; now a pick by space.
        c.key("space").await?;
        c.key_up("space").await?;
        c.key("down").await?;
        c.key("space").await?;
        c.key_up("space").await?;
        ensure!(!is_open(c, &shell)?, "space in the menu didn't pick");
        ensure!(selected(c, &shell)? == Some(1), "space didn't pick fish");
        ensure!(c.modality()? == Modality::Keyboard, "a pick by space didn't count as keyboard input");
        Ok(())
    }

    use brindle_ui::{ButtonSize, ButtonVariant};

    async fn button_disabled_inert(c: &mut Checker) -> Check {
        c.focus("behind").await?;
        let at = center(c.reveal("disabled").await?);
        c.press(at).await?;
        ensure!(c.clicks("disabled")? == 0, "a disabled button ran its action");
        ensure!(c.focused_name("behind")?, "pressing a disabled button moved focus");
        Ok(())
    }

    /// The icon-only sizes paint as shadcn's `size-9`, `size-8` and `size-6`
    /// squares: 2.25, 2 and 1.5 rem.
    async fn icon_button_sizes(c: &mut Checker) -> Check {
        let rem = c.with(|_, window, _| f32::from(window.rem_size()))?;
        for (size, rems) in [(ButtonSize::Icon, 2.25), (ButtonSize::IconSm, 2.), (ButtonSize::IconXs, 1.5)] {
            let bounds = c.reveal(&format!("size:{size:?}")).await?;
            let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            let side = rems * rem;
            ensure!(
                (width - side).abs() < 0.5 && (height - side).abs() < 0.5,
                "{size:?} paints {width}x{height}, not {side}x{side} ({rems}rem)"
            );
        }
        Ok(())
    }

    async fn focus_visible(c: &mut Checker) -> Check {
        let at = center(c.recorded("behind")?);
        c.press(at).await?;
        ensure!(c.modality()? == Modality::Pointer, "a press didn't set the pointer modality");
        c.key("tab").await?;
        ensure!(c.modality()? == Modality::Keyboard, "a key didn't set the keyboard modality");
        // Tab moved on, to the control after the pressed button.
        let shell = c.with(|gallery, _, cx| gallery.shell.focus_handle(cx))?;
        ensure!(c.focused(&shell)?, "Tab after pressing a button didn't move focus to the next control");
        Ok(())
    }

    /// Tab from nothing focused reaches the first control and goes on;
    /// shift-Tab goes back and wraps from the first to the last. Without a
    /// focus scope over the gallery Tab moved nothing, so no control could
    /// show a ring for it.
    async fn tab_moves_focus(c: &mut Checker) -> Check {
        c.key("tab").await?;
        ensure!(c.focused_name("theme")?, "Tab with no control focused didn't focus the first control (the Dark switch)");
        c.key("tab").await?;
        ensure!(c.focused_name("open-dialog")?, "a second Tab didn't reach the Open dialog button");
        ensure!(c.modality()? == Modality::Keyboard, "tabbing didn't leave the keyboard modality (no ring would show)");
        c.key("shift-tab").await?;
        ensure!(c.focused_name("theme")?, "shift-Tab didn't go back to the Dark switch");
        c.key("shift-tab").await?;
        ensure!(c.focused_name("corner-3")?, "shift-Tab from the first control didn't wrap to the last");
        Ok(())
    }

    /// The controls whose ring [`ring_paints`] reads, by recorded name, in
    /// Tab order.
    const RING_CONTROLS: [(&str, &str); 6] = [
        ("theme", "a switch"),
        ("open-dialog", "a button"),
        ("command-popover", "a popover trigger"),
        ("shell", "a select trigger"),
        ("name-input", "an input"),
        ("checkbox", "a checkbox"),
    ];

    /// `fg` drawn over `bg`, as 8-bit RGB.
    fn over(fg: gpui::Hsla, bg: gpui::Hsla) -> [u8; 3] {
        let (fg, bg): (gpui::Rgba, gpui::Rgba) = (fg.into(), bg.into());
        let mix = |f: f32, b: f32| ((f * fg.a + b * (1. - fg.a)) * 255.).round() as u8;
        [mix(fg.r, bg.r), mix(fg.g, bg.g), mix(fg.b, bg.b)]
    }

    fn near(a: [u8; 3], b: [u8; 3]) -> bool {
        a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 12)
    }

    /// Tab to each kind of control and read the window just outside its top
    /// edge: the ring's colour while it has focus, the background once Tab
    /// moves on. A ring the platform's renderer doesn't draw (as a box
    /// shadow with no blur wasn't, on Windows) fails here, where reading
    /// the modality alone passed.
    async fn ring_paints(c: &mut Checker) -> Check {
        let (background, ring) = c.with(|_, _, cx| {
            let t = cx.ui_theme();
            (t.background, brindle_ui::focus_ring(t))
        })?;
        let (background, ring) = (over(background, background), over(ring, background));
        for (name, what) in RING_CONTROLS {
            let handle = c.with(move |gallery, _, cx| match name {
                "shell" => gallery.shell.focus_handle(cx),
                "name-input" => gallery.name.focus_handle(cx),
                _ => gallery.handle(name),
            })?;
            let mut tabs = 0;
            while !c.focused(&handle)? {
                ensure!(tabs < 80, "Tab never reached {what} ({name})");
                c.key("tab").await?;
                tabs += 1;
            }
            ensure!(c.modality()? == Modality::Keyboard, "tabbing to {what} left the pointer modality");
            let bounds = c.reveal(name).await?;
            let at = point(bounds.center().x, bounds.top() - px(1.5));
            let shown = c.pixel(at).await?;
            ensure!(
                near(shown, ring),
                "{what} ({name}) focused by Tab shows {shown:?} just above it at {at:?}, not the ring {ring:?} (background {background:?})"
            );
            c.key("tab").await?;
            let after = c.pixel(at).await?;
            ensure!(near(after, background), "the ring stayed on {what} ({name}) after Tab moved on: {after:?}");
        }
        Ok(())
    }

    async fn input_frame_press(c: &mut Checker) -> Check {
        let frame = c.reveal("name-input").await?;
        let input_focused =
            |c: &mut Checker| c.with(|gallery, window, cx| gallery.name.read(cx).focus_handle(cx).is_focused(window));
        // On the text input itself first.
        c.press(frame.center()).await?;
        if !input_focused(c)? {
            let focused = c.with(|_, window, cx| window.focused(cx).map(|handle| format!("{handle:?}")))?;
            fail!("a press on the text input at {:?} in {frame:?} didn't focus it (focused: {focused:?})", frame.center());
        }
        c.with(|_, window, _| window.blur())?;
        c.tick().await;
        // Then in the frame's right padding, past the text input.
        let at = point(frame.right() - px(6.), frame.center().y);
        c.press(at).await?;
        if !input_focused(c)? {
            let focused = c.with(|_, window, cx| window.focused(cx).map(|handle| format!("{handle:?}")))?;
            fail!("a press in the frame's padding at {at:?} in {frame:?} didn't focus the text input (focused: {focused:?})");
        }
        c.key("a").await?;
        let text = c.with(|gallery, _, cx| gallery.name.read(cx).text().to_string())?;
        ensure!(text == "a", "typing after the press gave {text:?}");
        c.with(|gallery, _, cx| gallery.name.update(cx, |input, cx| input.set_text("", cx)))?;
        Ok(())
    }

    async fn checkbox_space(c: &mut Checker) -> Check {
        c.focus("checkbox").await?;
        let before = c.with(|gallery, _, _| gallery.checked)?;
        c.key("space").await?;
        c.key_up("space").await?;
        let after = c.with(|gallery, _, _| gallery.checked)?;
        ensure!(after == !before, "space didn't ask the owner to flip the checkbox");
        c.key("enter").await?;
        c.key_up("enter").await?;
        let after_enter = c.with(|gallery, _, _| gallery.checked)?;
        ensure!(after_enter == after, "enter toggled the checkbox");
        c.key("space").await?;
        c.key_up("space").await?;
        let back = c.with(|gallery, _, _| gallery.checked)?;
        ensure!(back == before, "space again didn't flip it back");
        Ok(())
    }

    async fn switch_space_and_enter(c: &mut Checker) -> Check {
        c.focus("switch").await?;
        let before = c.with(|gallery, _, _| gallery.switched)?;
        c.key("space").await?;
        c.key_up("space").await?;
        let after = c.with(|gallery, _, _| gallery.switched)?;
        ensure!(after == !before, "space didn't ask the owner to flip the switch");
        c.key("space").await?;
        c.key_up("space").await?;
        ensure!(c.with(|gallery, _, _| gallery.switched)? == before, "space again didn't flip it back");
        // Enter too, as Radix's switch (a plain button) takes it, unlike a
        // checkbox.
        c.key("enter").await?;
        c.key_up("enter").await?;
        ensure!(c.with(|gallery, _, _| gallery.switched)? == !before, "enter didn't ask the owner to flip the switch");
        c.key("enter").await?;
        c.key_up("enter").await?;
        ensure!(c.with(|gallery, _, _| gallery.switched)? == before, "enter again didn't flip it back (or flipped twice)");
        Ok(())
    }

    async fn checkbox_disabled(c: &mut Checker) -> Check {
        c.focus("behind").await?;
        let at = center(c.reveal("checkbox-off").await?);
        c.press(at).await?;
        ensure!(c.clicks("checkbox-off")? == 0, "a disabled checkbox asked for a change");
        ensure!(c.focused_name("behind")?, "pressing a disabled checkbox moved focus");
        Ok(())
    }

    // ---- toggle and tooltip ----

    fn pressed(c: &mut Checker, name: &str) -> Result<bool, Failure> {
        let name = name.to_string();
        c.with(move |gallery, _, _| gallery.is_pressed(&name))
    }

    /// Space and enter on a focused toggle each ask the owner to flip it
    /// (Radix's toggle is a button), once per key, as keyboard input.
    async fn toggle_space_and_enter(c: &mut Checker) -> Check {
        c.focus("toggle").await?;
        let before = pressed(c, "toggle")?;
        let asked = c.clicks("toggle")?;
        c.key("space").await?;
        c.key_up("space").await?;
        ensure!(pressed(c, "toggle")? == !before, "space didn't ask the owner to flip the toggle");
        ensure!(c.modality()? == Modality::Keyboard, "space on a toggle didn't count as keyboard input");
        c.key("enter").await?;
        c.key_up("enter").await?;
        ensure!(pressed(c, "toggle")? == before, "enter didn't flip it back (or flipped it twice)");
        let asked = c.clicks("toggle")? - asked;
        ensure!(asked == 2, "two keys asked {asked} times");
        Ok(())
    }

    /// A click flips the toggle and focuses it as a pointer does. A click on
    /// a toggle without a focus handle flips it and leaves focus where it
    /// was, not on the gallery around it.
    async fn toggle_click(c: &mut Checker) -> Check {
        let at = center(c.reveal("toggle").await?);
        let before = pressed(c, "toggle")?;
        c.press(at).await?;
        ensure!(pressed(c, "toggle")? == !before, "a click didn't flip the toggle");
        ensure!(c.focused_name("toggle")?, "the clicked toggle isn't focused");
        ensure!(c.modality()? == Modality::Pointer, "a click left the keyboard modality (a ring would show)");
        let name = format!("toggle:{:?}:{:?}", ToggleVariant::Outline, ToggleSize::Sm);
        let at = center(c.reveal(&name).await?);
        let before = pressed(c, &name)?;
        c.press(at).await?;
        ensure!(pressed(c, &name)? == !before, "a click didn't flip the toggle without a focus handle");
        ensure!(c.focused_name("toggle")?, "a press on a toggle without a focus handle moved focus");
        Ok(())
    }

    /// A disabled toggle tracks no focus and has no key handler: space with
    /// its handle focused asks nothing.
    async fn toggle_disabled_keys(c: &mut Checker) -> Check {
        c.focus("toggle-off").await?;
        c.key("space").await?;
        c.key_up("space").await?;
        c.key("enter").await?;
        c.key_up("enter").await?;
        ensure!(c.clicks("toggle-off")? == 0, "a disabled toggle asked for a change from a key");
        Ok(())
    }

    async fn toggle_disabled_press(c: &mut Checker) -> Check {
        c.focus("behind").await?;
        let at = center(c.reveal("toggle-off").await?);
        c.press(at).await?;
        ensure!(c.clicks("toggle-off")? == 0, "a disabled toggle asked for a change");
        ensure!(c.focused_name("behind")?, "pressing a disabled toggle moved focus");
        Ok(())
    }

    /// Longer than gpui's 500 ms hover delay.
    const TOOLTIP_WAIT: Duration = Duration::from_millis(800);

    /// Where a tooltip shown for a pointer at `at` is sure to paint its
    /// background: gpui places it 1px below and right of the pointer, and
    /// this is inside its padding.
    fn tooltip_point(at: Point<Pixels>) -> Point<Pixels> {
        point(at.x + px(6.), at.y + px(4.))
    }

    /// The tooltip's background as the window shows it.
    fn tooltip_colour(c: &mut Checker) -> Result<[u8; 3], Failure> {
        c.with(|_, _, cx| {
            let t = cx.ui_theme();
            over(brindle_ui::tooltip_style(t).background, t.background)
        })
    }

    /// Rests the pointer at `at` past the hover delay.
    async fn rest(c: &mut Checker, at: Point<Pixels>) -> Result<(), Failure> {
        c.hover(at).await?;
        c.cx.background_executor().timer(TOOLTIP_WAIT).await;
        c.tick().await;
        Ok(())
    }

    /// Moves the pointer to blank space, so no tooltip stays up for the
    /// next check's samples.
    async fn pointer_away(c: &mut Checker) -> Result<(), Failure> {
        let viewport = c.viewport()?;
        c.hover(point(viewport.width - px(40.), viewport.height / 2.)).await?;
        c.tick().await;
        Ok(())
    }

    /// [`pointer_away`] after a tooltip check, whatever its result; the
    /// check's own failure or skip wins over the move's.
    async fn pointer_away_after(c: &mut Checker, check: Check) -> Check {
        let away = pointer_away(c).await;
        check.and(away)
    }

    async fn tooltip_hover_and_press(c: &mut Checker) -> Check {
        let at = center(c.reveal("tip-new").await?);
        let tooltip = tooltip_colour(c)?;
        let sample = tooltip_point(at);
        let before = c.pixel(sample).await?;
        ensure!(!near(before, tooltip), "the tooltip's colour {tooltip:?} shows at {sample:?} before any hover");
        rest(c, at).await?;
        let shown = c.pixel(sample).await?;
        ensure!(near(shown, tooltip), "resting on the button showed {shown:?} at {sample:?}, not the tooltip {tooltip:?}");
        let clicks = c.clicks("tip-new")?;
        c.press(at).await?;
        ensure!(c.clicks("tip-new")? == clicks + 1, "the press under the tooltip didn't click the button");
        let after = c.pixel(sample).await?;
        ensure!(!near(after, tooltip), "the tooltip stayed up after a press ({after:?})");
        Ok(())
    }

    async fn tooltip_escape(c: &mut Checker) -> Check {
        let at = center(c.reveal("tip-find").await?);
        let tooltip = tooltip_colour(c)?;
        let sample = tooltip_point(at);
        rest(c, at).await?;
        let shown = c.pixel(sample).await?;
        ensure!(near(shown, tooltip), "resting on the button showed {shown:?}, not the tooltip {tooltip:?}");
        c.key("escape").await?;
        let after = c.pixel(sample).await?;
        ensure!(!near(after, tooltip), "the tooltip stayed up after Escape ({after:?})");
        Ok(())
    }

    async fn tooltip_disabled(c: &mut Checker) -> Check {
        let at = center(c.reveal("tip-disabled").await?);
        let tooltip = tooltip_colour(c)?;
        let sample = tooltip_point(at);
        rest(c, at).await?;
        let shown = c.pixel(sample).await?;
        ensure!(!near(shown, tooltip), "a disabled button showed a tooltip ({shown:?})");
        Ok(())
    }

    use brindle_ui::{ToggleSize, ToggleVariant};

    // ---- dialog ----

    /// Opens the demo dialog from its button with the keyboard and returns
    /// the view.
    async fn open_dialog(c: &mut Checker, stubborn: bool) -> Result<Entity<super::DemoDialog>, Failure> {
        let opener = if stubborn { "open-stubborn" } else { "open-dialog" };
        c.focus(opener).await?;
        c.key("enter").await?;
        c.key_up("enter").await?;
        let view = c.with(|gallery, _, _| gallery.dialog.view().cloned())?;
        let Some(view) = view else { fail!("enter on {opener} didn't open the dialog") };
        ensure!(c.layers()? == 1, "the dialog isn't the one open layer ({} open)", c.layers()?);
        Ok(view)
    }

    async fn dialog_opens_focusing_input(c: &mut Checker) -> Check {
        let dialog = open_dialog(c, false).await?;
        let name = c.read(&dialog, |dialog, _| dialog.name.clone())?;
        let input = c.read(&name, |input, cx| input.focus_handle(cx))?;
        ensure!(c.focused(&input)?, "the dialog's input isn't focused on open");
        c.key("a").await?;
        c.key("b").await?;
        let text = c.with(move |_, _, cx| name.read(cx).text().to_string())?;
        ensure!(text == "ab", "typing into the opened dialog gave {text:?}");
        Ok(())
    }

    async fn dialog_tab_cycles(c: &mut Checker) -> Check {
        let dialog = open_dialog(c, false).await?;
        let stops = c.with({
            let dialog = dialog.clone();
            move |_, _, cx| {
                let d = dialog.read(cx);
                vec![
                    ("name", d.name.focus_handle(cx)),
                    ("shell", d.shell.focus_handle(cx)),
                    ("cancel", d.cancel.clone()),
                    ("save", d.save.clone()),
                    ("close", d.close.clone()),
                ]
            }
        })?;
        ensure!(c.focused(&stops[0].1)?, "focus doesn't start on the input");
        for (ix, (name, handle)) in stops.iter().enumerate().skip(1) {
            c.key("tab").await?;
            ensure!(c.focused(handle)?, "tab {ix} didn't reach {name}");
        }
        c.key("tab").await?;
        ensure!(c.focused(&stops[0].1)?, "tab from the last stop didn't wrap to the input");
        c.key("shift-tab").await?;
        ensure!(c.focused(&stops[4].1)?, "shift-tab from the first stop didn't wrap to the close button");
        c.key("shift-tab").await?;
        ensure!(c.focused(&stops[3].1)?, "shift-tab didn't move back to save");
        ensure!(c.modality()? == Modality::Keyboard, "tabbing didn't leave the keyboard modality (no ring)");
        Ok(())
    }

    async fn dialog_escape_returns_focus(c: &mut Checker) -> Check {
        open_dialog(c, false).await?;
        c.key("escape").await?;
        ensure!(!c.with(|gallery, _, _| gallery.dialog.is_open())?, "escape didn't close the dialog");
        ensure!(c.layers()? == 0, "a layer is still open");
        ensure!(c.focused_name("open-dialog")?, "focus didn't return to the opener");
        Ok(())
    }

    async fn dialog_backdrop_returns_focus(c: &mut Checker) -> Check {
        open_dialog(c, false).await?;
        let viewport = c.viewport()?;
        c.press(backdrop_point(viewport)).await?;
        ensure!(!c.with(|gallery, _, _| gallery.dialog.is_open())?, "a backdrop press didn't close the dialog");
        ensure!(c.focused_name("open-dialog")?, "focus didn't return to the opener");
        Ok(())
    }

    async fn dialog_blocks_press_beneath(c: &mut Checker) -> Check {
        let behind = c.recorded("behind")?;
        let before = c.clicks("behind")?;
        let dialog = open_dialog(c, true).await?;
        c.press(center(behind)).await?;
        ensure!(c.clicks("behind")? == before, "the button beneath the backdrop ran its action");
        let (open, requests) = c.with({
            let dialog = dialog.clone();
            move |gallery, _, cx| (gallery.dialog.is_open(), dialog.read(cx).dismiss_requests)
        })?;
        ensure!(open, "the stubborn dialog closed on a backdrop press");
        ensure!(requests == 1, "the dialog was asked to dismiss {requests} times, not once");
        Ok(())
    }

    /// Opens the sticky dialog (its slot ignores backdrop presses) from its
    /// button with the keyboard and returns the view.
    async fn open_sticky(c: &mut Checker) -> Result<Entity<super::DemoDialog>, Failure> {
        c.focus("open-sticky").await?;
        c.key("enter").await?;
        c.key_up("enter").await?;
        let view = c.with(|gallery, _, _| gallery.sticky_dialog.view().cloned())?;
        let Some(view) = view else { fail!("enter on open-sticky didn't open the dialog") };
        ensure!(c.layers()? == 1, "the dialog isn't the one open layer ({} open)", c.layers()?);
        Ok(view)
    }

    async fn sticky_backdrop_press_keeps_it_open(c: &mut Checker) -> Check {
        let behind = c.recorded("behind")?;
        let before = c.clicks("behind")?;
        open_sticky(c).await?;
        let focus = c.with(|_, window, cx| window.focused(cx))?;
        ensure!(focus.is_some(), "nothing has focus in the open dialog");
        let viewport = c.viewport()?;
        c.press(backdrop_point(viewport)).await?;
        ensure!(c.with(|gallery, _, _| gallery.sticky_dialog.is_open())?, "a backdrop press closed the dialog");
        ensure!(c.layers()? == 1, "{} layers open, expected the dialog alone", c.layers()?);
        // Focus stays where it was.
        ensure!(c.with(|_, window, cx| window.focused(cx))? == focus, "the backdrop press moved focus");
        // The backdrop still blocks the pointer.
        c.press(center(behind)).await?;
        ensure!(c.clicks("behind")? == before, "the button beneath the backdrop ran its action");
        ensure!(c.with(|gallery, _, _| gallery.sticky_dialog.is_open())?, "the dialog closed");
        Ok(())
    }

    async fn sticky_escape_still_closes(c: &mut Checker) -> Check {
        open_sticky(c).await?;
        c.key("escape").await?;
        ensure!(!c.with(|gallery, _, _| gallery.sticky_dialog.is_open())?, "escape didn't close the dialog");
        ensure!(c.layers()? == 0, "a layer is still open");
        ensure!(c.focused_name("open-sticky")?, "focus didn't return to the opener");
        Ok(())
    }

    async fn sticky_menu_outside_press_closes_menu_only(c: &mut Checker) -> Check {
        let dialog = open_sticky(c).await?;
        let (shell, bounds) = c.read(&dialog, |dialog, _| (dialog.shell.clone(), dialog.shell_bounds.get()))?;
        let Some(trigger) = bounds else { fail!("the dialog's select recorded no painted bounds") };
        c.key("tab").await?;
        c.key("down").await?;
        ensure!(c.layers()? == 2, "the menu didn't open over the dialog");
        // The dialog's own body, in the name row above the select.
        let in_body = Point { x: trigger.origin.x + px(8.), y: trigger.origin.y - px(30.) };
        c.press(in_body).await?;
        ensure!(!is_open(c, &shell)?, "the press in the dialog's body didn't close the menu");
        ensure!(c.with(|gallery, _, _| gallery.sticky_dialog.is_open())?, "the dialog closed with the menu");
        ensure!(c.layers()? == 1, "{} layers open, expected the dialog alone", c.layers()?);
        Ok(())
    }

    async fn dialog_stubborn(c: &mut Checker) -> Check {
        let dialog = open_dialog(c, true).await?;
        c.key("escape").await?;
        let (open, warned) = c.with({
            let dialog = dialog.clone();
            move |gallery, _, cx| (gallery.dialog.is_open(), dialog.read(cx).warned)
        })?;
        ensure!(open, "escape closed a dialog that handles Dismiss itself");
        ensure!(warned, "the dialog didn't show its warning");
        // Discard is now a stop; it closes the dialog.
        let discard = c.with({
            let dialog = dialog.clone();
            move |_, _, cx| dialog.read(cx).discard.clone()
        })?;
        c.focus_handle(&discard).await?;
        c.key("enter").await?;
        c.key_up("enter").await?;
        ensure!(!c.with(|gallery, _, _| gallery.dialog.is_open())?, "Discard didn't close the dialog");
        ensure!(c.focused_name("open-stubborn")?, "focus didn't return to the opener");
        Ok(())
    }

    // ---- select ----

    /// Focuses the gallery's shell select, with `bash` selected.
    async fn shell_focused(c: &mut Checker) -> Result<Entity<Select>, Failure> {
        let shell = c.with(|gallery, _, cx| {
            gallery.shell.update(cx, |select, cx| select.set_selected(Some(0), cx));
            gallery.shell.clone()
        })?;
        let handle = trigger_handle(c, &shell)?;
        c.focus_handle(&handle).await?;
        Ok(shell)
    }

    fn trigger_handle(c: &mut Checker, select: &Entity<Select>) -> Result<FocusHandle, Failure> {
        c.read(select, |select, cx| select.focus_handle(cx))
    }

    fn highlighted(c: &mut Checker, select: &Entity<Select>) -> Result<Option<usize>, Failure> {
        let select = select.clone();
        c.with(move |_, _, cx| select.read(cx).highlighted(cx))
    }

    fn selected(c: &mut Checker, select: &Entity<Select>) -> Result<Option<usize>, Failure> {
        let select = select.clone();
        c.with(move |_, _, cx| select.read(cx).selected())
    }

    fn is_open(c: &mut Checker, select: &Entity<Select>) -> Result<bool, Failure> {
        let select = select.clone();
        c.with(move |_, _, cx| select.read(cx).is_open())
    }

    fn changes(c: &mut Checker) -> Result<Vec<usize>, Failure> {
        c.with(|gallery, _, _| gallery.shell_changes.clone())
    }

    async fn select_opens_on_down(c: &mut Checker) -> Check {
        let shell = shell_focused(c).await?;
        c.key("down").await?;
        ensure!(is_open(c, &shell)?, "down didn't open the menu");
        ensure!(c.layers()? == 1, "the menu isn't a layer");
        ensure!(highlighted(c, &shell)? == Some(0), "the selection (bash) isn't highlighted");
        Ok(())
    }

    /// The closed trigger's key table, as in Radix: enter, space, up and
    /// down open the menu on the selected option; left, right, home and end
    /// do nothing (no menu, no change, focus stays on the trigger). The
    /// selection is fish, so "on the selected option" isn't the first one.
    async fn select_closed_trigger_keys(c: &mut Checker) -> Check {
        let shell = shell_focused(c).await?;
        c.with(|gallery, _, cx| gallery.shell.update(cx, |select, cx| select.set_selected(Some(1), cx)))?;
        let trigger = trigger_handle(c, &shell)?;
        let before = changes(c)?;
        for (key, opens) in [
            ("enter", true),
            ("space", true),
            ("up", true),
            ("down", true),
            ("left", false),
            ("right", false),
            ("home", false),
            ("end", false),
        ] {
            c.focus_handle(&trigger).await?;
            c.key(key).await?;
            let open = is_open(c, &shell)?;
            ensure!(open == opens, "{key}: the menu is {}", if open { "open" } else { "closed" });
            if opens {
                let highlight = highlighted(c, &shell)?;
                ensure!(highlight == Some(1), "{key}: the menu highlights {highlight:?}, not the selection (fish)");
                c.key("escape").await?;
                ensure!(!is_open(c, &shell)?, "escape after {key} didn't close the menu");
            } else {
                ensure!(c.focused(&trigger)?, "{key} moved focus off the trigger");
            }
            ensure!(selected(c, &shell)? == Some(1), "{key} changed the selection");
            ensure!(changes(c)? == before, "{key}: the owner heard {:?}", &changes(c)?[before.len()..]);
        }
        Ok(())
    }

    /// Radix's pointer: the trigger opens on the press, and a release that
    /// didn't move (over the trigger) leaves the menu open on the selection.
    /// A second press, on the trigger the menu's layer now covers, is an
    /// outside press: it closes the menu and doesn't reopen it.
    async fn select_press_opens_and_release_on_trigger_stays(c: &mut Checker) -> Check {
        let shell = shell_focused(c).await?;
        let before = changes(c)?;
        let at = center(c.recorded("shell")?);
        c.press(at).await?;
        ensure!(is_open(c, &shell)?, "a press and release on the trigger didn't leave the menu open");
        ensure!(highlighted(c, &shell)? == Some(0), "the menu doesn't highlight the selection (bash)");
        c.press(at).await?;
        ensure!(!is_open(c, &shell)?, "a press on the trigger of an open menu didn't close it");
        ensure!(c.layers()? == 0, "{} layers open after the second press", c.layers()?);
        ensure!(selected(c, &shell)? == Some(0) && changes(c)? == before, "the presses changed the selection");
        let trigger = trigger_handle(c, &shell)?;
        ensure!(c.focused(&trigger)?, "focus didn't return to the trigger");
        Ok(())
    }

    /// One gesture: press on the trigger, drag onto foot, release. The
    /// row is found first with the menu opened by key, since a search by
    /// hovering would need the button up.
    async fn select_press_drag_release_picks(c: &mut Checker) -> Check {
        let shell = shell_focused(c).await?;
        let before = changes(c)?.len();
        let trigger = c.recorded("shell")?;
        c.key("down").await?;
        let row = c.option_row(&shell, trigger, 2).await?;
        c.key("escape").await?;
        ensure!(!is_open(c, &shell)?, "escape didn't close the menu opened to find the row");
        c.drag(center(trigger), row).await?;
        ensure!(!is_open(c, &shell)?, "the release over foot didn't close the menu");
        ensure!(selected(c, &shell)? == Some(2), "the gesture didn't pick foot");
        let changes = changes(c)?;
        ensure!(changes[before..] == [2], "the owner heard {:?}", &changes[before..]);
        let handle = trigger_handle(c, &shell)?;
        ensure!(c.focused(&handle)?, "focus didn't return to the trigger");
        Ok(())
    }

    /// The gesture dragged off the menu onto a button and released there:
    /// the menu closes without a change, and the button isn't clicked. As
    /// in Radix, the same goes for a release over the trigger itself once
    /// the pointer moved more than 10px.
    async fn select_drag_off_and_release_closes(c: &mut Checker) -> Check {
        let shell = shell_focused(c).await?;
        let before = changes(c)?;
        let clicks = c.clicks("behind")?;
        let bounds = c.recorded("shell")?;
        let behind = center(c.recorded("behind")?);
        c.drag(center(bounds), behind).await?;
        ensure!(!is_open(c, &shell)?, "the release away from the menu didn't close it");
        ensure!(selected(c, &shell)? == Some(0) && changes(c)? == before, "the release changed the selection");
        ensure!(c.clicks("behind")? == clicks, "the release clicked the button under it");
        // Along the trigger, well past 10px, and released on it.
        let y = bounds.center().y;
        let (from, to) = (point(bounds.left() + px(8.), y), point(bounds.right() - px(8.), y));
        ensure!(to.x - from.x > px(20.), "the trigger is too narrow to drag along");
        c.drag(from, to).await?;
        ensure!(!is_open(c, &shell)?, "a dragged release over the trigger didn't close the menu");
        ensure!(selected(c, &shell)? == Some(0) && changes(c)? == before, "the release changed the selection");
        Ok(())
    }

    /// While the menu is open its layer blocks the pointer outside it: a
    /// press on a button only closes the menu (Radix's
    /// `disableOutsidePointerEvents`).
    async fn select_outside_press_reaches_nothing(c: &mut Checker) -> Check {
        let shell = shell_focused(c).await?;
        c.key("down").await?;
        ensure!(is_open(c, &shell)?, "down didn't open the menu");
        let before = c.clicks("behind")?;
        let at = center(c.recorded("behind")?);
        c.press(at).await?;
        ensure!(!is_open(c, &shell)?, "the outside press didn't close the menu");
        ensure!(c.clicks("behind")? == before, "the outside press clicked the button beneath");
        let trigger = trigger_handle(c, &shell)?;
        ensure!(c.focused(&trigger)?, "focus didn't return to the trigger");
        ensure!(!c.focused_name("behind")?, "the button beneath took focus");
        Ok(())
    }

    async fn select_nav_skips_disabled(c: &mut Checker) -> Check {
        let shell = shell_focused(c).await?;
        c.key("down").await?;
        // bash, fish, foot, zsh (disabled), nu
        for (key, expected) in [("down", 1), ("down", 2), ("down", 4), ("down", 4), ("home", 0), ("end", 4), ("up", 2)] {
            c.key(key).await?;
            let highlight = highlighted(c, &shell)?;
            ensure!(highlight == Some(expected), "{key} highlighted {highlight:?}, expected {expected}");
        }
        Ok(())
    }

    async fn select_enter_picks(c: &mut Checker) -> Check {
        let shell = shell_focused(c).await?;
        let before = changes(c)?.len();
        c.key("down").await?;
        c.key("down").await?;
        ensure!(highlighted(c, &shell)? == Some(1), "down didn't highlight fish");
        c.key("enter").await?;
        ensure!(!is_open(c, &shell)?, "enter didn't close the menu");
        ensure!(selected(c, &shell)? == Some(1), "fish isn't selected");
        let changes = changes(c)?;
        ensure!(changes.len() == before + 1 && changes.last() == Some(&1), "the owner heard {:?}", &changes[before..]);
        let trigger = trigger_handle(c, &shell)?;
        ensure!(c.focused(&trigger)?, "the trigger isn't focused after the pick");
        Ok(())
    }

    async fn select_enter_keyup(c: &mut Checker) -> Check {
        let shell = shell_focused(c).await?;
        c.key("down").await?;
        c.key("enter").await?;
        ensure!(!is_open(c, &shell)?, "enter didn't close the menu");
        if !c.key_up("enter").await? {
            return Err(Failure::Skip(NO_KEY_UP.to_string()));
        }
        ensure!(!is_open(c, &shell)?, "the enter key-up reopened the menu through a keyboard click");
        ensure!(c.layers()? == 0, "a layer is open after the key-up");
        Ok(())
    }

    async fn select_rechoose_same(c: &mut Checker) -> Check {
        let shell = shell_focused(c).await?;
        let before = changes(c)?;
        c.key("down").await?;
        let trigger = c.recorded("shell")?;
        let at = c.option_row(&shell, trigger, 0).await?;
        c.press(at).await?;
        ensure!(!is_open(c, &shell)?, "clicking the selected option didn't close the menu");
        ensure!(selected(c, &shell)? == Some(0), "the selection changed");
        ensure!(changes(c)? == before, "the owner was told of a change");
        Ok(())
    }

    async fn select_escape(c: &mut Checker) -> Check {
        let shell = shell_focused(c).await?;
        let before = changes(c)?;
        c.key("down").await?;
        c.key("down").await?;
        c.key("escape").await?;
        ensure!(!is_open(c, &shell)?, "escape didn't close the menu");
        ensure!(selected(c, &shell)? == Some(0), "escape changed the selection");
        ensure!(changes(c)? == before, "the owner was told of a change");
        let trigger = trigger_handle(c, &shell)?;
        ensure!(c.focused(&trigger)?, "the trigger isn't focused after escape");
        Ok(())
    }

    async fn select_typeahead_open(c: &mut Checker) -> Check {
        let shell = shell_focused(c).await?;
        c.key("down").await?;
        c.key("f").await?;
        ensure!(highlighted(c, &shell)? == Some(1), "f didn't highlight fish");
        c.cx.background_executor().timer(TYPEAHEAD_GAP).await;
        c.key("f").await?;
        ensure!(highlighted(c, &shell)? == Some(2), "f again, later, didn't cycle to foot");
        c.key("escape").await?;
        Ok(())
    }

    async fn select_typeahead_closed(c: &mut Checker) -> Check {
        let shell = shell_focused(c).await?;
        let before = changes(c)?.len();
        c.key("f").await?;
        ensure!(!is_open(c, &shell)?, "typing opened the menu");
        ensure!(selected(c, &shell)? == Some(1), "f didn't select fish");
        c.cx.background_executor().timer(TYPEAHEAD_GAP).await;
        c.key("f").await?;
        ensure!(selected(c, &shell)? == Some(2), "f again, later, didn't select foot");
        let changes = changes(c)?;
        ensure!(changes[before..] == [1, 2], "the owner heard {:?}", &changes[before..]);
        Ok(())
    }

    async fn select_in_dialog_escape(c: &mut Checker) -> Check {
        let dialog = open_dialog(c, false).await?;
        let shell = c.read(&dialog, |dialog, _| dialog.shell.clone())?;
        c.key("tab").await?;
        let trigger = trigger_handle(c, &shell)?;
        ensure!(c.focused(&trigger)?, "tab didn't reach the dialog's select");
        c.key("down").await?;
        ensure!(is_open(c, &shell)?, "down didn't open the menu in the dialog");
        ensure!(c.layers()? == 2, "the menu isn't a second layer ({} open)", c.layers()?);
        c.key("escape").await?;
        ensure!(!is_open(c, &shell)?, "escape didn't close the menu");
        ensure!(c.with(|gallery, _, _| gallery.dialog.is_open())?, "escape closed the dialog too");
        ensure!(c.layers()? == 1, "the dialog isn't the one layer left");
        ensure!(c.focused(&trigger)?, "focus didn't return to the select's trigger");
        c.key("escape").await?;
        ensure!(!c.with(|gallery, _, _| gallery.dialog.is_open())?, "a second escape didn't close the dialog");
        Ok(())
    }

    async fn select_in_dialog_backdrop(c: &mut Checker) -> Check {
        let dialog = open_dialog(c, false).await?;
        let shell = c.read(&dialog, |dialog, _| dialog.shell.clone())?;
        c.key("tab").await?;
        c.key("down").await?;
        ensure!(c.layers()? == 2, "the menu didn't open over the dialog");
        let viewport = c.viewport()?;
        c.press(backdrop_point(viewport)).await?;
        ensure!(!is_open(c, &shell)?, "the backdrop press didn't close the menu");
        ensure!(c.with(|gallery, _, _| gallery.dialog.is_open())?, "the backdrop press closed the dialog too");
        ensure!(c.layers()? == 1, "{} layers open, expected the dialog alone", c.layers()?);
        Ok(())
    }

    async fn select_in_dialog_option_press(c: &mut Checker) -> Check {
        let dialog = open_dialog(c, false).await?;
        let (shell, bounds) = c.read(&dialog, |dialog, _| (dialog.shell.clone(), dialog.shell_bounds.get()))?;
        let Some(trigger_bounds) = bounds else { fail!("the dialog's select recorded no painted bounds") };
        c.key("tab").await?;
        c.key("down").await?;
        let trigger = trigger_handle(c, &shell)?;
        let at = c.option_row(&shell, trigger_bounds, 2).await?;
        c.press(at).await?;
        ensure!(!is_open(c, &shell)?, "the press didn't close the menu");
        ensure!(selected(c, &shell)? == Some(2), "foot wasn't chosen");
        ensure!(c.with(|gallery, _, _| gallery.dialog.is_open())?, "the dialog closed");
        ensure!(c.focused(&trigger)?, "focus didn't return to the trigger");
        Ok(())
    }

    // ---- popover ----

    async fn popover_corner(c: &mut Checker, corner: usize) -> Check {
        let name = format!("corner-{corner}");
        c.focus(&name).await?;
        c.key("enter").await?;
        c.key_up("enter").await?;
        ensure!(c.layers()? == 1, "enter on the trigger didn't open the popover ({} layers)", c.layers()?);
        let panel = c.with(move |gallery, _, _| gallery.corners[corner].get())?;
        let Some(panel) = panel else { fail!("the popover's panel recorded no bounds") };
        let trigger = c.recorded(&name)?;
        let viewport = c.viewport()?;
        let margin = px(8.);
        ensure!(
            panel.left() >= margin
                && panel.top() >= margin
                && panel.right() <= viewport.width - margin
                && panel.bottom() <= viewport.height - margin,
            "the panel {panel:?} leaves the window less its margin ({viewport:?})"
        );
        if corner < 2 {
            ensure!(panel.top() >= trigger.bottom(), "a top-edge popover {panel:?} isn't below its trigger {trigger:?}");
        } else {
            ensure!(panel.bottom() <= trigger.top(), "a bottom-edge popover {panel:?} didn't flip above its trigger {trigger:?}");
        }
        ensure!(c.modality()? == Modality::Keyboard, "opening by key didn't keep the keyboard modality");
        c.key("escape").await?;
        ensure!(c.layers()? == 0, "escape didn't close the popover");
        ensure!(c.focused_name(&name)?, "focus didn't return to the trigger");
        Ok(())
    }

    async fn popover_outside_press(c: &mut Checker) -> Check {
        c.focus("corner-0").await?;
        c.key("enter").await?;
        c.key_up("enter").await?;
        ensure!(c.layers()? == 1, "the popover didn't open");
        let before = c.clicks("behind")?;
        let at = center(c.recorded("behind")?);
        c.press(at).await?;
        ensure!(c.layers()? == 0, "an outside press didn't close the popover");
        ensure!(c.clicks("behind")? == before + 1, "the pressed control didn't run its click");
        ensure!(c.focused_name("corner-0")?, "focus didn't return to the trigger (design.md: the restore is unconditional)");
        Ok(())
    }

    async fn popover_trigger_toggles(c: &mut Checker) -> Check {
        let at = center(c.recorded("corner-1")?);
        c.press(at).await?;
        ensure!(c.layers()? == 1, "a press on the trigger didn't open the popover");
        c.press(at).await?;
        ensure!(c.layers()? == 0, "a press on the trigger didn't close the open popover (or reopened it on release)");
        c.tick().await;
        ensure!(c.layers()? == 0, "the popover reopened after the release");
        c.press(at).await?;
        ensure!(c.layers()? == 1, "a third press didn't open it again");
        Ok(())
    }

    async fn popover_close_button(c: &mut Checker) -> Check {
        c.focus("corner-2").await?;
        c.key("enter").await?;
        c.key_up("enter").await?;
        ensure!(c.layers()? == 1, "the popover didn't open");
        // The content's first stop, its Close button, has focus.
        let trigger = c.handle("corner-2")?;
        ensure!(!c.focused(&trigger)?, "focus stayed on the trigger instead of moving into the popover");
        c.key("enter").await?;
        c.key_up("enter").await?;
        ensure!(c.layers()? == 0, "enter on the content's Close didn't close the popover, or the key-up reopened it");
        ensure!(c.focused(&trigger)?, "focus didn't return to the trigger");
        Ok(())
    }

    // ---- command ----

    async fn command_focused(c: &mut Checker) -> Result<Entity<brindle_ui::Command>, Failure> {
        let command = c.with(|gallery, _, cx| {
            gallery.command.update(cx, |command, cx| command.set_query("", cx));
            gallery.command.clone()
        })?;
        let handle = c.read(&command, |command, cx| command.focus_handle(cx))?;
        c.focus_handle(&handle).await?;
        Ok(command)
    }

    async fn command_filter(c: &mut Checker) -> Check {
        let command = command_focused(c).await?;
        for key in ["s", "p", "r"] {
            c.key(key).await?;
        }
        let (query, matches, highlighted) = c.with(move |_, _, cx| {
            let command = command.read(cx);
            (command.query(cx), command.rows().matches().to_vec(), command.highlighted())
        })?;
        ensure!(query == "spr", "the filter reads {query:?}");
        ensure!(matches == [2], "the matches are {matches:?}, expected Split Right alone");
        ensure!(highlighted == Some(2), "Split Right isn't highlighted");
        Ok(())
    }

    async fn command_nothing_matches(c: &mut Checker) -> Check {
        let command = command_focused(c).await?;
        for key in ["z", "z", "z"] {
            c.key(key).await?;
        }
        let (empty, rows) = c.with(move |_, _, cx| {
            let rows = command.read(cx).rows();
            (rows.is_empty(), rows.rows().len())
        })?;
        ensure!(empty && rows == 0, "nothing should match, but {rows} rows are listed");
        Ok(())
    }

    async fn command_wraps(c: &mut Checker) -> Check {
        let command = command_focused(c).await?;
        let read = |c: &mut Checker| {
            let command = command.clone();
            c.with(move |_, _, cx| command.read(cx).highlighted())
        };
        ensure!(read(c)? == Some(0), "the first item isn't highlighted at the start");
        // 0 New Tab, 1 Close Tab, 2 Split Right, 3 Split Down, 4 Attach (disabled), 5 Reload Config.
        for (key, expected) in [("down", 1), ("down", 2), ("down", 3), ("down", 5), ("down", 0), ("up", 5), ("ctrl-p", 3), ("ctrl-n", 5)] {
            c.key(key).await?;
            let highlight = read(c)?;
            ensure!(highlight == Some(expected), "{key} highlighted {highlight:?}, expected {expected}");
        }
        Ok(())
    }

    /// Reads the inline command list's pixels: the line under the filter
    /// is drawn, and the highlighted row's background reaches across the
    /// list to the group's padding at the right, not just past its text,
    /// while the padding at the left stays clear.
    async fn command_rows_fill_the_width(c: &mut Checker) -> Check {
        let command = c.with(|gallery, _, cx| {
            gallery.command.update(cx, |command, cx| command.set_query("", cx));
            gallery.command.clone()
        })?;
        ensure!(c.read(&command, |command, _| command.highlighted())? == Some(0), "New Tab isn't highlighted");
        let (popover, accent, border) = c.with(|_, _, cx| {
            let t = cx.ui_theme();
            (t.popover, t.accent, t.border)
        })?;
        let (accent, border, panel) = (over(accent, popover), over(border, popover), over(popover, popover));
        let close = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 2);
        let frame = c.reveal("command").await?;
        // The frame's 1px border, then the filter row: h-9 with a 1px
        // border at its bottom.
        let under_filter = point(frame.center().x, frame.top() + px(1. + 35.5));
        let line = c.pixel(under_filter).await?;
        ensure!(close(line, border), "no border under the filter: {line:?} at {under_filter:?}, the border is {border:?}");
        // Down the list, near the right edge and inside the left padding.
        let (right, left) = (frame.right() - px(10.), frame.left() + px(3.));
        let ys: Vec<Pixels> = (0..40).map(|step| frame.top() + px(40. + 2. * step as f32)).collect();
        let points: Vec<_> = ys.iter().flat_map(|&y| [point(right, y), point(left, y)]).collect();
        let shown = c.pixels(&points).await?;
        for (at, colour) in points.iter().zip(&shown).skip(1).step_by(2) {
            ensure!(!close(*colour, accent) || accent == panel, "the highlight reaches into the group's padding at {at:?}");
        }
        let highlight_right = shown.iter().step_by(2).any(|colour| close(*colour, accent));
        ensure!(highlight_right, "the highlighted row's background doesn't reach the list's right side (x = {right:?})");
        Ok(())
    }

    /// Reads the inline command list's pixels: below the highlighted New
    /// Tab, the separator between Tabs and Panes is a thin line in the
    /// border colour that reaches into the groups' padding at both sides
    /// (shadcn's `-mx-1`), where the highlight stops short.
    async fn command_separator_spans_the_list(c: &mut Checker) -> Check {
        let command = c.with(|gallery, _, cx| {
            gallery.command.update(cx, |command, cx| command.set_query("", cx));
            gallery.command.clone()
        })?;
        let rows = c.read(&command, |command, _| command.rows().rows().to_vec())?;
        ensure!(rows.contains(&brindle_ui::CommandRow::Separator(1)), "no separator is listed above Panes: {rows:?}");
        ensure!(c.read(&command, |command, _| command.highlighted())? == Some(0), "New Tab isn't highlighted");
        let (popover, accent, border) = c.with(|_, _, cx| {
            let t = cx.ui_theme();
            (t.popover, t.accent, t.border)
        })?;
        let (accent, border, panel) = (over(accent, popover), over(border, popover), over(popover, popover));
        let close = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 2);
        let frame = c.reveal("command").await?;
        // Every device row from below the filter's line (the frame's 1px
        // border, then h-9 with its own border at the bottom), at the
        // list's centre and 3px in from each side of the frame: 2px into
        // the groups' 4px p-1.
        let scale = c.scale;
        let top = frame.top() + px(1. + 36. + 1.);
        let ys: Vec<Pixels> = (0..(200. * scale) as usize).map(|step| top + px(step as f32 / scale)).collect();
        let (left, centre, right) = (frame.left() + px(3.), frame.center().x, frame.right() - px(3.));
        let points: Vec<_> = ys.iter().flat_map(|&y| [point(left, y), point(centre, y), point(right, y)]).collect();
        let shown = c.pixels(&points).await?;
        let lines: Vec<&[[u8; 3]]> = shown.chunks(3).collect();
        let Some(highlight) = lines.iter().position(|line| close(line[1], accent)) else {
            fail!("the highlighted New Tab wasn't found at x = {centre:?} below {top:?}")
        };
        let border_across = |line: &&[[u8; 3]]| line.iter().all(|&colour| close(colour, border));
        let found = lines.iter().enumerate().skip(highlight).find(|(_, line)| border_across(line));
        let Some((at, _)) = found else {
            let seen: Vec<_> = lines[highlight..]
                .iter()
                .map(|line| line[1])
                .filter(|&colour| !close(colour, panel) && !close(colour, accent))
                .take(8)
                .collect();
            fail!(
                "below New Tab no device row is the border colour {border:?} at both sides and the centre; colours at the centre other than the panel's: {seen:?}"
            )
        };
        // A line, not a box: 2px above and below it, the groups' padding
        // shows the panel.
        let margin = (2. * scale).round() as usize;
        let (above, below) = (at.checked_sub(margin).and_then(|row| lines.get(row)), lines.get(at + margin));
        for (side, line) in [("above", above), ("below", below)] {
            let colour = line.map(|line| line[1]);
            ensure!(
                colour.is_some_and(|colour| close(colour, panel)),
                "2px {side} the separator at {:?} the centre is {colour:?}, not the panel {panel:?}",
                ys[at]
            );
        }
        Ok(())
    }

    async fn command_enter_and_escape_inline(c: &mut Checker) -> Check {
        command_focused(c).await?;
        for key in ["s", "p", "r"] {
            c.key(key).await?;
        }
        c.key("enter").await?;
        let log = c.with(|gallery, _, _| gallery.command_log.clone())?;
        ensure!(log.last().map(String::as_str) == Some("confirmed:Split Right"), "enter reported {:?}", log.last());
        c.key("escape").await?;
        let log = c.with(|gallery, _, _| gallery.command_log.clone())?;
        ensure!(log.last().map(String::as_str) == Some("cancelled"), "escape reported {:?}", log.last());
        ensure!(c.layers()? == 0, "an inline command list opened a layer");
        Ok(())
    }

    async fn command_dialog(c: &mut Checker) -> Check {
        c.focus("open-command-dialog").await?;
        c.key("enter").await?;
        c.key_up("enter").await?;
        let host = c.with(|gallery, _, _| gallery.command_dialog.view().cloned())?;
        let Some(host) = host else { fail!("enter didn't open the command dialog") };
        let command = c.read(&host, |host, _| host.command.clone())?;
        let filter = c.read(&command, |command, cx| command.focus_handle(cx))?;
        ensure!(c.focused(&filter)?, "the command's filter isn't focused in the dialog");
        c.key("c").await?;
        c.key("l").await?;
        let query = c.with({
            let command = command.clone();
            move |_, _, cx| command.read(cx).query(cx)
        })?;
        ensure!(query == "cl", "typing gave {query:?}");
        c.key("escape").await?;
        ensure!(!c.with(|gallery, _, _| gallery.command_dialog.is_open())?, "escape (Cancelled) didn't close the dialog");
        ensure!(c.focused_name("open-command-dialog")?, "focus didn't return to the opener");
        // Again, confirming this time.
        let before = c.with(|gallery, _, _| gallery.confirmed.borrow().len())?;
        c.key("enter").await?;
        c.key_up("enter").await?;
        ensure!(c.with(|gallery, _, _| gallery.command_dialog.is_open())?, "enter didn't reopen the command dialog");
        for key in ["s", "p", "r"] {
            c.key(key).await?;
        }
        c.key("enter").await?;
        ensure!(!c.with(|gallery, _, _| gallery.command_dialog.is_open())?, "enter didn't close the dialog");
        let confirmed = c.with(|gallery, _, _| gallery.confirmed.borrow().clone())?;
        ensure!(confirmed.len() == before + 1 && confirmed.last().map(String::as_str) == Some("Split Right"), "the host recorded {confirmed:?}");
        if c.key_up("enter").await? {
            ensure!(!c.with(|gallery, _, _| gallery.command_dialog.is_open())?, "the enter key-up reopened the dialog");
        }
        Ok(())
    }

    /// gpui's keyboard click fires on key-up for whatever is focused then.
    /// Enter in the command (key-down) confirms, the host closes and focus
    /// returns to the opener button; the key-up must not click it open again.
    async fn command_popover_hazard(c: &mut Checker) -> Check {
        c.focus("command-popover").await?;
        c.key("enter").await?;
        c.key_up("enter").await?;
        ensure!(c.layers()? == 1, "enter on the trigger didn't open the command popover ({} layers)", c.layers()?);
        ensure!(!c.focused_name("command-popover")?, "focus stayed on the trigger instead of the filter");
        for key in ["s", "p", "r"] {
            c.key(key).await?;
        }
        let before = c.with(|gallery, _, _| gallery.confirmed.borrow().len())?;
        c.key("enter").await?;
        let confirmed = c.with(|gallery, _, _| gallery.confirmed.borrow().clone())?;
        ensure!(confirmed.len() == before + 1, "enter didn't confirm ({confirmed:?})");
        ensure!(c.layers()? == 0, "the host didn't close on Confirmed");
        ensure!(c.focused_name("command-popover")?, "focus didn't return to the trigger");
        if !c.key_up("enter").await? {
            return Err(Failure::Skip(NO_KEY_UP.to_string()));
        }
        c.tick().await;
        ensure!(c.layers()? == 0, "the enter key-up clicked the trigger and reopened the popover");
        Ok(())
    }

    // ---- menus ----

    /// The gallery's menus.
    #[derive(Clone, Copy, Debug)]
    enum Which {
        Context,
        Dropdown,
        Long,
    }

    /// The open menu of `which`, if one is open.
    fn open_menu(c: &mut Checker, which: Which) -> Result<Option<Entity<Menu>>, Failure> {
        c.with(move |gallery, _, cx| match which {
            Which::Context => gallery.context_menu.read(cx).menu().cloned(),
            Which::Dropdown => gallery.dropdown.read(cx).menu().cloned(),
            Which::Long => gallery.long_dropdown.read(cx).menu().cloned(),
        })
    }

    fn the_menu(c: &mut Checker, which: Which) -> Result<Entity<Menu>, Failure> {
        open_menu(c, which)?.ok_or_else(|| Failure::Fail(format!("the {which:?} menu isn't open")))
    }

    fn menu_highlight(c: &mut Checker, which: Which) -> Result<Option<usize>, Failure> {
        let menu = the_menu(c, which)?;
        c.read(&menu, |menu, _| menu.highlighted())
    }

    /// The open menu's panel, as last painted.
    fn menu_bounds(c: &mut Checker, which: Which) -> Result<Bounds<Pixels>, Failure> {
        let menu = the_menu(c, which)?;
        c.read(&menu, |menu, _| menu.bounds())?
            .ok_or_else(|| Failure::Fail(format!("the {which:?} menu painted no bounds")))
    }

    /// Where entry `ix` of the open menu was painted, scrolled as shown.
    fn entry_bounds(c: &mut Checker, which: Which, ix: usize) -> Result<Bounds<Pixels>, Failure> {
        let menu = the_menu(c, which)?;
        c.read(&menu, move |menu, _| menu.entry_bounds(ix))?
            .ok_or_else(|| Failure::Fail(format!("entry {ix} of the {which:?} menu painted no bounds")))
    }

    fn menu_log(c: &mut Checker) -> Result<Vec<String>, Failure> {
        c.with(|gallery, _, _| gallery.menu_log.clone())
    }

    /// The trigger bounds a dropdown places its menu against.
    fn dropdown_trigger(c: &mut Checker, which: Which) -> Result<Bounds<Pixels>, Failure> {
        c.with(move |gallery, _, cx| match which {
            Which::Long => gallery.long_dropdown.read(cx).trigger_bounds(),
            _ => gallery.dropdown.read(cx).trigger_bounds(),
        })?
        .ok_or_else(|| Failure::Fail(format!("the {which:?} dropdown's trigger painted no bounds")))
    }

    /// Within a pixel, for positions that went through a message's whole
    /// physical pixels.
    fn close_to(a: Pixels, b: Pixels) -> bool {
        (a - b).abs() <= px(1.)
    }

    fn inside(bounds: Bounds<Pixels>, viewport: gpui::Size<Pixels>, margin: Pixels) -> bool {
        bounds.left() >= margin - px(0.5)
            && bounds.top() >= margin - px(0.5)
            && bounds.right() <= viewport.width - margin + px(0.5)
            && bounds.bottom() <= viewport.height - margin + px(0.5)
    }

    /// Whether `inner` lies within `outer` vertically: a row in a menu's view.
    fn in_view(inner: Bounds<Pixels>, outer: Bounds<Pixels>) -> bool {
        inner.top() >= outer.top() - px(0.5) && inner.bottom() <= outer.bottom() + px(0.5)
    }

    /// Opens the context menu at `at` from code, and lets it paint.
    async fn open_context_at(c: &mut Checker, at: Point<Pixels>) -> Check {
        c.with(move |gallery, window, cx| gallery.open_context_menu(at, window, cx))?;
        c.tick().await;
        c.tick().await;
        ensure!(c.layers()? == 1, "the context menu isn't the one layer ({} open)", c.layers()?);
        Ok(())
    }

    /// The spec's scenario, with the press stood in for by code: the corner
    /// lands on (200, 40), the menu has focus and highlights nothing, and
    /// the first down highlights the first enabled item.
    async fn context_menu_corner(c: &mut Checker) -> Check {
        let at = point(px(200.), px(40.));
        open_context_at(c, at).await?;
        let bounds = menu_bounds(c, Which::Context)?;
        ensure!(
            close_to(bounds.left(), at.x) && close_to(bounds.top(), at.y),
            "the menu's corner is at {:?}, not {at:?}",
            bounds.origin
        );
        let highlight = menu_highlight(c, Which::Context)?;
        ensure!(highlight.is_none(), "the menu opened highlighting {highlight:?}");
        let menu = the_menu(c, Which::Context)?;
        let own = c.read(&menu, |menu, cx| menu.focus_handle(cx))?;
        ensure!(c.focused(&own)?, "the open menu doesn't have focus");
        c.key("down").await?;
        let highlight = menu_highlight(c, Which::Context)?;
        ensure!(highlight == Some(0), "the first down highlighted {highlight:?}, not Rename Tab");
        Ok(())
    }

    /// Near the right edge the menu opens to the left of the pointer; near
    /// the bottom it is shifted up; it stays inside the window less its
    /// margin either way.
    async fn context_menu_edges(c: &mut Checker) -> Check {
        let viewport = c.viewport()?;
        let margin = px(8.);
        let right = point(viewport.width - px(20.), px(100.));
        let bottom = point(px(100.), viewport.height - px(20.));
        let corner = point(viewport.width - px(20.), viewport.height - px(20.));
        for at in [right, bottom, corner] {
            open_context_at(c, at).await?;
            let bounds = menu_bounds(c, Which::Context)?;
            ensure!(inside(bounds, viewport, margin), "the menu opened at {at:?} leaves the window: {bounds:?} in {viewport:?}");
            if at.x == right.x {
                ensure!(
                    close_to(bounds.right(), at.x),
                    "near the right edge the menu {bounds:?} isn't just to the left of the pointer at {at:?}"
                );
            } else {
                ensure!(close_to(bounds.left(), at.x), "the menu {bounds:?} isn't at the pointer's x ({at:?})");
            }
            if at.y == right.y {
                ensure!(close_to(bounds.top(), at.y), "the menu {bounds:?} isn't at the pointer's y ({at:?})");
            } else {
                ensure!(bounds.top() < at.y, "near the bottom the menu {bounds:?} wasn't shifted up from {at:?}");
            }
            c.key("escape").await?;
            ensure!(c.layers()? == 0, "escape didn't close the menu opened at {at:?}");
        }
        Ok(())
    }

    /// Up and down skip the disabled item and the separator and stop at
    /// the ends; home, end, PageUp and PageDown go to the first and last;
    /// typing goes by the labels and passes the disabled item. Nothing is
    /// selected and the menu stays open.
    async fn context_menu_keys(c: &mut Checker) -> Check {
        let before = menu_log(c)?;
        open_context_at(c, point(px(200.), px(40.))).await?;
        // Rename Tab 0, Move Tab Left 1 (disabled), Move Tab Right 2,
        // separator 3, Close Tab 4.
        let keys = [
            ("down", 0),
            ("down", 2),
            ("down", 4),
            ("down", 4),
            ("up", 2),
            ("up", 0),
            ("up", 0),
            ("end", 4),
            ("home", 0),
            ("pagedown", 4),
            ("pageup", 0),
            ("ctrl-n", 2),
            ("ctrl-p", 0),
        ];
        for (key, expected) in keys {
            c.key(key).await?;
            let highlight = menu_highlight(c, Which::Context)?;
            ensure!(highlight == Some(expected), "{key} highlighted {highlight:?}, expected {expected}");
        }
        for (key, expected) in [("m", 2), ("c", 4), ("r", 0)] {
            c.cx.background_executor().timer(TYPEAHEAD_GAP).await;
            c.key(key).await?;
            let highlight = menu_highlight(c, Which::Context)?;
            ensure!(highlight == Some(expected), "typing {key} highlighted {highlight:?}, expected {expected}");
        }
        ensure!(c.layers()? == 1, "a key closed the menu");
        ensure!(menu_log(c)? == before, "a key selected {:?}", &menu_log(c)?[before.len()..]);
        Ok(())
    }

    /// The spec's scenario: down twice and enter select the second enabled
    /// item, tell the owner and close the menu; focus goes back to the
    /// button that had it, which the enter doesn't reach.
    async fn context_menu_enter_selects(c: &mut Checker) -> Check {
        c.focus("open-dialog").await?;
        let before = menu_log(c)?.len();
        open_context_at(c, point(px(200.), px(40.))).await?;
        c.key("down").await?;
        c.key("down").await?;
        c.key("enter").await?;
        ensure!(c.layers()? == 0 && open_menu(c, Which::Context)?.is_none(), "enter didn't close the menu");
        let log = menu_log(c)?;
        ensure!(log[before..] == ["context:Move Tab Right"], "the owner heard {:?}", &log[before..]);
        ensure!(c.focused_name("open-dialog")?, "focus didn't return to the button that had it");
        ensure!(!c.with(|gallery, _, _| gallery.dialog.is_open())?, "the enter reached the button and opened the dialog");
        Ok(())
    }

    /// Space selects the highlighted item, except while a typeahead search
    /// is under way, when it is part of the search (Radix's rule).
    async fn context_menu_space_selects(c: &mut Checker) -> Check {
        let before = menu_log(c)?.len();
        open_context_at(c, point(px(200.), px(40.))).await?;
        c.key("m").await?;
        ensure!(menu_highlight(c, Which::Context)? == Some(2), "m didn't highlight Move Tab Right");
        c.key("space").await?;
        ensure!(c.layers()? == 1, "a space during the search closed the menu");
        ensure!(menu_log(c)?.len() == before, "a space during the search selected");
        ensure!(menu_highlight(c, Which::Context)? == Some(2), "a space during the search moved the highlight");
        c.cx.background_executor().timer(TYPEAHEAD_GAP).await;
        c.key("space").await?;
        ensure!(c.layers()? == 0, "space didn't select and close the menu");
        let log = menu_log(c)?;
        ensure!(log[before..] == ["context:Move Tab Right"], "the owner heard {:?}", &log[before..]);
        Ok(())
    }

    /// The spec's scenario: Escape closes the menu and focus is where it
    /// was before; nothing is selected.
    async fn context_menu_escape(c: &mut Checker) -> Check {
        c.focus("open-dialog").await?;
        let before = menu_log(c)?;
        open_context_at(c, point(px(200.), px(40.))).await?;
        c.key("down").await?;
        c.key("escape").await?;
        ensure!(c.layers()? == 0 && open_menu(c, Which::Context)?.is_none(), "escape didn't close the menu");
        ensure!(c.focused_name("open-dialog")?, "focus isn't back on the button that had it");
        ensure!(menu_log(c)? == before, "escape selected {:?}", &menu_log(c)?[before.len()..]);
        Ok(())
    }

    /// Radix's trigger keys: enter, space and down open the menu below the
    /// trigger, start-aligned and 4px away, highlighting nothing; Escape
    /// gives focus back to the trigger. Up, left, right, home, end and
    /// ctrl-n don't open it.
    async fn dropdown_keys(c: &mut Checker) -> Check {
        let trigger = dropdown_trigger(c, Which::Dropdown)?;
        let handle = c.handle("dropdown-trigger")?;
        for key in ["enter", "space", "down"] {
            c.focus("dropdown-trigger").await?;
            c.key(key).await?;
            ensure!(c.layers()? == 1 && open_menu(c, Which::Dropdown)?.is_some(), "{key} on the trigger didn't open the menu");
            c.tick().await;
            let highlight = menu_highlight(c, Which::Dropdown)?;
            ensure!(highlight.is_none(), "{key}: the menu opened highlighting {highlight:?}");
            let bounds = menu_bounds(c, Which::Dropdown)?;
            ensure!(
                close_to(bounds.left(), trigger.left()) && close_to(bounds.top(), trigger.bottom() + px(4.)),
                "{key}: the menu {bounds:?} isn't 4px below the trigger {trigger:?}, aligned to its start"
            );
            ensure!(c.modality()? == Modality::Keyboard, "{key} didn't count as keyboard input");
            c.key("escape").await?;
            ensure!(c.layers()? == 0, "escape after {key} didn't close the menu");
            ensure!(c.focused(&handle)?, "escape after {key} didn't give focus back to the trigger");
        }
        for key in ["up", "left", "right", "home", "end", "ctrl-n"] {
            c.focus("dropdown-trigger").await?;
            c.key(key).await?;
            ensure!(c.layers()? == 0, "{key} on the trigger opened the menu");
            ensure!(c.focused(&handle)?, "{key} moved focus off the trigger");
        }
        Ok(())
    }

    /// The spec's scenario: the tmux item shows its description on a second
    /// line, and `t` highlights it (bash's description starts with `t`, its
    /// label doesn't). Typing passes the disabled zsh, and a letter only a
    /// description starts with matches nothing.
    async fn dropdown_description(c: &mut Checker) -> Check {
        c.focus("dropdown-trigger").await?;
        c.key("down").await?;
        c.key("t").await?;
        let highlight = menu_highlight(c, Which::Dropdown)?;
        ensure!(highlight == Some(2), "t highlighted {highlight:?}, not tmux");
        let (bash, fish, tmux) =
            (entry_bounds(c, Which::Dropdown, 0)?, entry_bounds(c, Which::Dropdown, 1)?, entry_bounds(c, Which::Dropdown, 2)?);
        ensure!(
            tmux.size.height > fish.size.height + px(8.),
            "the tmux row ({:?} tall) holds no second line beside fish's ({:?})",
            tmux.size.height,
            fish.size.height
        );
        ensure!(close_to(bash.size.height, tmux.size.height), "the described rows differ in height");
        c.cx.background_executor().timer(TYPEAHEAD_GAP).await;
        c.key("s").await?;
        let highlight = menu_highlight(c, Which::Dropdown)?;
        ensure!(highlight == Some(5), "s highlighted {highlight:?}, not Settings… past the disabled zsh");
        c.cx.background_executor().timer(TYPEAHEAD_GAP).await;
        // "not installed", zsh's description: no label starts with n.
        c.key("n").await?;
        let highlight = menu_highlight(c, Which::Dropdown)?;
        ensure!(highlight == Some(5), "n, which only a description starts with, moved the highlight to {highlight:?}");
        Ok(())
    }

    /// The spec's scenario: a menu with more items than fit is as tall as
    /// the window allows below its trigger, and scrolls so the highlight
    /// stays in view.
    async fn dropdown_long_menu(c: &mut Checker) -> Check {
        c.reveal("long-dropdown").await?;
        let trigger = dropdown_trigger(c, Which::Long)?;
        let viewport = c.viewport()?;
        c.focus("long-dropdown-trigger").await?;
        c.key("down").await?;
        c.tick().await;
        let bounds = menu_bounds(c, Which::Long)?;
        let limit = MenuAnchor::below(trigger).max_height(viewport);
        ensure!(
            (bounds.size.height - limit).abs() <= px(1.),
            "the menu is {:?} tall, not the {limit:?} the window allows",
            bounds.size.height
        );
        ensure!(inside(bounds, viewport, px(8.)), "the menu {bounds:?} leaves the window {viewport:?}");
        c.key("down").await?;
        ensure!(menu_highlight(c, Which::Long)? == Some(0), "down didn't highlight the first profile");
        let last = entry_bounds(c, Which::Long, 39)?;
        ensure!(!in_view(last, bounds), "the last profile {last:?} is in view at the start: nothing to scroll");
        for (key, ix) in [("end", 39), ("home", 0), ("pagedown", 39), ("up", 38)] {
            c.key(key).await?;
            c.tick().await;
            let highlight = menu_highlight(c, Which::Long)?;
            ensure!(highlight == Some(ix), "{key} highlighted {highlight:?}, not {ix}");
            let row = entry_bounds(c, Which::Long, ix)?;
            ensure!(in_view(row, bounds), "after {key} the highlighted row {row:?} is outside the menu's view {bounds:?}");
        }
        Ok(())
    }

    /// The spec's scenarios: a secondary press on the target opens the menu
    /// with its corner at the pointer, and its release 8px away, over the
    /// first item, selects nothing and leaves the menu open.
    async fn context_menu_press_and_near_release(c: &mut Checker) -> Check {
        let at = center(c.reveal("context-target").await?);
        let before = menu_log(c)?;
        c.button_down(MouseButton::Right, at).await?;
        c.tick().await;
        let bounds = menu_bounds(c, Which::Context)?;
        ensure!(
            close_to(bounds.left(), at.x) && close_to(bounds.top(), at.y),
            "the menu's corner is at {:?}, not the pointer's {at:?}",
            bounds.origin
        );
        let release = point(at.x + px(8.), at.y + px(8.));
        c.button_up(MouseButton::Right, release).await?;
        ensure!(open_menu(c, Which::Context)?.is_some(), "the release within 10px closed the menu");
        ensure!(menu_log(c)? == before, "the release within 10px selected {:?}", &menu_log(c)?[before.len()..]);
        Ok(())
    }

    /// The spec's scenario: secondary press, a move of more than 10px onto
    /// an enabled item, and the release there select it.
    async fn context_menu_one_gesture(c: &mut Checker) -> Check {
        let at = center(c.reveal("context-target").await?);
        let before = menu_log(c)?.len();
        c.button_down(MouseButton::Right, at).await?;
        c.tick().await;
        let row = center(entry_bounds(c, Which::Context, 2)?);
        ensure!(row.y - at.y > px(10.), "Move Tab Right at {row:?} is within 10px of the press at {at:?}");
        c.button_move(MouseButton::Right, row).await?;
        let highlight = menu_highlight(c, Which::Context)?;
        ensure!(highlight == Some(2), "moving onto Move Tab Right with the button held highlighted {highlight:?}");
        c.button_up(MouseButton::Right, row).await?;
        ensure!(c.layers()? == 0, "the release on Move Tab Right didn't close the menu");
        let log = menu_log(c)?;
        ensure!(log[before..] == ["context:Move Tab Right"], "the owner heard {:?}", &log[before..]);
        Ok(())
    }

    /// Moving over an enabled item highlights it, over the disabled one
    /// clears the highlight, and a primary click selects.
    async fn context_menu_hover_and_click(c: &mut Checker) -> Check {
        open_context_at(c, point(px(200.), px(40.))).await?;
        let before = menu_log(c)?.len();
        let rows = [
            center(entry_bounds(c, Which::Context, 0)?),
            center(entry_bounds(c, Which::Context, 1)?),
            center(entry_bounds(c, Which::Context, 2)?),
        ];
        for (row, expected) in [(rows[0], Some(0)), (rows[1], None), (rows[2], Some(2))] {
            c.hover(row).await?;
            c.tick().await;
            let highlight = menu_highlight(c, Which::Context)?;
            ensure!(highlight == expected, "the pointer at {row:?} highlighted {highlight:?}, expected {expected:?}");
        }
        c.press(rows[2]).await?;
        ensure!(c.layers()? == 0, "the click on Move Tab Right didn't close the menu");
        let log = menu_log(c)?;
        ensure!(log[before..] == ["context:Move Tab Right"], "the owner heard {:?}", &log[before..]);
        Ok(())
    }

    /// The spec's scenario: a click on the disabled item (or the
    /// separator) selects nothing and the menu stays open.
    async fn context_menu_disabled_click(c: &mut Checker) -> Check {
        open_context_at(c, point(px(200.), px(40.))).await?;
        let before = menu_log(c)?;
        for (ix, what) in [(1, "the disabled Move Tab Left"), (3, "the separator")] {
            let at = center(entry_bounds(c, Which::Context, ix)?);
            c.press(at).await?;
            ensure!(open_menu(c, Which::Context)?.is_some(), "a click on {what} closed the menu");
            ensure!(menu_log(c)? == before, "a click on {what} selected {:?}", &menu_log(c)?[before.len()..]);
        }
        Ok(())
    }

    /// The spec's scenario: a press outside closes the menu and the button
    /// pressed isn't activated; focus goes back. A secondary press outside,
    /// on the menu's own target, closes it and opens nothing.
    async fn context_menu_outside_press(c: &mut Checker) -> Check {
        let target = center(c.reveal("context-target").await?);
        let behind = center(c.recorded("behind")?);
        c.focus("open-dialog").await?;
        open_context_at(c, point(px(200.), px(40.))).await?;
        let clicks = c.clicks("behind")?;
        c.press(behind).await?;
        ensure!(c.layers()? == 0, "the outside press didn't close the menu");
        ensure!(c.clicks("behind")? == clicks, "the outside press clicked the button beneath");
        ensure!(c.focused_name("open-dialog")?, "focus didn't return to the button that had it");
        open_context_at(c, point(px(200.), px(40.))).await?;
        c.button_down(MouseButton::Right, target).await?;
        c.button_up(MouseButton::Right, target).await?;
        ensure!(c.layers()? == 0, "the secondary press outside left {} layers open (it reached the target)", c.layers()?);
        ensure!(c.focused_name("open-dialog")?, "focus didn't return after the secondary press");
        Ok(())
    }

    /// The spec's scenario: a press on the trigger opens the menu (its
    /// release, where it began, leaves it open), and a second press closes
    /// it without reopening it on the release.
    async fn dropdown_trigger_toggles(c: &mut Checker) -> Check {
        c.reveal("dropdown").await?;
        let at = center(dropdown_trigger(c, Which::Dropdown)?);
        let before = menu_log(c)?;
        c.press(at).await?;
        ensure!(c.layers()? == 1 && open_menu(c, Which::Dropdown)?.is_some(), "the first press didn't leave the menu open");
        let highlight = menu_highlight(c, Which::Dropdown)?;
        ensure!(highlight.is_none(), "the press opened the menu highlighting {highlight:?}");
        c.press(at).await?;
        ensure!(c.layers()? == 0, "the second press didn't close the menu");
        c.tick().await;
        ensure!(c.layers()? == 0, "the menu reopened after the second press");
        ensure!(c.focused_name("dropdown-trigger")?, "focus isn't on the trigger after the menu closed");
        ensure!(menu_log(c)? == before, "the presses selected {:?}", &menu_log(c)?[before.len()..]);
        Ok(())
    }

    /// The spec's scenario: press the trigger, move onto the third item and
    /// release. The row is found first with the menu opened by key.
    async fn dropdown_drag_selects(c: &mut Checker) -> Check {
        c.reveal("dropdown").await?;
        let trigger = dropdown_trigger(c, Which::Dropdown)?;
        c.focus("dropdown-trigger").await?;
        c.key("down").await?;
        let row = center(entry_bounds(c, Which::Dropdown, 2)?);
        c.key("escape").await?;
        ensure!(c.layers()? == 0, "escape didn't close the menu opened to find the row");
        let before = menu_log(c)?.len();
        c.drag(center(trigger), row).await?;
        ensure!(c.layers()? == 0, "the release on tmux didn't close the menu");
        let log = menu_log(c)?;
        ensure!(log[before..] == ["dropdown:tmux"], "the owner heard {:?}", &log[before..]);
        ensure!(c.focused_name("dropdown-trigger")?, "focus didn't return to the trigger");
        Ok(())
    }

    /// Enter opens the menu and enter selects; the key-up that follows,
    /// with focus back on the trigger, doesn't open it again.
    async fn dropdown_enter_keyup(c: &mut Checker) -> Check {
        let before = menu_log(c)?.len();
        c.focus("dropdown-trigger").await?;
        c.key("enter").await?;
        ensure!(c.layers()? == 1, "enter didn't open the menu");
        c.key("down").await?;
        c.key("enter").await?;
        ensure!(c.layers()? == 0, "enter didn't select and close");
        let log = menu_log(c)?;
        ensure!(log[before..] == ["dropdown:bash"], "the owner heard {:?}", &log[before..]);
        ensure!(c.focused_name("dropdown-trigger")?, "focus didn't return to the trigger");
        if !c.key_up("enter").await? {
            return Err(Failure::Skip(NO_KEY_UP.to_string()));
        }
        ensure!(c.layers()? == 0, "the enter key-up reopened the menu");
        Ok(())
    }

    // ---- tabs ----

    use brindle_ui::set_modality;

    fn tabs_of(c: &mut Checker, name: &'static str) -> Result<super::TabsDemo, Failure> {
        c.with(move |gallery, _, _| gallery.tabs_demo(name).clone())
    }

    fn tab_active(c: &mut Checker, name: &'static str) -> Result<usize, Failure> {
        Ok(tabs_of(c, name)?.active)
    }

    fn tab_labels(c: &mut Checker, name: &'static str) -> Result<Vec<String>, Failure> {
        Ok(tabs_of(c, name)?.tabs.iter().map(|tab| tab.label.to_string()).collect())
    }

    fn tab_log(c: &mut Checker) -> Result<Vec<String>, Failure> {
        c.with(|gallery, _, _| gallery.tab_log.clone())
    }

    /// The check's precondition: trigger `ix` of `name` is the active one.
    async fn set_tab_active(c: &mut Checker, name: &'static str, ix: usize) -> Check {
        c.with(move |gallery, _, cx| gallery.set_tab_active(name, ix, cx))?;
        c.tick().await;
        Ok(())
    }

    /// Where trigger `ix` of `name` is laid out, scrolled into view.
    async fn trigger(c: &mut Checker, name: &str, ix: usize) -> Result<Bounds<Pixels>, Failure> {
        c.reveal(&format!("tab:{name}:{ix}")).await
    }

    /// The spec's scenario: focus on the second of four triggers, right
    /// makes the third active, and focus stays on the list, which is focus
    /// on the active trigger. The key counts as keyboard input, so the ring
    /// shows.
    async fn tabs_arrow_activates(c: &mut Checker) -> Check {
        set_tab_active(c, "plain", 1).await?;
        c.focus("tabs-plain").await?;
        c.with(|_, window, cx| set_modality(window, Modality::Pointer, cx))?;
        let before = tab_log(c)?.len();
        c.key("right").await?;
        let active = tab_active(c, "plain")?;
        ensure!(active == 2, "right on the second of four made trigger {active} active, not the third");
        ensure!(c.focused_name("tabs-plain")?, "focus left the list");
        ensure!(c.modality()? == Modality::Keyboard, "right didn't count as keyboard input (no ring would show)");
        let log = tab_log(c)?;
        ensure!(log[before..] == ["plain:activate:2:keyboard"], "the owner heard {:?}", &log[before..]);
        Ok(())
    }

    /// The spec's scenario: right on the last trigger comes round to the
    /// first, and left goes back round to the last.
    async fn tabs_wrap(c: &mut Checker) -> Check {
        set_tab_active(c, "plain", 3).await?;
        c.focus("tabs-plain").await?;
        c.key("right").await?;
        let active = tab_active(c, "plain")?;
        ensure!(active == 0, "right on the last trigger made {active} active, not the first");
        ensure!(c.focused_name("tabs-plain")?, "focus left the list");
        c.key("left").await?;
        let active = tab_active(c, "plain")?;
        ensure!(active == 3, "left on the first trigger made {active} active, not the last");
        Ok(())
    }

    /// The spec's scenario: the next trigger (Billing) is disabled, so
    /// right makes the one after it active; left skips it on the way back.
    async fn tabs_disabled_skipped(c: &mut Checker) -> Check {
        set_tab_active(c, "default", 1).await?;
        c.focus("tabs-default").await?;
        let before = tab_log(c)?.len();
        c.key("right").await?;
        let active = tab_active(c, "default")?;
        ensure!(active == 3, "right past the disabled Billing made {active} active, not Settings");
        c.key("left").await?;
        let active = tab_active(c, "default")?;
        ensure!(active == 1, "left past the disabled Billing made {active} active, not Password");
        let log = tab_log(c)?;
        ensure!(
            log[before..] == ["default:activate:3:keyboard", "default:activate:1:keyboard"],
            "the owner heard {:?}",
            &log[before..]
        );
        Ok(())
    }

    /// Home and PageUp go to the first trigger, end and PageDown to the
    /// last (Radix's roving focus); up, down, modified arrows, enter, space
    /// and escape change nothing and leave focus on the list.
    async fn tabs_home_end(c: &mut Checker) -> Check {
        set_tab_active(c, "plain", 1).await?;
        c.focus("tabs-plain").await?;
        for (key, expected) in [("end", 3), ("home", 0), ("pagedown", 3), ("pageup", 0)] {
            c.key(key).await?;
            let active = tab_active(c, "plain")?;
            ensure!(active == expected, "{key} made {active} active, not {expected}");
        }
        let before = tab_log(c)?.len();
        for key in ["up", "down", "shift-right", "ctrl-right", "alt-left", "shift-end", "enter", "space", "escape"] {
            c.key(key).await?;
            let active = tab_active(c, "plain")?;
            ensure!(active == 0, "{key} made {active} active");
            ensure!(c.focused_name("tabs-plain")?, "{key} moved focus off the list");
        }
        let log = tab_log(c)?;
        ensure!(log.len() == before, "the other keys asked for {:?}", &log[before..]);
        Ok(())
    }

    /// Each list is one keyboard stop: Tab goes from list to list (not from
    /// trigger to trigger) and on past the last; shift-Tab comes back.
    async fn tabs_one_stop(c: &mut Checker) -> Check {
        c.focus("toggle").await?;
        for name in ["tabs-default", "tabs-plain", "tabs-strip", "corner-0"] {
            c.key("tab").await?;
            ensure!(c.focused_name(name)?, "Tab didn't move on to {name}");
        }
        c.key("shift-tab").await?;
        ensure!(c.focused_name("tabs-strip")?, "shift-Tab didn't come back to the strip list");
        Ok(())
    }

    /// The long nvim title is cut at the 15rem maximum, the tmux tab keeps
    /// its own width between the limits, and the crowded list's triggers
    /// shrink to the 5rem minimum.
    async fn tabs_widths(c: &mut Checker) -> Check {
        let rem = c.with(|_, window, _| f32::from(window.rem_size()))?;
        let limits = super::demo_tab_width();
        let (min, max) = (limits.min().0 * rem, limits.max().0 * rem);
        let close = |a: f32, b: f32| (a - b).abs() < 0.5;
        let long = f32::from(trigger(c, "strip", 1).await?.size.width);
        ensure!(close(long, max), "the long title's trigger is {long}px wide, not the {max}px maximum");
        let tmux = f32::from(trigger(c, "strip", 2).await?.size.width);
        ensure!(tmux > min + 0.5 && tmux < max - 0.5, "the tmux trigger is {tmux}px wide, not its own width within {min}..{max}");
        for ix in 0..6 {
            let width = f32::from(trigger(c, "crowded", ix).await?.size.width);
            ensure!(close(width, min), "crowded trigger {ix} is {width}px wide, not the {min}px minimum");
        }
        Ok(())
    }

    /// Tabbed to, the list rings its active trigger, and nothing else; the
    /// ring goes when Tab moves on. Round the triggers is the pill's muted
    /// padding.
    async fn tabs_ring(c: &mut Checker) -> Check {
        set_tab_active(c, "plain", 1).await?;
        c.focus("tabs-default").await?;
        c.key("tab").await?;
        ensure!(c.focused_name("tabs-plain")?, "Tab didn't reach the plain list");
        let (background, ring) = c.with(|_, _, cx| {
            // The muted token is opaque.
            let t = cx.ui_theme();
            (over(t.muted, t.background), over(brindle_ui::focus_ring(t), t.muted))
        })?;
        let (active, other) = (trigger(c, "plain", 1).await?, trigger(c, "plain", 0).await?);
        let above = |bounds: Bounds<Pixels>| point(bounds.center().x, bounds.top() - px(1.5));
        let shown = c.pixels(&[above(active), above(other)]).await?;
        ensure!(near(shown[0], ring), "above the active trigger shows {:?}, not the ring {ring:?}", shown[0]);
        ensure!(near(shown[1], background), "an inactive trigger shows a ring ({:?})", shown[1]);
        c.key("tab").await?;
        let after = c.pixel(above(active)).await?;
        ensure!(near(after, background), "the ring stayed after Tab moved on ({after:?})");
        Ok(())
    }

    /// The spec's scenarios: a trigger given owner colours is drawn on its
    /// surface in its text colour whether active or not (bash `#c0caf5` on
    /// `#1a1b26`), with no line along its top; a trigger without colours of
    /// its own is active on the pill's raised background and inactive on
    /// the muted list; the first, no longer active, keeps its surface and
    /// its text colour (at 60%).
    async fn tabs_owner_colours(c: &mut Checker) -> Check {
        pointer_away(c).await?;
        set_tab_active(c, "strip", 0).await?;
        let (raised, muted, primary) = c.with(|_, _, cx| {
            let t = cx.ui_theme();
            let tab = brindle_ui::tab_style(true, Default::default(), t);
            let raised = tab.background.map_or(over(t.muted, t.background), |b| over(b, t.muted));
            (raised, over(t.muted, t.background), over(t.primary, t.background))
        })?;
        let (bash_surface, bash_text, nvim_surface) = ([0x1a, 0x1b, 0x26], [0xc0, 0xca, 0xf5], [0xfd, 0xf6, 0xe3]);
        // How light a pixel is, and the lightest a label's glyphs get: the
        // text colour where they cover a pixel, 60% of it over the surface
        // while inactive. Edges blend towards the surface, so the label is
        // judged by its lightest pixel.
        let light = |p: [u8; 3]| p.iter().map(|&c| c as u32).sum::<u32>();
        let dim = (light(bash_text) as f32 * 0.6 + light(bash_surface) as f32 * 0.4) as u32;
        let lightest = |row: &[[u8; 3]]| row.iter().copied().map(light).max().unwrap_or(0);
        let inside = |bounds: Bounds<Pixels>| point(bounds.left() + px(6.), bounds.bottom() - px(4.));
        let top = |bounds: Bounds<Pixels>| point(bounds.center().x, bounds.top() + px(1.5));
        // The label's pixels: a row through the text's middle, from the left
        // padding to the trigger's middle.
        let label_row = |bounds: Bounds<Pixels>| -> Vec<Point<Pixels>> {
            let (left, right) = (f32::from(bounds.left()) + 8., f32::from(bounds.center().x));
            (0..((right - left) as usize)).map(|dx| point(px(left + dx as f32), bounds.center().y)).collect()
        };
        let (bash, nvim) = (trigger(c, "strip", 0).await?, trigger(c, "strip", 1).await?);
        let shown = c.pixels(&[inside(bash), top(bash), inside(nvim), top(nvim)]).await?;
        ensure!(near(shown[0], bash_surface), "the active trigger's surface is {:?}, not #1a1b26", shown[0]);
        ensure!(near(shown[1], bash_surface), "the active trigger's top edge is {:?}, not its surface (a line?)", shown[1]);
        ensure!(near(shown[2], nvim_surface), "an inactive trigger with a surface is drawn on {:?}, not #fdf6e3", shown[2]);
        ensure!(!near(shown[3], primary) && near(shown[3], nvim_surface), "an inactive trigger's top edge is {:?}", shown[3]);
        let text = lightest(&c.pixels(&label_row(bash)).await?);
        ensure!(text > dim + 24, "the active bash label is at most {text} light, not #c0caf5 at full opacity (60% is {dim})");
        // The close button's X crosses at its centre, in the text colour:
        // judged by the lightest pixel of the 3x3 round the centre, since
        // where the centre falls on the pixel grid depends on the display
        // scale and the strokes' edges blend towards the surface (Windows
        // measured a 75% blend on the one pixel this used to read).
        let rem = c.with(|_, window, _| f32::from(window.rem_size()))?;
        let centre = close_button(bash, rem);
        let around: Vec<Point<Pixels>> =
            (-1..=1).flat_map(|dy| (-1..=1).map(move |dx| point(centre.x + px(dx as f32), centre.y + px(dy as f32)))).collect();
        let x = lightest(&c.pixels(&around).await?);
        ensure!(
            x > dim + 24,
            "the active bash close button's X is at most {x} light, not #c0caf5 at full opacity (60% is {dim})"
        );
        set_tab_active(c, "strip", 2).await?;
        let (bash, tmux) = (trigger(c, "strip", 0).await?, trigger(c, "strip", 2).await?);
        let shown = c.pixels(&[inside(tmux), top(tmux), inside(bash), top(bash)]).await?;
        ensure!(near(shown[0], raised), "a trigger without colours is active on {:?}, not the raised pill {raised:?}", shown[0]);
        ensure!(near(shown[1], raised), "its top edge is {:?}, not the raised pill (a line?)", shown[1]);
        ensure!(near(shown[2], bash_surface), "the inactive bash trigger lost its surface ({:?})", shown[2]);
        ensure!(near(shown[3], bash_surface), "the inactive bash trigger's top edge is {:?}", shown[3]);
        let text = lightest(&c.pixels(&label_row(bash)).await?);
        ensure!(
            text <= dim + 12 && text > light(bash_surface) + 120,
            "the inactive bash label is at most {text} light, not #c0caf5 at 60% ({dim})"
        );
        let below = c.pixel(point(bash.center().x, bash.bottom() + px(1.5))).await?;
        ensure!(near(below, muted), "the list round the triggers is {below:?}, not the muted pill {muted:?}");
        Ok(())
    }

    /// A primary press activates the trigger (and tells the owner it was the
    /// pointer) without moving focus: not to the list, not to the gallery
    /// around it. A press in the list's padding does nothing.
    async fn tabs_press_activates(c: &mut Checker) -> Check {
        set_tab_active(c, "plain", 1).await?;
        let at = center(trigger(c, "plain", 3).await?);
        c.focus("toggle").await?;
        let before = tab_log(c)?.len();
        c.press(at).await?;
        let active = tab_active(c, "plain")?;
        ensure!(active == 3, "a press on Notifications made {active} active");
        let log = tab_log(c)?;
        ensure!(log[before..] == ["plain:activate:3:pointer"], "the owner heard {:?}", &log[before..]);
        ensure!(c.focused_name("toggle")?, "the press moved focus off the toggle");
        ensure!(!c.focused_name("tabs-plain")?, "the press focused the list");
        ensure!(c.modality()? == Modality::Pointer, "the press didn't set the pointer modality");
        let list = c.recorded("tabs:plain")?;
        c.press(point(list.left() + px(1.5), list.center().y)).await?;
        ensure!(tab_active(c, "plain")? == 3, "a press in the list's padding changed the active trigger");
        ensure!(tab_log(c)?.len() == before + 1, "a press in the list's padding asked for something");
        ensure!(c.focused_name("toggle")?, "a press in the list's padding moved focus");
        Ok(())
    }

    async fn tabs_disabled_press(c: &mut Checker) -> Check {
        set_tab_active(c, "default", 1).await?;
        let at = center(trigger(c, "default", 2).await?);
        c.focus("toggle").await?;
        let before = tab_log(c)?.len();
        c.press(at).await?;
        ensure!(tab_active(c, "default")? == 1, "a press on the disabled Billing activated it");
        ensure!(tab_log(c)?.len() == before, "a press on a disabled trigger asked for something");
        ensure!(c.focused_name("toggle")?, "a press on a disabled trigger moved focus");
        Ok(())
    }

    /// The centre of the close button of trigger `bounds`: an `IconXs`
    /// button (1.5rem) against the trigger's 0.25rem right padding.
    fn close_button(bounds: Bounds<Pixels>, rem: f32) -> Point<Pixels> {
        point(bounds.right() - px(rem), bounds.center().y)
    }

    /// The spec's scenario: a press on an inactive trigger's close button
    /// (shown under the pointer) asks to close it and doesn't activate it;
    /// the active trigger's button closes it too.
    async fn tabs_close_button(c: &mut Checker) -> Check {
        set_tab_active(c, "strip", 0).await?;
        let rem = c.with(|_, window, _| f32::from(window.rem_size()))?;
        let labels = tab_labels(c, "strip")?;
        let at = close_button(trigger(c, "strip", 1).await?, rem);
        c.hover(at).await?;
        c.tick().await;
        let before = tab_log(c)?.len();
        c.press(at).await?;
        let log = tab_log(c)?;
        ensure!(log[before..] == ["strip:close:1"], "the close button's press made the owner hear {:?}", &log[before..]);
        let mut expected = labels.clone();
        expected.remove(1);
        ensure!(tab_labels(c, "strip")? == expected, "the tabs are {:?} after closing the second", tab_labels(c, "strip")?);
        ensure!(tab_active(c, "strip")? == 0, "closing the second tab changed the active one");
        let at = close_button(trigger(c, "strip", 0).await?, rem);
        c.press(at).await?;
        let log = tab_log(c)?;
        ensure!(log[before + 1..] == ["strip:close:0"], "the active trigger's close button made the owner hear {:?}", &log[before + 1..]);
        Ok(())
    }

    /// The spec's scenario: a middle press on a closable trigger asks to
    /// close it; on a trigger that doesn't close it does nothing, and
    /// neither moves focus.
    async fn tabs_middle_click(c: &mut Checker) -> Check {
        set_tab_active(c, "strip", 0).await?;
        let labels = tab_labels(c, "strip")?;
        let at = center(trigger(c, "strip", 2).await?);
        c.focus("toggle").await?;
        let before = tab_log(c)?.len();
        c.button_down(MouseButton::Middle, at).await?;
        c.button_up(MouseButton::Middle, at).await?;
        let log = tab_log(c)?;
        ensure!(log[before..] == ["strip:close:2"], "the middle press made the owner hear {:?}", &log[before..]);
        let mut expected = labels.clone();
        expected.remove(2);
        ensure!(tab_labels(c, "strip")? == expected, "the tabs are {:?} after the middle press", tab_labels(c, "strip")?);
        ensure!(c.focused_name("toggle")?, "the middle press moved focus");
        let at = center(trigger(c, "plain", 0).await?);
        c.button_down(MouseButton::Middle, at).await?;
        c.button_up(MouseButton::Middle, at).await?;
        ensure!(tab_log(c)?.len() == before + 1, "a middle press on a trigger that doesn't close asked for something");
        ensure!(tabs_of(c, "plain")?.tabs.len() == 4, "a middle press closed a trigger that doesn't close");
        ensure!(c.focused_name("toggle")?, "the middle press on the plain list moved focus");
        Ok(())
    }

    /// A secondary press on a trigger of a list with `on_context_menu` asks
    /// its owner for that trigger's menu, which the gallery opens at the
    /// pointer: nothing is activated or closed, the release within 10px
    /// leaves the menu open, and escape gives focus back. On a list without
    /// the handler the press asks for nothing.
    async fn tabs_context_menu(c: &mut Checker) -> Check {
        set_tab_active(c, "strip", 0).await?;
        let at = center(trigger(c, "strip", 2).await?);
        c.focus("toggle").await?;
        let before = tab_log(c)?.len();
        c.button_down(MouseButton::Right, at).await?;
        c.tick().await;
        let log = tab_log(c)?;
        ensure!(log[before..] == ["strip:menu:2"], "the secondary press made the owner hear {:?}", &log[before..]);
        ensure!(open_menu(c, Which::Context)?.is_some(), "the secondary press opened no menu");
        let bounds = menu_bounds(c, Which::Context)?;
        ensure!(
            close_to(bounds.left(), at.x) && close_to(bounds.top(), at.y),
            "the menu's corner is at {:?}, not the pointer's {at:?}",
            bounds.origin
        );
        c.button_up(MouseButton::Right, at).await?;
        ensure!(open_menu(c, Which::Context)?.is_some(), "the release of the opening press closed the menu");
        ensure!(tab_active(c, "strip")? == 0, "the secondary press activated a trigger");
        c.key("escape").await?;
        ensure!(open_menu(c, Which::Context)?.is_none(), "escape left the menu open");
        ensure!(c.focused_name("toggle")?, "escape didn't give focus back to the toggle");
        let at = center(trigger(c, "plain", 0).await?);
        c.button_down(MouseButton::Right, at).await?;
        c.button_up(MouseButton::Right, at).await?;
        ensure!(tab_log(c)?.len() == before + 1, "a secondary press on a list without a menu asked for something");
        ensure!(open_menu(c, Which::Context)?.is_none(), "a secondary press on a list without a menu opened one");
        Ok(())
    }

    /// The spec's scenario: the first trigger dragged onto the third asks
    /// to move it to the third place, and while it is over the third a
    /// primary bar on that trigger's right marks where it will land.
    async fn tabs_drag(c: &mut Checker) -> Check {
        set_tab_active(c, "strip", 0).await?;
        let labels = tab_labels(c, "strip")?;
        let from = center(trigger(c, "strip", 0).await?);
        // Near the third trigger's left edge, so the preview, which keeps the
        // pointer where it pressed the first trigger, is clear of the bar on
        // its right.
        let target = trigger(c, "strip", 2).await?;
        let to = point(target.left() + px(12.), target.center().y);
        let primary = c.with(|_, _, cx| over(cx.ui_theme().primary, cx.ui_theme().primary))?;
        let before = tab_log(c)?.len();
        c.button_down(MouseButton::Left, from).await?;
        c.button_move(MouseButton::Left, point(from.x + px(8.), from.y)).await?;
        c.button_move(MouseButton::Left, to).await?;
        let dragging = c.with(|_, _, cx| cx.has_active_drag())?;
        // As the target is drawn now, with the bar.
        let target = c.recorded("tab:strip:2")?;
        let bar = c.pixel(point(target.right() - px(1.), target.center().y)).await;
        c.button_up(MouseButton::Left, to).await?;
        ensure!(dragging, "pressing the first trigger and moving onto the third started no drag");
        let bar = bar?;
        ensure!(near(bar, primary), "the third trigger's right edge shows {bar:?} while dragged over, not the primary bar");
        let log = tab_log(c)?;
        ensure!(
            log[before..] == ["strip:activate:0:pointer", "strip:move:0->2"],
            "the owner heard {:?}",
            &log[before..]
        );
        let expected = vec![labels[1].clone(), labels[2].clone(), labels[0].clone(), labels[3].clone()];
        ensure!(tab_labels(c, "strip")? == expected, "the tabs are {:?} after the drop", tab_labels(c, "strip")?);
        ensure!(tab_active(c, "strip")? == 2, "the moved tab didn't stay active");
        Ok(())
    }

    /// A trigger's tooltip shows after the pointer rests on it.
    async fn tabs_tooltip(c: &mut Checker) -> Check {
        set_tab_active(c, "strip", 0).await?;
        let at = center(trigger(c, "strip", 1).await?);
        let tooltip = tooltip_colour(c)?;
        let sample = tooltip_point(at);
        rest(c, at).await?;
        let shown = c.pixel(sample).await?;
        ensure!(near(shown, tooltip), "resting on a trigger showed {shown:?} at {sample:?}, not the tooltip {tooltip:?}");
        Ok(())
    }

    // ---- cards ----

    use brindle_ui::{CARD_BORDER, CARD_HEADER_HEIGHT, CardColors, card_style};

    /// Whether two laid-out boxes match, within a pixel on each edge.
    fn same_box(a: Bounds<Pixels>, b: Bounds<Pixels>) -> bool {
        close_to(a.left(), b.left()) && close_to(a.top(), b.top()) && close_to(a.right(), b.right()) && close_to(a.bottom(), b.bottom())
    }

    /// `bounds` less `inset` on every side.
    fn inset(bounds: Bounds<Pixels>, inset: Pixels) -> Bounds<Pixels> {
        Bounds {
            origin: point(bounds.left() + inset, bounds.top() + inset),
            size: gpui::size(bounds.size.width - inset * 2., bounds.size.height - inset * 2.),
        }
    }

    /// Within `tolerance` on every channel.
    fn within(a: [u8; 3], b: [u8; 3], tolerance: u8) -> bool {
        a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= tolerance)
    }

    /// The pixel of `row` that stands out most from `surface`: a glyph's
    /// core, where its colour is least blended with what is behind it.
    fn strongest(row: &[[u8; 3]], surface: [u8; 3]) -> [u8; 3] {
        let distance = |p: &[u8; 3]| p.iter().zip(surface).map(|(a, b)| a.abs_diff(b) as u32).sum::<u32>();
        row.iter().copied().max_by_key(distance).unwrap_or(surface)
    }

    /// The points across a header's title, through its middle: from past
    /// the leading dot to the header's middle.
    fn title_row(header: Bounds<Pixels>) -> Vec<Point<Pixels>> {
        let (left, right) = (f32::from(header.left()) + 24., f32::from(header.center().x));
        (0..((right - left) as usize)).map(|dx| point(px(left + dx as f32), header.center().y)).collect()
    }

    /// The spec's "Header optional": a card drawn without a header has its
    /// content fill the whole card inside its border.
    async fn card_headerless_fills(c: &mut Checker) -> Check {
        let card = c.reveal("card:plain").await?;
        let content = c.recorded("card:plain:content")?;
        let inner = inset(card, CARD_BORDER);
        ensure!(same_box(content, inner), "the headerless card's content is laid out at {content:?}, not {inner:?} (inside the border)");
        Ok(())
    }

    /// The card's header row: inside the border at the card's top, the
    /// header's height, and the content below it.
    async fn card_header_above_content(c: &mut Checker) -> Check {
        let card = c.reveal("card:default").await?;
        let header = c.recorded("card:default:header")?;
        let inner = inset(card, CARD_BORDER);
        ensure!(
            close_to(header.top(), inner.top()) && close_to(header.left(), inner.left()) && close_to(header.right(), inner.right()),
            "the header is at {header:?}, not along the top of {inner:?}"
        );
        ensure!(
            close_to(header.size.height, CARD_HEADER_HEIGHT),
            "the header is {:?} tall, not {CARD_HEADER_HEIGHT:?}",
            header.size.height
        );
        Ok(())
    }

    /// The spec's "Default colours" (surface, title and border from the
    /// card, foreground and border tokens, the line under the header in the
    /// border token), and the corners on the background.
    async fn card_default_colours(c: &mut Checker) -> Check {
        let card = c.reveal("card:default").await?;
        let header = c.recorded("card:default:header")?;
        let s = c.with(|_, _, cx| card_style(cx.ui_theme()))?;
        card_colours(c, card, header, s, "default").await
    }

    /// The spec's "Owner colours": a card given a terminal's colours is
    /// drawn in them, header included, whichever token set is active; an
    /// inactive header's title is in the owner's muted colour.
    async fn card_owner_colours(c: &mut Checker) -> Check {
        let owner = super::owner_card_colors();
        let result = card_owner_colours_in_both_sets(c, owner).await;
        c.with(|gallery, _, cx| gallery.set_dark(false, cx))?;
        c.tick().await;
        result
    }

    async fn card_owner_colours_in_both_sets(c: &mut Checker, owner: CardColors) -> Check {
        for dark in [false, true] {
            c.with(move |gallery, _, cx| gallery.set_dark(dark, cx))?;
            c.tick().await;
            let card = c.reveal("card:owner").await?;
            let header = c.recorded("card:owner:header")?;
            let s = c.with(move |_, _, cx| card_style(cx.ui_theme()).with_colors(owner))?;
            card_colours(c, card, header, s, if dark { "owner (dark set)" } else { "owner (light set)" }).await?;
            // The inactive card's title is muted, not in the text colour.
            let inactive = c.recorded("card:owner-inactive")?;
            let inactive_header = Bounds {
                origin: point(inactive.left() + CARD_BORDER, inactive.top() + CARD_BORDER),
                size: gpui::size(inactive.size.width - CARD_BORDER * 2., CARD_HEADER_HEIGHT),
            };
            let (surface, text, muted) = (over(s.surface, s.surface), over(s.text, s.surface), over(s.muted, s.surface));
            let shown = strongest(&c.pixels(&title_row(inactive_header)).await?, surface);
            ensure!(
                within(shown, muted, 32) && !within(shown, text, 32),
                "the muted title's strongest pixel is {shown:?}, not the muted {muted:?} (text {text:?})"
            );
        }
        Ok(())
    }

    /// Reads one card's surface, title, border, header line and corner.
    async fn card_colours(
        c: &mut Checker,
        card: Bounds<Pixels>,
        header: Bounds<Pixels>,
        s: brindle_ui::CardStyle,
        which: &str,
    ) -> Check {
        // The surfaces are opaque, in the tokens and the owner's colours.
        let surface = over(s.surface, s.surround);
        let (text, border, surround) = (over(s.text, s.surface), over(s.border, s.surface), over(s.surround, s.surround));
        let samples = [
            // The content's empty bottom middle.
            point(card.center().x, card.bottom() - px(8.)),
            // The left border, half way down.
            point(card.left() + px(0.5), card.center().y),
            // The line under the header.
            point(card.center().x, header.bottom() - px(0.5)),
            // Just inside the top left corner's square, outside its arc.
            point(card.left() + px(1.5), card.top() + px(1.5)),
        ];
        let shown = c.pixels(&samples).await?;
        ensure!(near(shown[0], surface), "the {which} card's surface is {:?}, not {surface:?}", shown[0]);
        ensure!(near(shown[1], border), "the {which} card's border is {:?}, not {border:?}", shown[1]);
        ensure!(near(shown[2], border), "the {which} card's header line is {:?}, not {border:?}", shown[2]);
        ensure!(near(shown[3], surround), "the {which} card's corner is {:?}, not the surround {surround:?}", shown[3]);
        let title = strongest(&c.pixels(&title_row(header)).await?, surface);
        ensure!(within(title, text, 32), "the {which} card's title is at most {title:?}, not the text {text:?}");
        Ok(())
    }

    /// The spec's "Ring outside the border": the ringed card is laid out as
    /// the plain one is, and a band in the ring colour shows just above its
    /// border, where the plain card shows the background.
    async fn card_ring(c: &mut Checker) -> Check {
        let off = c.reveal("card:ring-off").await?;
        let on = c.recorded("card:ring-on")?;
        ensure!(
            close_to(on.size.width, off.size.width) && close_to(on.size.height, off.size.height) && close_to(on.top(), off.top()),
            "the ringed card is laid out at {on:?}, the plain one at {off:?}: the ring took room"
        );
        let (background, ring) = c.with(|_, _, cx| {
            let t = cx.ui_theme();
            (over(t.background, t.background), over(brindle_ui::focus_ring(t), t.background))
        })?;
        let above = |bounds: Bounds<Pixels>| point(bounds.center().x, bounds.top() - px(1.5));
        let shown = c.pixels(&[above(on), above(off)]).await?;
        ensure!(near(shown[0], ring), "above the ringed card shows {:?}, not the ring {ring:?}", shown[0]);
        ensure!(near(shown[1], background), "above the plain card shows {:?}, not the background {background:?}", shown[1]);
        Ok(())
    }

    /// The spec's "Content with a square background": each corner of the
    /// headerless card, whose content paints a square to its box, shows the
    /// surround outside the rounded border, and the content shows inside.
    async fn card_corners(c: &mut Checker) -> Check {
        let card = c.reveal("card:plain").await?;
        let (surround, border) = c.with(|_, _, cx| {
            let s = card_style(cx.ui_theme());
            (over(s.surround, s.surround), over(s.border, s.surround))
        })?;
        let square = {
            let rgb = gpui::rgb(super::CARD_SQUARE);
            [(rgb.r * 255.).round() as u8, (rgb.g * 255.).round() as u8, (rgb.b * 255.).round() as u8]
        };
        let d = px(1.5);
        let corners = [
            point(card.left() + d, card.top() + d),
            point(card.right() - d, card.top() + d),
            point(card.right() - d, card.bottom() - d),
            point(card.left() + d, card.bottom() - d),
        ];
        let mut samples = corners.to_vec();
        samples.push(card.center());
        samples.push(point(card.left() + px(0.5), card.center().y));
        let shown = c.pixels(&samples).await?;
        for (ix, at) in corners.iter().enumerate() {
            ensure!(
                near(shown[ix], surround),
                "corner {ix} at {at:?} shows {:?}, not the surround {surround:?} (the content is {square:?})",
                shown[ix]
            );
        }
        ensure!(near(shown[4], square), "the content shows {:?}, not its square {square:?}", shown[4]);
        ensure!(near(shown[5], border), "the border over the content is {:?}, not {border:?}", shown[5]);
        Ok(())
    }
}
