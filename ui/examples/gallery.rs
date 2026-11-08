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
//! follow). Elsewhere those checks are skipped. It prints one `ok NAME`,
//! `FAIL NAME: why` or `SKIP NAME: why` line per check and exits non-zero
//! when any check failed or the app panicked.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use brindle_ui::{
    ActiveTheme, Button, ButtonSize, ButtonVariant, Checkbox, Command, CommandEvent, CommandGroup, CommandItem, Dialog,
    Dismiss, FocusStops, Icon, Input, Overlay, Popover, PopoverContent, Select, SelectEvent, SelectOption, Switch,
    TextInput, Tokens, UiTheme,
};
use gpui::{
    App, Application, Bounds, ClickEvent, Context, DismissEvent, Div, Entity, EventEmitter, FocusHandle, Focusable,
    Pixels, ScrollHandle, SharedString, Subscription, TitlebarOptions, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, size,
};

/// Bounds the gallery records as components paint, by name, for the checks
/// (and the popovers' triggers).
type Records = Rc<RefCell<HashMap<String, Bounds<Pixels>>>>;

/// Click counts by control name.
type Counts = Rc<RefCell<HashMap<String, usize>>>;

/// What the command hosts (dialog and popover) confirmed, in order.
type Confirmed = Rc<RefCell<Vec<String>>>;

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
/// Down 3, Attach 4 (disabled), Reload Config 5.
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
        ),
        CommandGroup::unnamed(["Reload Config"]),
    ]
}

