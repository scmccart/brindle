---
paths:
  - "ui/**"
---
# brindle-ui: components and tokens on gpui alone

You are editing the shared UI crate. Read `docs/architecture/ports-and-adapters.md` and `docs/architecture/functional-core-imperative-shell.md` before non-trivial changes, and `ui/README.md` for the component list and the listed differences from shadcn.

- Its dependencies are `gpui` and the `log` facade (a failed icon is logged once), and the crate does no IO of its own (`ports R2`, `R3`).
- It knows nothing of terminals, config, `Settings`, terminal themes or the `ACTIONS` table. Components take tokens and plain values; the app maps its concepts onto them (`ports R2`).
- It binds no keys itself: `default_bindings()` returns them and the app chains them in (`ports R2`).
- State and rules go in a GPUI-free model (`EditState`, the filter, placement, list navigation) with unit tests; the entity adds focus, input and painting (`fcis R1`, `R5`).
- Keep choices by value, not by index, across a rebuild of the list (`types R3`).
- A behaviour that differs from shadcn/Radix is listed with its reason in `ui/README.md`; an unlisted difference is a defect.
- Behaviour changes get a gallery check (`-- --check`); the gallery opens a window, so ask first (`testing R5`).

If a component needs to know about a terminal, a profile or a config key, that knowledge belongs in `src/`, passed in as a value or a callback the app supplies.

Enforced by: `tests/architecture.rs` (`crates_below_the_app_name_only_their_allowed_dependencies`, `brindle_ui_does_no_io_of_its_own`).
