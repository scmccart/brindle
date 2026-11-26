## ADDED Requirements

### Requirement: Images in the dump
In an agent zone's dump, an image attachment's `--- attached` and `--- draft` lines SHALL give its kind as `image`, its type (`png`, `jpeg`, `gif` or `webp`), its name and its size, and never its bytes; an `--- attached` line SHALL also give its row's state (`closed`, `loading`, `shown`, `gone` or `undrawable`). While a prompt's attachments are being sent, the `--- prompt` line SHALL read `status=sending` with the bytes sent and the total. `--wait-for` SHALL match these lines.

#### Scenario: A draft image
- **WHEN** a 200 KiB PNG named `shot.png` waits in the prompt input, and the dump is taken
- **THEN** the dump has `--- draft index=0 kind=image type=png name="shot.png" bytes=204800`

#### Scenario: A sent image
- **WHEN** the user sent a prompt with that image, and the dump is taken
- **THEN** the dump has a line `--- attached entry=N index=0 kind=image id=1 type=png name="shot.png" bytes=204800 state=closed`

### Requirement: Attaching a file from the command line
`--attach-file PATH` SHALL be a debug step that attaches the file to the active agent zone's prompt as if it had been dropped there, so a case can attach an image without a pointer. A path that can't be read SHALL be a usage error at launch, naming the option, and the step SHALL do nothing in a terminal zone but say so in the dump.

#### Scenario: An image attached by a step
- **WHEN** Brindle runs `--attach-file shot.png --dump-screen-after 2` in an agent zone, `shot.png` being a PNG
- **THEN** the dump has a `--- draft` line of kind `image` and type `png`

#### Scenario: A file that isn't there
- **WHEN** Brindle is launched with `--attach-file /nonexistent`
- **THEN** it exits with status 2 before any window opens, naming `--attach-file`
