# Changelog

## [0.4.0](https://github.com/scmccart/brindle/compare/v0.3.0...v0.4.0) (2026-11-28)


### ⚠ BREAKING CHANGES

* **cli:** split launch options into the terminal subcommand ([#121](https://github.com/scmccart/brindle/issues/121))
* show link failures on the config line and give an elevated Brindle its own instance ([#105](https://github.com/scmccart/brindle/issues/105))
* close the 2026-11-20 security audit's high and medium findings ([#104](https://github.com/scmccart/brindle/issues/104))
* **remote:** remote tabs run in the host's terminal server and survive a quit ([#56](https://github.com/scmccart/brindle/issues/56))
* run Windows tabs in the terminal server ([#51](https://github.com/scmccart/brindle/issues/51))
* **tmux:** drop tmux support ([#41](https://github.com/scmccart/brindle/issues/41))
* **palette:** rebuild the palettes on brindle-ui's Command ([#38](https://github.com/scmccart/brindle/issues/38))
* **settings:** rebuild the settings dialog on brindle-ui ([#37](https://github.com/scmccart/brindle/issues/37))
* **terminal-server:** keep terminals across restarts, one window, single instance ([#28](https://github.com/scmccart/brindle/issues/28))

### Features

* **agents:** complete slash commands and set session options from the prompt input ([#151](https://github.com/scmccart/brindle/issues/151)) ([ba18a14](https://github.com/scmccart/brindle/commit/ba18a146df19ca0a365caf0f99009a4be755546f))
* **agents:** continue an ended or lost agent's conversation ([#132](https://github.com/scmccart/brindle/issues/132)) ([a709f18](https://github.com/scmccart/brindle/commit/a709f18a178d1c33284f8d12188fded15905ad14))
* **agents:** draw an agent zone wholly in its profile's theme ([#139](https://github.com/scmccart/brindle/issues/139)) ([d914cd4](https://github.com/scmccart/brindle/commit/d914cd4542e88f171b72bef94080e20ad1a47361))
* **agents:** list an agent's earlier conversations and continue one ([#150](https://github.com/scmccart/brindle/issues/150)) ([5f335dc](https://github.com/scmccart/brindle/commit/5f335dcacfb987b79e2dc9f06ee2cd9d4b4d23ad))
* **agents:** log in to an agent from its zone ([#133](https://github.com/scmccart/brindle/issues/133)) ([725ed28](https://github.com/scmccart/brindle/commit/725ed2805e266514703069bd33541e39708757a7))
* **agents:** open ACP agents in zones from agent profiles ([#123](https://github.com/scmccart/brindle/issues/123)) ([5535e90](https://github.com/scmccart/brindle/commit/5535e90f83da92c3bc2ae73d2d11a57d213c47b2))
* **agents:** page an agent conversation's log on attach and fetch earlier entries ([#144](https://github.com/scmccart/brindle/issues/144)) ([a6c6d23](https://github.com/scmccart/brindle/commit/a6c6d23d290f2a365994d850a3533bf9b543fc2c))
* **agents:** render markdown in agent zones ([#127](https://github.com/scmccart/brindle/issues/127)) ([b68312a](https://github.com/scmccart/brindle/commit/b68312a842b996f555f1a77a97233c155ec434d0))
* **agents:** show a tool call's diffs, output and text in its row ([#146](https://github.com/scmccart/brindle/issues/146)) ([e7cf6fe](https://github.com/scmccart/brindle/commit/e7cf6fe91902359029e168d0b9393d65db2597eb))
* **agents:** show a working agent and its permissions as a risk ([#142](https://github.com/scmccart/brindle/issues/142)) ([6c8ecd2](https://github.com/scmccart/brindle/commit/6c8ecd2d952583fffff00575a8779960d766eaf2))
* **agents:** title an agent tab by its profile until the agent names the conversation ([#141](https://github.com/scmccart/brindle/issues/141)) ([068624f](https://github.com/scmccart/brindle/commit/068624fc4a179d709f844a6a82f7dc0fcd78f93d))
* **cli:** split launch options into the terminal subcommand ([#121](https://github.com/scmccart/brindle/issues/121)) ([f6f59dc](https://github.com/scmccart/brindle/commit/f6f59dc655bdfcc6db4317e7a6c0689f18b4ddf1))
* **config:** show clamped, removed and unusable config values on the window's config line ([#85](https://github.com/scmccart/brindle/issues/85)) ([eacb88a](https://github.com/scmccart/brindle/commit/eacb88aabed3cd0675316d9dd92d5eaf413164aa))
* **contexts:** show OSC 3008 context badges on tabs and zone headers ([#115](https://github.com/scmccart/brindle/issues/115)) ([bde22a2](https://github.com/scmccart/brindle/commit/bde22a2e28d82e8e9708c8942d5c868c754f14a0))
* detect windows elevation, add one-command installers and polish the clipboard ([#119](https://github.com/scmccart/brindle/issues/119)) ([82995d2](https://github.com/scmccart/brindle/commit/82995d27ed9bcd9a00524981e75d2cd238338cfb))
* **notifications:** support kitty's OSC 99 notifications with updates, queries and buttons ([#117](https://github.com/scmccart/brindle/issues/117)) ([a62a6c1](https://github.com/scmccart/brindle/commit/a62a6c177ea00e23e3a343aca830c27cbb6957ec))
* **palette:** rebuild the palettes on brindle-ui's Command ([#38](https://github.com/scmccart/brindle/issues/38)) ([cfe571a](https://github.com/scmccart/brindle/commit/cfe571a781b56d7dedc6e962fb3c2e9a692a640e))
* paste the clipboard on right-click when right_click = "paste" ([#100](https://github.com/scmccart/brindle/issues/100)) ([446b837](https://github.com/scmccart/brindle/commit/446b837d48b28749f81bab52616aa02ea3c03c63))
* **pointer:** show OSC 22 pointer shapes over the grid, with kitty's stack and queries ([#116](https://github.com/scmccart/brindle/issues/116)) ([4cbb1c8](https://github.com/scmccart/brindle/commit/4cbb1c816550500da64caa65e02036f1fba7bc5e))
* print the built-in themes' UI tokens as CSS with --export-ui-tokens ([#103](https://github.com/scmccart/brindle/issues/103)) ([90aac19](https://github.com/scmccart/brindle/commit/90aac195e19cd0faa92bf6f025d62efb4244dcca))
* **progress:** show OSC 9;4 progress as a bar on tabs and zone headers ([#113](https://github.com/scmccart/brindle/issues/113)) ([5bd1d21](https://github.com/scmccart/brindle/commit/5bd1d21da0affc2947f45050fd813d3b99be89c2))
* **remote:** remote tabs run in the host's terminal server and survive a quit ([#56](https://github.com/scmccart/brindle/issues/56)) ([173a20b](https://github.com/scmccart/brindle/commit/173a20b98c629482611bea2e8ff02a60645ee3c1))
* **remote:** run ACP agent sessions in the terminal server ([#122](https://github.com/scmccart/brindle/issues/122)) ([79381c2](https://github.com/scmccart/brindle/commit/79381c2041da054302c95b803375b41d670a91e6))
* run Windows tabs in the terminal server ([#51](https://github.com/scmccart/brindle/issues/51)) ([3976963](https://github.com/scmccart/brindle/commit/3976963f3add352e0491600f95e541b4d9d7596b))
* **session-restore:** rebuild split tabs from layouts kept in the terminal server ([#61](https://github.com/scmccart/brindle/issues/61)) ([d667db0](https://github.com/scmccart/brindle/commit/d667db09650ad0d052c31c087bacb4f4944f275e))
* **settings:** add [ui_font] and a Fonts section to the settings dialog ([#65](https://github.com/scmccart/brindle/issues/65)) ([963849e](https://github.com/scmccart/brindle/commit/963849e15810a90e8f07c5a1d51bb87bb23c9ddb))
* **settings:** add a Clipboard section for copy on select and right-click paste ([#102](https://github.com/scmccart/brindle/issues/102)) ([8a36762](https://github.com/scmccart/brindle/commit/8a36762df9e5add55245860512dc1ac217ef8588))
* **settings:** open settings from the profile menu, mark built-in themes, close on X ([#99](https://github.com/scmccart/brindle/issues/99)) ([fe89829](https://github.com/scmccart/brindle/commit/fe898299c7e9854135dbedfd9cf8ff512fd04a71))
* **settings:** rebuild the settings dialog on brindle-ui ([#37](https://github.com/scmccart/brindle/issues/37)) ([bad3fe8](https://github.com/scmccart/brindle/commit/bad3fe858912e9058b01f63e4cb3bfd2a6be7e08))
* **shell-integration:** read OSC 633 and copy the last command line ([#112](https://github.com/scmccart/brindle/issues/112)) ([0d2dac8](https://github.com/scmccart/brindle/commit/0d2dac807f33638649b0e6b9b585b3b5ff6c1c76))
* show link failures on the config line and give an elevated Brindle its own instance ([#105](https://github.com/scmccart/brindle/issues/105)) ([a2f7809](https://github.com/scmccart/brindle/commit/a2f78091d98487e1d4f671d056cec96c5a80623f))
* **terminal-server:** keep terminals across restarts, one window, single instance ([#28](https://github.com/scmccart/brindle/issues/28)) ([8d703de](https://github.com/scmccart/brindle/commit/8d703deb0d177471c0d8fd7ea52d592f8871af70))
* **terminal-server:** run local Linux terminals in a persistent server ([#22](https://github.com/scmccart/brindle/issues/22)) ([1b4bd93](https://github.com/scmccart/brindle/commit/1b4bd9340d67d93770dfe6440945568a58bade16))
* **terminal:** say in the tab what failed: lost links, opener errors, missing profiles ([#88](https://github.com/scmccart/brindle/issues/88)) ([056e686](https://github.com/scmccart/brindle/commit/056e686a0ba9666abd6ec3d86f0e68d5680e8479))
* **tmux:** drop tmux support ([#41](https://github.com/scmccart/brindle/issues/41)) ([0d02107](https://github.com/scmccart/brindle/commit/0d02107bfc49a72704d36379695205a695917b4f))
* **ui:** add the brindle-ui crate with shadcn-style components ([#31](https://github.com/scmccart/brindle/issues/31)) ([d9e70dc](https://github.com/scmccart/brindle/commit/d9e70dcd13e127e1b509e2e7a9fb95d5d9a078e2))
* **ui:** derive brindle-ui tokens from the terminal theme ([#35](https://github.com/scmccart/brindle/issues/35)) ([64b1573](https://github.com/scmccart/brindle/commit/64b1573ff821d9a60d69bec181a523d9816c5f43))
* **ui:** draw zones and tmux windows as cards, with a spaced tab strip ([#40](https://github.com/scmccart/brindle/issues/40)) ([26fd093](https://github.com/scmccart/brindle/commit/26fd093d7572537a61b7b886347398c72d69ec09))
* **ui:** rebuild the window chrome on brindle-ui ([#39](https://github.com/scmccart/brindle/issues/39)) ([935c1a5](https://github.com/scmccart/brindle/commit/935c1a5a53aec982f1485170d67ac1cd4176629c))
* **zones:** let one tab hold terminal and agent zones of any profiles and hosts ([#149](https://github.com/scmccart/brindle/issues/149)) ([42c35fe](https://github.com/scmccart/brindle/commit/42c35fe262e3345dcb67cb485abacc9ab0a3e913))


### Bug Fixes

* **agents:** close what an agent process leaves open, and replay its restarts and exits ([#156](https://github.com/scmccart/brindle/issues/156)) ([c955783](https://github.com/scmccart/brindle/commit/c9557836806ac3fab4602a6e3ca80d8fa8c6e2c9))
* **agents:** keep every agent event within one frame and bound tool entries in the fold ([#155](https://github.com/scmccart/brindle/issues/155)) ([8081050](https://github.com/scmccart/brindle/commit/8081050013b159dccb895208f170e623739018c9))
* **agents:** keep focus in an agent zone's prompt and its reading place while it scrolls ([#154](https://github.com/scmccart/brindle/issues/154)) ([677dffd](https://github.com/scmccart/brindle/commit/677dffd41434c40c0288ec7b20929ff30eec1aec))
* **agents:** lay out markdown as one bubble and wrap text at its container's width ([#136](https://github.com/scmccart/brindle/issues/136)) ([163acb2](https://github.com/scmccart/brindle/commit/163acb2619e8ebef30cd36000935011bb7fb7489))
* **agents:** send nothing until an agent session runs, and tell a lost answer from a refusal ([#153](https://github.com/scmccart/brindle/issues/153)) ([992607b](https://github.com/scmccart/brindle/commit/992607b501b09d1454ef71017ac2a56b71eb02c4))
* **build:** silence the unused closing warning on windows ([#30](https://github.com/scmccart/brindle/issues/30)) ([6da6b56](https://github.com/scmccart/brindle/commit/6da6b5635640f5b462e5251694f367576ed83462))
* **cli:** reject bad debug steps at launch and report a stopped latency log, no state dir ([#89](https://github.com/scmccart/brindle/issues/89)) ([786d1ec](https://github.com/scmccart/brindle/commit/786d1ec0e57078fe1eef4e3c9d20347b123e9935))
* close the 2026-11-20 security audit's high and medium findings ([#104](https://github.com/scmccart/brindle/issues/104)) ([3bec95f](https://github.com/scmccart/brindle/commit/3bec95f7d2f12b963b9263338285c71d119d8e2b))
* **e2e:** draw the window before an --action or --key step dispatches ([#143](https://github.com/scmccart/brindle/issues/143)) ([19368a9](https://github.com/scmccart/brindle/commit/19368a9dbd23256e11efa6208e84de81071bd6b0))
* make the test suites reliable and fix the races they exposed ([#47](https://github.com/scmccart/brindle/issues/47)) ([e46e790](https://github.com/scmccart/brindle/commit/e46e790e173ff5ebe2e3904a515fe97bcb05e702))
* **remote-agent:** send SIGHUP before closing the PTY, and keep tests out of the env ([#60](https://github.com/scmccart/brindle/issues/60)) ([0252d46](https://github.com/scmccart/brindle/commit/0252d46b2741d6d299362664ad971ef729d975a0))
* **remote:** bound queues and contain panics at the process edges ([#77](https://github.com/scmccart/brindle/issues/77)) ([c5ce485](https://github.com/scmccart/brindle/commit/c5ce4852e71ab4ee65f0b38d7454fc8b05140660))
* **remote:** keep a background restored tab at its session's size until it is laid out ([#118](https://github.com/scmccart/brindle/issues/118)) ([5d2d2bb](https://github.com/scmccart/brindle/commit/5d2d2bbb187564b779e2b42485b2c9c2c41ab64d))
* **remote:** match shown sessions by id and key, open the config editor locally ([#71](https://github.com/scmccart/brindle/issues/71)) ([1b2cecd](https://github.com/scmccart/brindle/commit/1b2cecd9358a8e9c73b6bed3ce7b930582815504))
* **remote:** name failure causes, bound relay retries and keep the sessions palette steady ([#58](https://github.com/scmccart/brindle/issues/58)) ([092fc07](https://github.com/scmccart/brindle/commit/092fc07ac5f7129ab4bb36dfb8ce47d8a54770c2))
* **remote:** name the uploaded helper and what the host's copy printed ([#63](https://github.com/scmccart/brindle/issues/63)) ([9177c4c](https://github.com/scmccart/brindle/commit/9177c4ced12759d65fc6fa9c603e4c1b4a40cfc1))
* **remote:** show unlisted machines and failed kills, keep causes of exits, archive ([#90](https://github.com/scmccart/brindle/issues/90)) ([1d559ce](https://github.com/scmccart/brindle/commit/1d559ced54e3a6027ebba3985dedfed27155a847))
* **remote:** start remote programs in the login shell's environment ([#137](https://github.com/scmccart/brindle/issues/137)) ([cdb972c](https://github.com/scmccart/brindle/commit/cdb972c45570b0b9baa708878a2cb61bfe4a0eca))
* **session:** keep the state file when the state can't be serialized ([#70](https://github.com/scmccart/brindle/issues/70)) ([beea716](https://github.com/scmccart/brindle/commit/beea7161e37cea812dd4c263e0ccb2ce129127a6))
* **session:** list sessions, read config and write logs off the UI thread ([#76](https://github.com/scmccart/brindle/issues/76)) ([bb44a10](https://github.com/scmccart/brindle/commit/bb44a10da8ae69d51f03e17696a0164bbed92616))
* **tabs:** open a background tab's menu without switching to it ([#66](https://github.com/scmccart/brindle/issues/66)) ([1127a54](https://github.com/scmccart/brindle/commit/1127a540ea28cd406a9dd7b31b514bf1039158d5))
* **terminal-server:** surface the failures the server and helper swallowed ([#86](https://github.com/scmccart/brindle/issues/86)) ([731e4c9](https://github.com/scmccart/brindle/commit/731e4c994e94a9b50314465c3f6e18c4b6b3059b))
* **terminal:** keep the window responsive while a terminal floods output ([#26](https://github.com/scmccart/brindle/issues/26)) ([d492012](https://github.com/scmccart/brindle/commit/d4920126f107e281048db8ebcec41c6299a0b5ae))
* **ui:** build icon paths purely and log a failed one once instead of dropping it ([#84](https://github.com/scmccart/brindle/issues/84)) ([2e4b316](https://github.com/scmccart/brindle/commit/2e4b3161c98d45883ab7c80348ce9e3771c2266a))


### Performance

* **agents:** append checkpoint deltas, encode each event once and stop recopying output ([#158](https://github.com/scmccart/brindle/issues/158)) ([4640c48](https://github.com/scmccart/brindle/commit/4640c482921ffa3fa26301b13844594b34283e57))
* **agents:** copy only what changed under the shared lock and draw each entry once a frame ([#159](https://github.com/scmccart/brindle/issues/159)) ([ecd51ab](https://github.com/scmccart/brindle/commit/ecd51ab964b185ebdd6d56417e7d6f5f952b353c))

## [0.3.0](https://github.com/scmccart/brindle/compare/v0.2.0...v0.3.0) (2026-11-04)


### Features

* **remote:** add SSH and container profiles ([#13](https://github.com/scmccart/brindle/issues/13)) ([4bfd358](https://github.com/scmccart/brindle/commit/4bfd358c5a0ceb5e115fe4c832d793857f1510dd))
* **remote:** edit remote targets in settings and detach on a dropped connection ([#14](https://github.com/scmccart/brindle/issues/14)) ([f663ba4](https://github.com/scmccart/brindle/commit/f663ba43154bfc84bf29e9e2488c01cead4ef618))


### Bug Fixes

* **ci:** trigger push workflows only on main to avoid duplicate PR runs ([#11](https://github.com/scmccart/brindle/issues/11)) ([d9c1801](https://github.com/scmccart/brindle/commit/d9c180102e233bf390583ea6c0e9f9e457dc8777))
* **terminal:** encode modified Enter as CSI u sequences ([#10](https://github.com/scmccart/brindle/issues/10)) ([09246ac](https://github.com/scmccart/brindle/commit/09246ac31da479369ab50b5c8d197a8dcf07538e))