/// The shells the gallery's select offers: `zsh` is disabled.
fn shells() -> Vec<SelectOption> {
    vec!["bash".into(), "fish".into(), "foot".into(), SelectOption::new("zsh").disabled(true), "nu".into()]
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
pub struct Gallery {
    dark: bool,
    handles: HashMap<String, FocusHandle>,
    clicks: Counts,
    records: Records,
    checked: bool,
    switched: bool,
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
    command_dialog: Overlay<CommandHost>,
    confirmed: Confirmed,
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
                "disabled", "icon-label", "behind", "open-dialog", "open-stubborn", "open-command-dialog",
                "command-popover", "corner-0", "corner-1", "corner-2", "corner-3", "checkbox", "checkbox-off", "switch",
                "switch-off", "theme",
            ]
            .map(String::from),
        );
        names
    }

    fn button_name(variant: ButtonVariant, size: ButtonSize) -> String {
        format!("button:{variant:?}:{size:?}")
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let handles = Self::handle_names().into_iter().map(|name| (name, cx.focus_handle())).collect();
        let name = cx.new(|cx| TextInput::new("", cx).with_placeholder("Name"));
        let email = cx.new(|cx| TextInput::new("not an address", cx).with_placeholder("Email"));
        let shell = cx.new(|cx| Select::new(shells(), Some(0), cx));
        let editor = cx.new(|cx| Select::new(["vim", "emacs", "nano"], None, cx).with_placeholder("Pick an editor…"));
        let off = cx.new(|cx| Select::new(["unavailable"], Some(0), cx));
        off.update(cx, |select, cx| select.set_disabled(true, window, cx));
        let command = cx.new(|cx| Command::new(command_groups(), cx));
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
        ];
        Self {
            dark: false,
            handles,
            clicks: Counts::default(),
            records: Records::default(),
            checked: false,
            switched: true,
            name,
            email,
            shell,
            shell_changes: Vec::new(),
            editor,
            off,
            command,
            command_log: Vec::new(),
            dialog: Overlay::modal(),
            command_dialog: Overlay::modal(),
            confirmed: Confirmed::default(),
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

    pub fn open_command_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let confirmed = self.confirmed.clone();
        let view = cx.new(|cx| CommandHost::new(Chrome::Dialog, confirmed, cx));
        self.command_dialog.open(view, |this| &mut this.command_dialog, window, cx);
    }

    /// Closes what the gallery owns a slot for (the checks' reset).
    pub fn close_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dialog.close(window, cx);
        self.command_dialog.close(window, cx);
        self.shell.update(cx, |select, cx| select.close(window, cx));
        self.editor.update(cx, |select, cx| select.close(window, cx));
        cx.notify();
    }

    fn header(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .items_center()
            .justify_between()
            .child(div().text_size(px(20.)).font_weight(gpui::FontWeight::SEMIBOLD).child("brindle-ui gallery"))
            .child(
                div().flex().items_center().gap_2().child(label("Dark", cx)).child(
                    Switch::new("theme", self.dark)
                        .focus(&self.handle("theme"))
                        .on_click(cx.listener(|this, dark: &bool, _, cx| this.set_dark(*dark, cx))),
                ),
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
                    .child(
                        Button::new("open-dialog", "Open dialog")
                            .focus(&self.handle("open-dialog"))
                            .on_click(cx.listener(|this, _, window, cx| this.open_dialog(false, window, cx))),
                    )
                    .child(
                        Button::new("open-stubborn", "Open stubborn dialog")
                            .variant(ButtonVariant::Secondary)
                            .focus(&self.handle("open-stubborn"))
                            .on_click(cx.listener(|this, _, window, cx| this.open_dialog(true, window, cx))),
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
        div().flex().flex_col().gap_3().child(heading("Command (inline)", cx)).child(
            div()
                .w(px(420.))
                .rounded(t.radius_md())
                .border_1()
                .border_color(t.border)
                .shadow_sm()
                .child(self.command.clone()),
        )
    }

    fn buttons_section(&self, cx: &mut Context<Self>) -> Div {
        let mut section = div().flex().flex_col().gap_3().child(heading("Button", cx));
        for variant in ButtonVariant::ALL {
            let mut row = div().flex().items_center().gap_3().child(label(format!("{variant:?}"), cx));
            for size in ButtonSize::ALL {
                let name = Self::button_name(variant, size);
                let id: SharedString = name.clone().into();
                let button = match size {
                    ButtonSize::Icon => Button::icon_only(id, Icon::Check),
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
                    .child(
                        Checkbox::new("checkbox", self.checked)
                            .focus(&self.handle("checkbox"))
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.checked = *checked;
                                cx.notify();
                            })),
                    )
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
            .child(self.command_section(cx))
            .child(self.buttons_section(cx))
            .child(self.inputs_section(cx))
            .child(self.toggles_section(cx))
            .child(self.status(cx));
        div()
            .relative()
            .size_full()
            .bg(t.background)
            .text_color(t.foreground)
            .text_size(px(14.))
            .child(content)
            .children(self.corner_triggers())
            // Modal layers render last in the root: tree order is their
            // z-order, and nothing lifts them.
            .children(self.dialog.render(window, cx))
            .children(self.command_dialog.render(window, cx))
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

    use brindle_ui::{ActiveTheme, Modality, modality, primitives::layer};
    use gpui::{
        App, AppContext, AsyncApp, Bounds, Context, Entity, FocusHandle, Focusable, Keystroke, Pixels, Point,
        SharedString, Window, WindowHandle, point, px,
    };

    use super::Gallery;

    /// The window messages the checks post, and the user32 calls for them.
    /// Declared here so the example adds no dependency.
    #[cfg(windows)]
    mod win {
        pub const WM_MOUSEMOVE: u32 = 0x0200;
        pub const WM_LBUTTONDOWN: u32 = 0x0201;
        pub const WM_LBUTTONUP: u32 = 0x0202;
        pub const WM_KEYUP: u32 = 0x0101;
        pub const MK_LBUTTON: usize = 0x0001;
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

    /// Why the pointer checks can't run on this platform.
    #[cfg(not(windows))]
    const NO_MESSAGES: &str = "needs a real pointer: the gallery posts presses as window messages on Windows only";

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

        /// Closes whatever a check left open and clears focus.
        async fn reset(&mut self) -> Result<(), Failure> {
            for _ in 0..4 {
                if self.layers()? == 0 {
                    break;
                }
                self.key("escape").await?;
            }
            self.with(|gallery, window, cx| {
                gallery.close_all(window, cx);
                gallery.scroll.set_offset(point(px(0.), px(0.)));
                window.blur();
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
        let check = theme_switch(&mut c).await;
        c.run_check("theme-switch-redraws-in-dark-tokens", check).await;
        let check = button_mouse_click(&mut c).await;
        c.run_check("button-click-counts-focuses-without-ring", check).await;
        let check = button_space_activates_once(&mut c).await;
        c.run_check("button-space-and-enter-activate-once", check).await;
        let check = button_disabled_inert(&mut c).await;
        c.run_check("button-disabled-runs-nothing-keeps-focus", check).await;
        let check = focus_visible(&mut c).await;
        c.run_check("focus-visible-pointer-then-keyboard", check).await;
        let check = input_frame_press(&mut c).await;
        c.run_check("input-press-in-frame-focuses-text-input", check).await;
        let check = checkbox_space(&mut c).await;
        c.run_check("checkbox-space-toggles-enter-does-not", check).await;
        let check = switch_space(&mut c).await;
        c.run_check("switch-space-toggles", check).await;
        let check = checkbox_disabled(&mut c).await;
        c.run_check("checkbox-disabled-asks-nothing", check).await;
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
        let check = select_opens_on_down(&mut c).await;
        c.run_check("select-down-opens-highlighting-the-selection", check).await;
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
        let check = command_enter_and_escape_inline(&mut c).await;
        c.run_check("command-inline-enter-confirms-escape-cancels", check).await;
        let check = command_dialog(&mut c).await;
        c.run_check("command-in-dialog-escape-and-enter", check).await;
        let check = command_popover_hazard(&mut c).await;
        c.run_check("command-in-popover-enter-key-up-does-not-reopen", check).await;

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

    async fn focus_visible(c: &mut Checker) -> Check {
        let at = center(c.recorded("behind")?);
        c.press(at).await?;
        ensure!(c.modality()? == Modality::Pointer, "a press didn't set the pointer modality");
        c.key("tab").await?;
        ensure!(c.modality()? == Modality::Keyboard, "a key didn't set the keyboard modality");
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

    async fn switch_space(c: &mut Checker) -> Check {
        c.focus("switch").await?;
        let before = c.with(|gallery, _, _| gallery.switched)?;
        c.key("space").await?;
        c.key_up("space").await?;
        let after = c.with(|gallery, _, _| gallery.switched)?;
        ensure!(after == !before, "space didn't ask the owner to flip the switch");
        c.key("space").await?;
        c.key_up("space").await?;
        ensure!(c.with(|gallery, _, _| gallery.switched)? == before, "space again didn't flip it back");
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
}
