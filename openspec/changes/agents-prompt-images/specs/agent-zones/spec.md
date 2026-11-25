## MODIFIED Requirements

### Requirement: Attachment limits are said when attaching
Adding a ninth attachment SHALL be refused: the window's notice line SHALL say a prompt takes at most 8, and the attachments already there SHALL stay. A text or an image longer than 5 MiB, and an image that would bring a prompt's images over 8 MiB, SHALL be refused outright, never shortened or shrunk: the notice line SHALL say how long it is and what the limit is, and nothing SHALL be attached.

#### Scenario: A ninth attachment
- **WHEN** the prompt input holds 8 attachments and the user attaches a file
- **THEN** the notice line says a prompt takes at most 8 attachments, and 8 remain

#### Scenario: Output that is too long
- **WHEN** the user runs Send Last Command Output to Agent for a command that printed 6 MiB
- **THEN** nothing is attached, the notice line says the output is 6 MiB and an attachment takes at most 5 MiB, and the terminal keeps focus

#### Scenario: A dropped file that is too long
- **WHEN** the user drops a 7 MiB text file on the zone of an agent on an SSH host
- **THEN** it isn't attached, and the notice line names the file, its size and the limit

#### Scenario: Output that used to be too long
- **WHEN** the user runs Send Last Command Output to Agent for a command that printed 1 MiB
- **THEN** the output is attached whole

#### Scenario: Too many image bytes
- **WHEN** the prompt input holds two 3 MiB images and the user pastes a third
- **THEN** it isn't attached, and the notice line says a prompt takes at most 8 MiB of images

### Requirement: Dropping files on an agent zone
Files dropped on an agent zone SHALL be attached. A file that is a PNG, JPEG, GIF or WebP image, as its first bytes say, SHALL be attached as an image, wherever the agent runs. Otherwise, when the zone's agent runs on this machine, each SHALL be a link to its path; when it runs on another host or in a container, a file that is text SHALL be attached as text under its file name, and any other SHALL be refused. A directory, and a file that can't be read, SHALL be refused. Each refusal SHALL name the file and the reason on the window's notice line, and SHALL NOT stop the other files being attached.

#### Scenario: A local agent
- **WHEN** the user drops `/home/u/a.rs` on the zone of an agent that runs on this machine
- **THEN** the prompt input shows a link attachment to `/home/u/a.rs`

#### Scenario: A remote agent
- **WHEN** the user drops a 2 KiB text file `notes.md` on the zone of an agent on an SSH host
- **THEN** the prompt input shows a text attachment `notes.md` holding the file's text

#### Scenario: An image, for a remote agent
- **WHEN** the user drops `logo.png` and `notes.md` on that zone
- **THEN** `logo.png` is attached as an image and `notes.md` as text

#### Scenario: An image, for a local agent
- **WHEN** the user drops `logo.png` on the zone of an agent that runs on this machine
- **THEN** the prompt input shows an image attachment `logo.png`, not a link

#### Scenario: Neither text nor an image, for a remote agent
- **WHEN** the user drops `archive.zip` on the zone of an agent on an SSH host
- **THEN** it isn't attached, and the notice line says `archive.zip` is neither text nor an image that can be attached

#### Scenario: A name that lies
- **WHEN** the user drops a text file named `notes.png` on the zone of an agent on an SSH host
- **THEN** it is attached as text

## ADDED Requirements

### Requirement: Pasting an image
Pasting into an agent zone's prompt input with an image on the clipboard and no text SHALL attach the image. With text on the clipboard, paste SHALL insert the text, as before, whether or not an image is there too. The `agent_paste_image` action SHALL attach the clipboard's image whatever else it holds, and the command palette SHALL list it as Paste Image while an agent zone is active. An image of a type that can't be attached, or no image at all, SHALL be said on the window's notice line.

#### Scenario: A screenshot on the clipboard
- **WHEN** the clipboard holds a PNG screenshot and no text, and the user pastes into the prompt input
- **THEN** the prompt input shows an image attachment, and its text is unchanged

#### Scenario: Text and an image
- **WHEN** the clipboard holds both text and an image, and the user pastes
- **THEN** the text is inserted and nothing is attached, and Paste Image then attaches the image

#### Scenario: A type that can't be attached
- **WHEN** the clipboard holds a TIFF image and the user runs Paste Image
- **THEN** nothing is attached, and the notice line says PNG, JPEG, GIF and WebP images can be attached

#### Scenario: No image
- **WHEN** the clipboard holds only text and the user runs Paste Image
- **THEN** nothing is attached, and the notice line says the clipboard holds no image

### Requirement: An attached image is previewed
An image waiting in the prompt input SHALL show a small preview of itself in its item, with its name and size. An image whose bytes can't be drawn SHALL show a plain image icon instead, and SHALL still be sent.

#### Scenario: Preview
- **WHEN** the user pastes a screenshot
- **THEN** its item in the prompt input shows a small picture of the screenshot, and its size

### Requirement: Opening an image in the conversation
In the conversation an image attachment SHALL be a row with its name, type and size, closed. A pointer press SHALL fetch the image from the server, showing how much has arrived, then show it under the row, no wider than the message and scaled whole. A second press SHALL close it. When the server no longer keeps the image, or its bytes can't be drawn, the row SHALL say so. An agent zone SHALL hold only the images of open rows, and SHALL draw only images the user attached.

#### Scenario: Opened
- **WHEN** the user presses the row of an image they sent
- **THEN** the row shows the image arriving, then the image under it

#### Scenario: After a relaunch
- **WHEN** Brindle is quit and launched again, and the user opens the same row in the restored zone
- **THEN** the same image shows

#### Scenario: An agent's image is not drawn
- **WHEN** an agent's message holds a Markdown image
- **THEN** it is named, as before, and never fetched or drawn

### Requirement: Opening a long text attachment
Opening a text attachment longer than 256 KiB SHALL fetch and show its first 256 KiB, with a line saying how long the whole is.

#### Scenario: A 3 MiB log
- **WHEN** the user opens the row of a 3 MiB text attachment
- **THEN** its first 256 KiB show, with a line saying the attachment is 3 MiB

### Requirement: Sending attachments shows progress and can be stopped
While a prompt's attachments are being sent to the server, the prompt input SHALL show that it is sending and how much has been sent, and SHALL accept no second send. Escape, or the prompt input's stop button, SHALL stop the send: nothing reaches the agent, the server keeps nothing of it, and the text and the attachments stay in the prompt input.

#### Scenario: Progress
- **WHEN** the user sends a prompt with a 5 MiB image over a slow connection
- **THEN** the prompt input shows it is sending, with the amount sent growing, and then the user message appears

#### Scenario: Stopped
- **WHEN** the user presses escape while the image is half sent
- **THEN** the send stops, the agent receives nothing, and the text and the image are still in the prompt input
