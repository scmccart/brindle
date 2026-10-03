# Spec Delta

## MODIFIED Requirements

### Requirement: GPU rendering
All terminal content and window chrome SHALL be drawn with GPUI's GPU renderer: Vulkan on Linux, Direct3D 11 on Windows.

#### Scenario: Window opens with GPU rendering
- **WHEN** Brindle starts on a Linux desktop with Vulkan available
- **THEN** a window appears and the terminal grid is drawn through GPUI

#### Scenario: Window opens on Windows
- **WHEN** Brindle starts on Windows 10 or 11
- **THEN** a window appears and the terminal grid is drawn through GPUI's Direct3D 11 renderer
