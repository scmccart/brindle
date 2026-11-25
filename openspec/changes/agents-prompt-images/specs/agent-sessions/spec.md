## MODIFIED Requirements

### Requirement: Prompts carry attachments
A client SHALL be able to send, with a prompt's text, a list of attachments of three kinds: a link to a file, by its path and name, text with a name, and an image with a name. The server SHALL send them to the agent after the prompt's text, in the client's order, and SHALL record them in the user message's log entry, where every attached client and every replay SHALL have them.

#### Scenario: A prompt with a link and a text
- **WHEN** a client sends the prompt `why does this fail?` with a text attachment named `cargo test` and a link to `src/main.rs`
- **THEN** the agent receives the prompt's text, then the attachment `cargo test`, then the link, and the log's user entry lists both attachments in that order

#### Scenario: Replayed
- **WHEN** that session is detached and a client attaches it
- **THEN** the replay holds the user message with its two attachments

#### Scenario: A prompt without attachments
- **WHEN** a client sends a prompt with no attachments
- **THEN** the agent receives the text alone, as before

#### Scenario: A prompt with an image
- **WHEN** a client sends the prompt `what is wrong here?` with a PNG image named `screenshot.png`
- **THEN** the agent receives the prompt's text, then the image, and the log's user entry lists the image by its name, type and size, with none of its bytes

### Requirement: A client can read an attachment's text
A client attached to an agent session SHALL be able to read a kept attachment, text or image, by its id, a piece at a time from a place in it that the client names, each answer saying how long the whole is. Read from its start to its end, the pieces SHALL be exactly the bytes that were sent. A request for an id the session doesn't hold, or whose file is no longer kept, SHALL be refused with an error saying so.

#### Scenario: Read back
- **WHEN** a client sent a text attachment holding `error: mismatched types`, and later asks for that attachment's text
- **THEN** it receives `error: mismatched types`

#### Scenario: After a reattach
- **WHEN** a client detaches, another attaches the session, and asks for the attachment listed in the replay
- **THEN** it receives the same text

#### Scenario: An unknown id
- **WHEN** a client asks for attachment 99 of a session that has kept three
- **THEN** it gets an error saying there is no such attachment

#### Scenario: An image in pieces
- **WHEN** a client reads a kept 3 MiB image from its start, asking for the next piece each time one arrives
- **THEN** the pieces together are the 3 MiB that were sent, and each answer said the whole is 3 MiB

### Requirement: Prompt attachments are bounded
The server SHALL refuse, with an error saying why, a prompt with more than 8 attachments or more than 8 MiB of images, an attachment over 5 MiB, and an attachment that would bring what the session keeps over 64 MiB; nothing of a refused prompt SHALL be added to the log, queued or kept. An attachment's name and a link's path SHALL have their control characters replaced as prompt text has; a kept text SHALL be kept exactly as it was sent.

#### Scenario: Too many
- **WHEN** a client sends a prompt with 9 attachments
- **THEN** it gets an error naming the limit of 8, and the agent receives nothing

#### Scenario: A text that is too long
- **WHEN** a client starts sending a text attachment of 6 MiB
- **THEN** it gets an error naming the 5 MiB limit before any of the text is sent

#### Scenario: The store is full
- **WHEN** a session keeps 64 MiB of attachments and a client attaches more
- **THEN** it gets an error saying how much the conversation's attachments hold

#### Scenario: A refused prompt keeps nothing
- **WHEN** a client attaches a text and its prompt is then refused because the agent isn't ready
- **THEN** no file for that text remains

#### Scenario: Too many image bytes
- **WHEN** a client sends a prompt naming three images of 3 MiB each
- **THEN** it gets an error naming the 8 MiB limit for a prompt's images, and the agent receives nothing

#### Scenario: Kept as sent
- **WHEN** a client attaches a text holding a tab, a carriage return and an escape character, and reads it back
- **THEN** it receives those same bytes

## ADDED Requirements

### Requirement: Attachments are sent to the server in pieces
A client SHALL send a text or image attachment as a start that names its kind and whole length, pieces of its bytes, and an end. The server SHALL refuse at the start what a bound rules out, SHALL accept one upload at a time on a connection, and SHALL keep the attachment only when exactly the stated length arrived. An upload that ends short, passes its stated length, or belongs to a connection that closes SHALL leave no file.

#### Scenario: In three pieces
- **WHEN** a client sends a 2.5 MiB attachment as a start, three pieces and an end
- **THEN** the server keeps a file of exactly those 2.5 MiB, and a prompt can name it

#### Scenario: Short
- **WHEN** a client states 2 MiB, sends 1 MiB and then the end
- **THEN** it gets an error saying the upload is incomplete, and no file remains

#### Scenario: Too many bytes
- **WHEN** a client states 1 MiB and sends pieces that pass it
- **THEN** it gets an error, and no file remains

#### Scenario: The connection closes
- **WHEN** a client's connection closes after two of five pieces
- **THEN** no file for that upload remains

#### Scenario: A second upload
- **WHEN** a client starts an upload while another on its connection hasn't ended
- **THEN** the second is refused with an error saying an upload is in progress

### Requirement: An image's type comes from its bytes
The server SHALL accept PNG, JPEG, GIF and WebP images, and SHALL decide an image's type from the first bytes of what was sent, never from its name. An upload stated as an image whose bytes are another type, or no image type the server knows, SHALL be refused with an error and leave no file. A text attachment whose bytes aren't UTF-8 SHALL be refused the same way.

#### Scenario: A PNG
- **WHEN** a client sends an upload stated as PNG whose bytes begin with the PNG signature
- **THEN** it is kept as a PNG image

#### Scenario: A wrong statement
- **WHEN** a client sends an upload stated as PNG whose bytes are a JPEG
- **THEN** it gets an error saying the bytes aren't a PNG, and no file remains

#### Scenario: Not an image
- **WHEN** a client sends an upload stated as PNG, named `photo.png`, whose bytes are a shell script
- **THEN** it is refused, and no file remains

#### Scenario: Not text
- **WHEN** a client sends an upload stated as text whose bytes aren't UTF-8
- **THEN** it gets an error saying it isn't text, and no file remains

### Requirement: An image reaches every agent
The server SHALL send an image attachment to an agent that says it accepts images as the image itself, with its type, its bytes unchanged. To an agent that doesn't, it SHALL send a link to the file that keeps the image. Image bytes SHALL NOT be put in the log, the queue slot or a checkpoint.

#### Scenario: An agent that accepts images
- **WHEN** the agent accepts images and a client sends a prompt with a 1 MiB PNG
- **THEN** the agent receives image content of type PNG whose bytes are the 1 MiB that were sent

#### Scenario: An agent that doesn't
- **WHEN** the agent doesn't accept images and a client sends a prompt with a PNG
- **THEN** the agent receives a link to a file on its host that holds the PNG

#### Scenario: Queued across a restart
- **WHEN** a prompt with an image waits in the queue, the core restarts, and the turn it waited for ends with `end_turn`
- **THEN** the agent receives the prompt with the image's bytes unchanged

### Requirement: A prompt with its images always reaches the agent whole
The server SHALL deliver a prompt and its images to the agent complete, however slowly the agent reads: what waits to be written to an agent session's input SHALL be allowed to hold the largest prompt the bounds permit.

#### Scenario: A slow reader
- **WHEN** a client sends a prompt with 8 MiB of images to an agent that reads its input slowly
- **THEN** the agent receives the whole prompt, and nothing is dropped
