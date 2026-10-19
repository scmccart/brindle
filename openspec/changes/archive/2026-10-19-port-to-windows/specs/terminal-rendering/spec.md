# Spec Delta

## MODIFIED Requirements

### Requirement: GPU rendering
All terminal content and window chrome SHALL be drawn with GPUI's GPU renderer: Vulkan on Linux, Direct3D 11 on Windows 11. On Windows the window SHALL be per-monitor DPI aware, so it stays sharp at every display scale and when moved between monitors with different scales.

#### Scenario: Window opens with GPU rendering
- **WHEN** Brindle starts on a Linux desktop with Vulkan available
- **THEN** a window appears and the terminal grid is drawn through GPUI

#### Scenario: Window opens on Windows
- **WHEN** Brindle starts on Windows 11
- **THEN** a window appears and the terminal grid is drawn through GPUI's Direct3D 11 renderer

#### Scenario: High-DPI display
- **WHEN** Brindle runs on a Windows display scaled to 150%
- **THEN** text and chrome are rendered at the display's native resolution, not stretched from 100%
