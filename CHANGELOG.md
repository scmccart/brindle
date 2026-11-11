# Changelog

## [0.4.0](https://github.com/scmccart/brindle/compare/v0.3.0...v0.4.0) (2026-11-11)


### ⚠ BREAKING CHANGES

* **tmux:** drop tmux support ([#41](https://github.com/scmccart/brindle/issues/41))
* **palette:** rebuild the palettes on brindle-ui's Command ([#38](https://github.com/scmccart/brindle/issues/38))
* **settings:** rebuild the settings dialog on brindle-ui ([#37](https://github.com/scmccart/brindle/issues/37))
* **terminal-server:** keep terminals across restarts, one window, single instance ([#28](https://github.com/scmccart/brindle/issues/28))

### Features

* **palette:** rebuild the palettes on brindle-ui's Command ([#38](https://github.com/scmccart/brindle/issues/38)) ([cfe571a](https://github.com/scmccart/brindle/commit/cfe571a781b56d7dedc6e962fb3c2e9a692a640e))
* **settings:** rebuild the settings dialog on brindle-ui ([#37](https://github.com/scmccart/brindle/issues/37)) ([bad3fe8](https://github.com/scmccart/brindle/commit/bad3fe858912e9058b01f63e4cb3bfd2a6be7e08))
* **terminal-server:** keep terminals across restarts, one window, single instance ([#28](https://github.com/scmccart/brindle/issues/28)) ([8d703de](https://github.com/scmccart/brindle/commit/8d703deb0d177471c0d8fd7ea52d592f8871af70))
* **terminal-server:** run local Linux terminals in a persistent server ([#22](https://github.com/scmccart/brindle/issues/22)) ([1b4bd93](https://github.com/scmccart/brindle/commit/1b4bd9340d67d93770dfe6440945568a58bade16))
* **tmux:** drop tmux support ([#41](https://github.com/scmccart/brindle/issues/41)) ([0d02107](https://github.com/scmccart/brindle/commit/0d02107bfc49a72704d36379695205a695917b4f))
* **ui:** add the brindle-ui crate with shadcn-style components ([#31](https://github.com/scmccart/brindle/issues/31)) ([d9e70dc](https://github.com/scmccart/brindle/commit/d9e70dcd13e127e1b509e2e7a9fb95d5d9a078e2))
* **ui:** derive brindle-ui tokens from the terminal theme ([#35](https://github.com/scmccart/brindle/issues/35)) ([64b1573](https://github.com/scmccart/brindle/commit/64b1573ff821d9a60d69bec181a523d9816c5f43))
* **ui:** draw zones and tmux windows as cards, with a spaced tab strip ([#40](https://github.com/scmccart/brindle/issues/40)) ([26fd093](https://github.com/scmccart/brindle/commit/26fd093d7572537a61b7b886347398c72d69ec09))
* **ui:** rebuild the window chrome on brindle-ui ([#39](https://github.com/scmccart/brindle/issues/39)) ([935c1a5](https://github.com/scmccart/brindle/commit/935c1a5a53aec982f1485170d67ac1cd4176629c))


### Bug Fixes

* **build:** silence the unused closing warning on windows ([#30](https://github.com/scmccart/brindle/issues/30)) ([6da6b56](https://github.com/scmccart/brindle/commit/6da6b5635640f5b462e5251694f367576ed83462))
* make the test suites reliable and fix the races they exposed ([#47](https://github.com/scmccart/brindle/issues/47)) ([e46e790](https://github.com/scmccart/brindle/commit/e46e790e173ff5ebe2e3904a515fe97bcb05e702))
* **terminal:** keep the window responsive while a terminal floods output ([#26](https://github.com/scmccart/brindle/issues/26)) ([d492012](https://github.com/scmccart/brindle/commit/d4920126f107e281048db8ebcec41c6299a0b5ae))

## [0.3.0](https://github.com/scmccart/brindle/compare/v0.2.0...v0.3.0) (2026-11-04)


### Features

* **remote:** add SSH and container profiles ([#13](https://github.com/scmccart/brindle/issues/13)) ([4bfd358](https://github.com/scmccart/brindle/commit/4bfd358c5a0ceb5e115fe4c832d793857f1510dd))
* **remote:** edit remote targets in settings and detach on a dropped connection ([#14](https://github.com/scmccart/brindle/issues/14)) ([f663ba4](https://github.com/scmccart/brindle/commit/f663ba43154bfc84bf29e9e2488c01cead4ef618))


### Bug Fixes

* **ci:** trigger push workflows only on main to avoid duplicate PR runs ([#11](https://github.com/scmccart/brindle/issues/11)) ([d9c1801](https://github.com/scmccart/brindle/commit/d9c180102e233bf390583ea6c0e9f9e457dc8777))
* **terminal:** encode modified Enter as CSI u sequences ([#10](https://github.com/scmccart/brindle/issues/10)) ([09246ac](https://github.com/scmccart/brindle/commit/09246ac31da479369ab50b5c8d197a8dcf07538e))
